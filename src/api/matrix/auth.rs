//! Matrix authentication module

use once_cell::sync::Lazy;
use poem::{
    error::InternalServerError,
    handler,
    http::StatusCode,
    web::{Data, Json},
    Request, Result,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::api::DateTime;
use crate::api_key::create_api_key;
use crate::password::verify_password;
use crate::state::{BotKey, State};

/// Matrix login request
/// Supports both simple format and identifier format
/// https://spec.matrix.org/v1.11/client-server-api/#post_matrixclientv3login
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// Login type, should be "m.login.password"
    #[serde(rename = "type")]
    pub login_type: String,
    /// User identifier (new format)
    pub identifier: Option<LoginIdentifier>,
    /// Username (old format)
    pub user: Option<String>,
    /// Password
    pub password: String,
    /// Device ID - if not provided, a new bot_key will be created
    #[serde(default)]
    pub device_id: Option<String>,
}

/// Matrix login identifier
#[derive(Debug, Deserialize)]
pub struct LoginIdentifier {
    /// Identifier type, should be "m.id.user"
    #[serde(rename = "type")]
    pub id_type: String,
    /// Username
    pub user: String,
}

/// Matrix login response
/// https://spec.matrix.org/v1.11/client-server-api/#post_matrixclientv3login
#[derive(Debug, serde::Serialize)]
pub struct LoginResponse {
    /// The access token
    pub access_token: String,
    /// The user's Matrix ID
    pub user_id: String,
    /// Device ID (we use a fixed one for bots)
    pub device_id: String,
}

/// Rate limiter for failed authentication attempts
static AUTH_FAILURE_CACHE: Lazy<RwLock<AuthFailureCache>> =
    Lazy::new(|| RwLock::new(AuthFailureCache::new()));

/// Maximum failed attempts before rate limiting
const MAX_AUTH_FAILURES: u32 = 5;

/// Time window for counting failures (in seconds)
const AUTH_FAILURE_WINDOW_SECS: u64 = 60;

/// How long to block after too many failures (in seconds)
const AUTH_BLOCK_DURATION_SECS: u64 = 120; // 2 minutes

struct AuthFailureCache {
    failures: HashMap<String, AuthFailureEntry>,
}

struct AuthFailureEntry {
    count: u32,
    first_failure: Instant,
    blocked_until: Option<Instant>,
}

impl AuthFailureCache {
    fn new() -> Self {
        Self {
            failures: HashMap::new(),
        }
    }

    /// Check if the client is blocked due to too many failures
    fn is_blocked(&self, key: &str) -> bool {
        if let Some(entry) = self.failures.get(key) {
            if let Some(blocked_until) = entry.blocked_until {
                if Instant::now() < blocked_until {
                    return true;
                }
            }
        }
        false
    }

    /// Get remaining block time in seconds
    fn remaining_block_time(&self, key: &str) -> u64 {
        if let Some(entry) = self.failures.get(key) {
            if let Some(blocked_until) = entry.blocked_until {
                let now = Instant::now();
                if now < blocked_until {
                    return blocked_until.duration_since(now).as_secs();
                }
            }
        }
        0
    }

    /// Record a failed authentication attempt
    fn record_failure(&mut self, key: &str) {
        let now = Instant::now();
        let entry = self
            .failures
            .entry(key.to_string())
            .or_insert(AuthFailureEntry {
                count: 0,
                first_failure: now,
                blocked_until: None,
            });

        // Reset counter if outside the window
        if now.duration_since(entry.first_failure).as_secs() > AUTH_FAILURE_WINDOW_SECS {
            entry.count = 0;
            entry.first_failure = now;
            entry.blocked_until = None;
        }

        entry.count += 1;

        // Block if too many failures
        if entry.count >= MAX_AUTH_FAILURES {
            entry.blocked_until =
                Some(now + std::time::Duration::from_secs(AUTH_BLOCK_DURATION_SECS));
            tracing::warn!(
                "Client {} blocked for {} seconds due to {} auth failures",
                key,
                AUTH_BLOCK_DURATION_SECS,
                entry.count
            );
        }
    }

    /// Clear failures on successful auth
    fn clear_failures(&mut self, key: &str) {
        self.failures.remove(key);
    }
}

/// Default Matrix domain if not configured
const DEFAULT_MATRIX_DOMAIN: &str = "localhost";

/// Default device ID when not provided by client
const DEFAULT_DEVICE_ID: &str = "matrix_device";

/// Get Matrix domain from config or use default
pub fn get_matrix_domain(state: &State) -> String {
    state
        .config
        .network
        .matrix_domain
        .clone()
        .or_else(|| state.config.network.domain.first().cloned())
        .unwrap_or_else(|| DEFAULT_MATRIX_DOMAIN.to_string())
}

/// Matrix error response format
/// See: https://spec.matrix.org/v1.11/client-server-api/#standard-error-response
pub fn matrix_error(status: StatusCode, errcode: &str, error: &str) -> poem::error::Error {
    let body = json!({
        "errcode": errcode,
        "error": error
    });
    poem::error::Error::from_string(body.to_string(), status)
}

/// Rate limit error with Retry-After header
pub fn rate_limit_error(retry_after_secs: u64) -> poem::error::Error {
    let body = json!({
        "errcode": "M_LIMIT_EXCEEDED",
        "error": format!("Too many failed attempts. Retry after {} seconds", retry_after_secs),
        "retry_after_ms": retry_after_secs * 1000
    });
    poem::error::Error::from_string(body.to_string(), StatusCode::TOO_MANY_REQUESTS)
}

/// Decode a URL-encoded path segment to a String
/// Returns an error if the encoding is invalid
pub fn decode_path_segment(segment: &str) -> Result<String, poem::error::Error> {
    use percent_encoding::percent_decode;
    percent_decode(segment.as_bytes())
        .decode_utf8()
        .map_err(|_| {
            poem::error::Error::from_string("Invalid path encoding", StatusCode::BAD_REQUEST)
        })
        .map(|s| s.to_string())
}

/// Parse and validate Matrix user ID format: @user:domain_name
/// Returns the username part after validation
pub fn parse_and_validate_matrix_user_id(
    user_id_input: &str,
    expected_domain: &str,
) -> Result<String, poem::error::Error> {
    let input = if user_id_input.starts_with('@') {
        &user_id_input[1..]
    } else {
        user_id_input
    };

    // Parse user:domain format
    let parts: Vec<&str> = input.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_INVALID_PARAM",
            "Invalid username format. Expected format: @user:domain_name",
        ));
    }

    let user_part = parts[0];
    let domain_part = parts[1];

    // Validate domain matches expected domain
    if domain_part != expected_domain {
        return Err(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_INVALID_PARAM",
            &format!(
                "Domain '{}' does not match server domain '{}'",
                domain_part, expected_domain
            ),
        ));
    }

    // Validate user part is not empty
    if user_part.is_empty() {
        return Err(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_INVALID_PARAM",
            "Username cannot be empty",
        ));
    }

    Ok(user_part.to_string())
}

/// Get client identifier for rate limiting (IP address or token prefix)
fn get_client_identifier(req: &Request, token: Option<&str>) -> String {
    // Try to get IP from X-Forwarded-For or X-Real-IP headers first
    if let Some(forwarded) = req.headers().get("X-Forwarded-For") {
        if let Ok(forwarded_str) = forwarded.to_str() {
            // Take the first IP in the chain
            if let Some(ip) = forwarded_str.split(',').next() {
                return format!("ip:{}", ip.trim());
            }
        }
    }
    if let Some(real_ip) = req.headers().get("X-Real-IP") {
        if let Ok(ip) = real_ip.to_str() {
            return format!("ip:{}", ip);
        }
    }
    // Fall back to token prefix if available
    if let Some(t) = token {
        let prefix = if t.len() > 16 { &t[..16] } else { t };
        return format!("token:{}", prefix);
    }
    // Last resort: use a hash of the request path
    "unknown".to_string()
}

/// Core function to validate Matrix access token and return user id
pub async fn validate_token_core(state: &State, token: &str, req: &Request) -> Result<i64> {
    // Get client identifier for rate limiting
    let client_id = get_client_identifier(req, Some(token));

    // Check if client is blocked due to too many failures
    {
        let cache = AUTH_FAILURE_CACHE.read().await;
        if cache.is_blocked(&client_id) {
            let remaining = cache.remaining_block_time(&client_id);
            tracing::warn!(
                "Client {} is rate limited, {} seconds remaining",
                client_id,
                remaining
            );
            return Err(rate_limit_error(remaining));
        }
    }

    // Find the user by token in bot_keys
    let cache = state.cache.read().await;
    let found_uid = cache
        .users
        .iter()
        .find(|(_, user)| user.bot_keys.values().any(|bot_key| bot_key.key == token))
        .map(|(uid, _)| *uid);

    let uid = match found_uid {
        Some(uid) => uid,
        None => {
            // Record failure
            drop(cache);
            {
                let mut auth_cache = AUTH_FAILURE_CACHE.write().await;
                auth_cache.record_failure(&client_id);
            }
            return Err(matrix_error(
                StatusCode::UNAUTHORIZED,
                "M_UNKNOWN_TOKEN",
                "Invalid or expired access token",
            ));
        }
    };

    // Clear failures on successful auth
    {
        let mut auth_cache = AUTH_FAILURE_CACHE.write().await;
        auth_cache.clear_failures(&client_id);
    }

    Ok(uid)
}

/// Get token from Authorization header (Bearer token)
/// Returns None if the header is missing or invalid
pub fn get_token_from_request(req: &Request) -> Option<String> {
    let auth_header = req.headers().get("Authorization")?;
    let auth_str = auth_header.to_str().ok()?;
    auth_str.strip_prefix("Bearer ").map(|s| s.to_string())
}

/// Validate Matrix access token and return the user id
pub async fn validate_access_token(state: &State, req: &Request) -> Result<i64> {
    // Get token from Authorization header (Bearer token)
    let token = get_token_from_request(req).ok_or_else(|| {
        matrix_error(
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
            "Missing Authorization header",
        )
    })?;

    validate_token_core(state, &token, req).await
}

/// Matrix login endpoint
/// Authenticates a bot user using username and password
/// If device_id is provided, returns existing bot_key; otherwise creates a new device
/// https://spec.matrix.org/v1.11/client-server-api/#post_matrixclientv3login
#[handler]
pub async fn login(
    state: Data<&State>,
    Json(req): Json<LoginRequest>,
    request: &Request,
) -> Result<Json<LoginResponse>> {
    // Validate login type
    if req.login_type != "m.login.password" {
        return Err(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_INVALID_PARAM",
            "Invalid login type. Only m.login.password is supported",
        ));
    }

    // Get Matrix domain from config
    let matrix_domain = get_matrix_domain(&state);

    // Get username from either old format (user) or new format (identifier.user)
    let username_input = match (&req.user, &req.identifier) {
        (Some(user), _) => user.clone(),
        (None, Some(identifier)) => {
            if identifier.id_type != "m.id.user" {
                return Err(matrix_error(
                    StatusCode::BAD_REQUEST,
                    "M_INVALID_PARAM",
                    "Invalid identifier type. Only m.id.user is supported",
                ));
            }
            identifier.user.clone()
        }
        (None, None) => {
            return Err(matrix_error(
                StatusCode::BAD_REQUEST,
                "M_MISSING_PARAM",
                "Missing user identifier",
            ));
        }
    };

    // Validate and parse username format using shared function
    let username = parse_and_validate_matrix_user_id(&username_input, &matrix_domain)?;

    // Get client identifier for rate limiting
    let client_id = get_client_identifier(request, None);

    // Check if client is blocked due to too many failures
    {
        let cache = AUTH_FAILURE_CACHE.read().await;
        if cache.is_blocked(&client_id) {
            let remaining = cache.remaining_block_time(&client_id);
            tracing::warn!(
                "Client {} is rate limited, {} seconds remaining",
                client_id,
                remaining
            );
            return Err(rate_limit_error(remaining));
        }
    }

    tracing::debug!(
        "Login request: username={}, device_id={:?}",
        username,
        req.device_id
    );

    let cache = state.cache.read().await;

    // Step 1: Find user by name who is a bot
    let bot_uid = cache
        .users
        .iter()
        .find(|(_, user)| user.is_bot && user.name.eq_ignore_ascii_case(&username))
        .map(|(uid, _)| *uid);

    let bot_uid = match bot_uid {
        Some(uid) => uid,
        None => {
            // Record failure
            {
                let mut auth_cache = AUTH_FAILURE_CACHE.write().await;
                auth_cache.record_failure(&client_id);
            }
            return Err(matrix_error(
                StatusCode::FORBIDDEN,
                "M_FORBIDDEN",
                "Invalid username or password",
            ));
        }
    };

    // Step 2: Validate password against user.password (NOT bot_key.password)
    let bot_user = cache.users.get(&bot_uid).unwrap();
    let server_key = state.key_config.read().await.server_key.clone();
    let password_valid = match &bot_user.password {
        Some(stored_hash) => verify_password(&req.password, &server_key, stored_hash),
        None => false,
    };
    if !password_valid {
        // Record failure
        {
            let mut auth_cache = AUTH_FAILURE_CACHE.write().await;
            auth_cache.record_failure(&client_id);
        }
        return Err(matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "Invalid username or password",
        ));
    }

    // Clear failures on successful auth
    {
        let mut auth_cache = AUTH_FAILURE_CACHE.write().await;
        auth_cache.clear_failures(&client_id);
    }

    let user_id = format!("@{}:{}", username, matrix_domain);

    // Step 3: Handle device_id
    // Use default device_id if not provided
    let device_id = req.device_id.clone().unwrap_or_else(|| DEFAULT_DEVICE_ID.to_string());

    // Check if device exists
    let matching_key = bot_user.bot_keys.values().find(|k| k.name == device_id);

    let (access_token, device_id) = match matching_key {
        Some(_) => {
            // Device exists - update the api_key (invalidate old token)
            drop(cache);
            update_device_api_key(&state, bot_uid, &device_id).await?
        }
        None => {
            // Device doesn't exist - create new device
            drop(cache);
            create_new_device(&state, bot_uid, &device_id, &user_id, &matrix_domain).await?
        }
    };

    tracing::info!(
        "Matrix login successful for bot user {} with device {}",
        username,
        device_id
    );

    Ok(Json(LoginResponse {
        access_token,
        user_id,
        device_id,
    }))
}

/// Update the api_key for an existing device (invalidate old token)
async fn update_device_api_key(
    state: &State,
    uid: i64,
    device_id: &str,
) -> Result<(String, String)> {
    let mut cache = state.cache.write().await;

    // Find the existing bot_key
    let user = cache
        .users
        .get(&uid)
        .ok_or_else(|| poem::error::Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;

    let (key_id, existing_key) = user
        .bot_keys
        .iter()
        .find(|(_, k)| k.name == device_id)
        .ok_or_else(|| {
            poem::error::Error::from_string("Device not found", StatusCode::NOT_FOUND)
        })?;

    let key_id = *key_id;
    let created_at = existing_key.created_at;

    // Generate new API key
    let server_key = state.key_config.read().await.server_key.clone();
    let new_api_key = create_api_key(uid, &server_key);

    // Update database
    let now = DateTime::now();
    sqlx::query("update `bot_key` set key = ?, updated_at = ?, last_used = ? where id = ?")
        .bind(&new_api_key)
        .bind(now)
        .bind(now)
        .bind(key_id)
        .execute(&state.db_pool)
        .await
        .map_err(InternalServerError)?;

    // Update cache
    let user = cache.users.get_mut(&uid).unwrap();
    user.bot_keys.insert(
        key_id,
        BotKey {
            name: device_id.to_string(),
            key: new_api_key.clone(),
            created_at,
            last_used: Some(now),
        },
    );

    tracing::info!(
        "Updated API key for uid={}, device_id={}",
        uid,
        device_id
    );

    Ok((new_api_key, device_id.to_string()))
}

/// Create a new bot_key and initialize Matrix data for a device
async fn create_new_device(
    state: &State,
    uid: i64,
    device_id: &str,
    user_id: &str,
    matrix_domain: &str,
) -> Result<(String, String)> {
    let mut cache = state.cache.write().await;

    // Check for conflict
    let user = cache
        .users
        .get(&uid)
        .ok_or_else(|| poem::error::Error::from_status(StatusCode::INTERNAL_SERVER_ERROR))?;

    if user.bot_keys.values().any(|k| k.name == device_id) {
        return Err(poem::error::Error::from_string(
            "Device ID already exists",
            StatusCode::CONFLICT,
        ));
    }

    // Create API key
    let server_key = state.key_config.read().await.server_key.clone();
    let api_key = create_api_key(uid, &server_key);

    // Insert into database
    let now = DateTime::now();
    let key_id =
        sqlx::query("insert into `bot_key` (uid, name, key, created_at) values (?, ?, ?, ?)")
            .bind(uid)
            .bind(device_id)
            .bind(&api_key)
            .bind(now)
            .execute(&state.db_pool)
            .await
            .map_err(InternalServerError)?
            .last_insert_rowid();

    // Update cache
    let user = cache.users.get_mut(&uid).unwrap();
    user.bot_keys.insert(
        key_id,
        BotKey {
            name: device_id.to_string(),
            key: api_key.clone(),
            created_at: now,
            last_used: None,
        },
    );

    // Initialize Matrix data for this device
    drop(cache);
    initialize_device_matrix_data(state, uid, device_id, user_id, matrix_domain, &server_key)
        .await?;

    Ok((api_key, device_id.to_string()))
}

/// Initialize Matrix E2EE data for a new device
async fn initialize_device_matrix_data(
    state: &State,
    uid: i64,
    device_id: &str,
    user_id: &str,
    _matrix_domain: &str,
    server_key: &str,
) -> Result<()> {
    // Initialize a new Olm Account for this device
    state
        .server_olm_account_manager
        .create_device_account(uid, device_id, user_id, server_key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to initialize Olm account for device: {}", e);
            InternalServerError(e)
        })?;

    tracing::info!(
        "Initialized Matrix data for uid={}, device_id={}",
        uid,
        device_id
    );

    Ok(())
}

/// Update bot_key.last_used to current time
pub async fn update_bot_key_last_used(state: &State, uid: i64, token: &str) {
    let now = DateTime::now();
    // Update cache
    let mut cache = state.cache.write().await;
    if let Some(user) = cache.users.get_mut(&uid) {
        if let Some(bot_key) = user.bot_keys.values_mut().find(|bk| bk.key == token) {
            bot_key.last_used = Some(now);
        }
    }
}
