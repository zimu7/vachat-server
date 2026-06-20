//! Matrix account module - handles user account endpoints

use poem::{
    handler,
    http::StatusCode,
    web::{Data, Json},
    Body, Request, Result,
};
use serde_json::{json, Value};

use crate::state::State;

/// Matrix devices endpoint - handles device listing and deletion
#[handler]
pub async fn devices_handler(state: Data<&State>, _body: Body, req: &Request) -> Result<Json<Value>> {
    // Validate access token
    let uid = super::auth::validate_access_token(&state, req).await?;

    let path = req.original_uri().path();
    let method = req.method();

    tracing::info!("devices_handler: path={}, method={}, uid={}", path, method, uid);

    // Parse device_id from path (e.g., /_matrix/client/v3/devices/BOTDEVICE)
    let device_id = path
        .strip_prefix("/_matrix/client/v3/devices")
        .map(|s| s.trim_start_matches('/').trim_end_matches('/'))
        .unwrap_or("");

    // Handle DELETE /_matrix/client/v3/devices/{device_id}
    if method == poem::http::Method::DELETE {
        if device_id.is_empty() {
            return Err(super::auth::matrix_error(
                StatusCode::BAD_REQUEST,
                "M_MISSING_PARAM",
                "Missing device_id",
            ));
        }

        tracing::info!("Deleting device: uid={}, device_id={}", uid, device_id);

        // Delete device keys and OTKs
        let _ = sqlx::query("DELETE FROM matrix_device_keys WHERE uid = ? AND device_id = ?")
            .bind(uid)
            .bind(device_id)
            .execute(&state.db_pool)
            .await;

        let _ = sqlx::query("DELETE FROM matrix_device_otk WHERE uid = ? AND device_id = ?")
            .bind(uid)
            .bind(device_id)
            .execute(&state.db_pool)
            .await;

        // Also delete from server_olm_account if it's SERVERDEVICE
        if device_id == "SERVERDEVICE" {
            let _ = sqlx::query("DELETE FROM matrix_olm_account WHERE uid = ? AND device_id = ?")
                .bind(uid)
                .bind(device_id)
                .execute(&state.db_pool)
                .await;
        }

        return Ok(Json(json!({})));
    }

    // Handle GET /_matrix/client/v3/devices or GET /_matrix/client/v3/devices/{device_id}
    if method == poem::http::Method::GET {
        // Return list of devices
        let cache = state.cache.read().await;
        let user = cache.users.get(&uid);
        let bot_device_id = user
            .and_then(|u| u.bot_keys.values().next().map(|bk| bk.name.clone()))
            .unwrap_or_else(|| "BOTDEVICE".to_string());
        drop(cache);

        // If specific device_id requested, return that device
        if !device_id.is_empty() {
            return Ok(Json(json!({
                "device_id": device_id,
                "display_name": device_id,
                "last_seen_ts": 0,
                "last_seen_ip": "",
                "user_id": ""
            })));
        }

        // Return all devices
        return Ok(Json(json!({
            "devices": [
                {
                    "device_id": bot_device_id,
                    "display_name": bot_device_id,
                    "last_seen_ts": 0,
                    "last_seen_ip": "",
                    "user_id": ""
                },
                {
                    "device_id": "SERVERDEVICE",
                    "display_name": "SERVERDEVICE",
                    "last_seen_ts": 0,
                    "last_seen_ip": "",
                    "user_id": ""
                }
            ]
        })));
    }

    Err(super::auth::matrix_error(
        StatusCode::METHOD_NOT_ALLOWED,
        "M_UNRECOGNIZED",
        "Method not allowed",
    ))
}

/// Matrix whoami endpoint - returns the user ID and device ID associated with the access token
#[handler]
pub async fn whoami(state: Data<&State>, req: &Request) -> Result<Json<Value>> {
    // Get token from Authorization header
    let auth_header = req.headers().get("Authorization").ok_or_else(|| {
        super::auth::matrix_error(
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
            "Missing Authorization header",
        )
    })?;

    let auth_str = auth_header.to_str().map_err(|_| {
        super::auth::matrix_error(
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
            "Invalid Authorization header",
        )
    })?;

    let token = auth_str.strip_prefix("Bearer ").ok_or_else(|| {
        super::auth::matrix_error(
            StatusCode::UNAUTHORIZED,
            "M_MISSING_TOKEN",
            "Invalid Authorization header format. Expected 'Bearer <token>'",
        )
    })?;

    // Validate access token and get uid
    let uid = super::auth::validate_token_core(&state, token, req).await?;

    // Update last_used in DB and cache
    super::auth::update_bot_key_last_used(&state, uid, token).await;

    // Get user info from cache
    let cache = state.cache.read().await;
    let user = cache.users.get(&uid).ok_or_else(|| {
        super::auth::matrix_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "M_UNKNOWN",
            "User not found in cache",
        )
    })?;

    let matrix_domain = super::auth::get_matrix_domain(&state);
    let user_id = format!("@{}:{}", user.name, matrix_domain);

    // Get device_id from the bot_key associated with the access token
    let device_id = user
        .bot_keys
        .values()
        .find(|bot_key| bot_key.key == token)
        .map(|bot_key| bot_key.name.clone())
        .unwrap_or_else(|| "BOTDEVICE".to_string());

    Ok(Json(json!({
        "user_id": user_id,
        "device_id": device_id
    })))
}

/// Unified handler for all user-related endpoints
#[handler]
pub async fn user_handler(state: Data<&State>, body: Body, req: &Request) -> Result<Json<Value>> {
    // Validate access token and get uid
    let uid = super::auth::validate_access_token(&state, req).await?;

    let path = req.original_uri().path();

    tracing::debug!("user_handler, path={}", path);

    // Check if it's a user endpoint
    if !path.starts_with("/_matrix/client/v3/user/") {
        return Err(poem::error::Error::from_string(
            "Invalid path",
            StatusCode::BAD_REQUEST,
        ));
    }

    let path_suffix = path
        .strip_prefix("/_matrix/client/v3/user/")
        .ok_or_else(|| poem::error::Error::from_string("Invalid path", StatusCode::BAD_REQUEST))?;

    // Find the next '/' to separate user_id from the rest
    let user_id_end = path_suffix.find('/').ok_or_else(|| {
        tracing::warn!("Invalid path format for user_handler, full path: {}", path);
        poem::error::Error::from_string("Invalid path format", StatusCode::BAD_REQUEST)
    })?;

    let user_id_encoded = &path_suffix[..user_id_end];
    let remaining_path = &path_suffix[user_id_end + 1..];

    // Decode the user_id (it may contain URL-encoded characters like ':' -> '%3A')
    let user_id = super::auth::decode_path_segment(user_id_encoded)?;

    let matrix_domain = super::auth::get_matrix_domain(&state);

    // Validate that the authenticated user matches the user_id in the path
    let expected_user_name = super::auth::parse_and_validate_matrix_user_id(&user_id, &matrix_domain)?;

    // Get the authenticated user's info from cache
    let auth_user_name = {
        let cache = state.cache.read().await;
        let auth_user = cache.users.get(&uid).ok_or_else(|| {
            super::auth::matrix_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "M_UNKNOWN",
                "User not found in cache",
            )
        })?;
        auth_user.name.clone()
    };

    // Verify the user_id matches the authenticated user
    if auth_user_name != expected_user_name {
        return Err(super::auth::matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "Cannot access data for other users",
        ));
    }

    // Route based on remaining path
    if remaining_path == "filter" {
        handle_create_filter().await
    } else if remaining_path == "account_data/m.direct" {
        handle_account_data_direct(&state, body, req, &matrix_domain).await
    } else if remaining_path == "account_data/m.secret_storage.default_key" {
        tracing::info!("Secret storage default key requested, returning 404 (not supported)");
        Err(super::auth::matrix_error(
            StatusCode::NOT_FOUND,
            "M_NOT_FOUND",
            "No default secret storage key found - secret storage is not supported",
        ))
    } else {
        tracing::warn!("Unknown user endpoint: {}", remaining_path);
        Err(super::auth::matrix_error(
            StatusCode::NOT_FOUND,
            "M_UNRECOGNIZED",
            "Unknown endpoint",
        ))
    }
}

/// Handle Matrix filter creation
async fn handle_create_filter() -> Result<Json<Value>> {
    Ok(Json(json!({
        "filter_id": "0"
    })))
}

/// Handle Matrix account data for m.direct
async fn handle_account_data_direct(
    state: &State,
    _body: Body,
    req: &Request,
    matrix_domain: &str,
) -> Result<Json<Value>> {
    // Get authenticated user's info from cache
    let uid = super::auth::validate_access_token(state, req).await?;
    let cache = state.cache.read().await;
    let auth_user = cache.users.get(&uid).ok_or_else(|| {
        super::auth::matrix_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "M_UNKNOWN",
            "User not found in cache",
        )
    })?;

    // Handle based on HTTP method
    let method = req.method();

    if method == poem::http::Method::GET {
        // Build m.direct response from room_cache
        let rooms = super::sync::get_bot_rooms(uid).await;

        let mut direct_map: serde_json::Map<String, Value> = serde_json::Map::new();

        for room_id in rooms {
            // Only include DM rooms in m.direct
            if !room_id.starts_with("!dm_") {
                continue;
            }

            let user_matrix_id = format!("@{}:{}", auth_user.name, matrix_domain);
            direct_map.insert(user_matrix_id, json!([room_id.clone()]));
        }

        let result = json!(direct_map);
        tracing::info!(
            "m.direct response: {}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
        Ok(Json(Value::Object(direct_map)))
    } else {
        Err(super::auth::matrix_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "M_UNRECOGNIZED",
            "Method not allowed",
        ))
    }
}

/// Matrix pushrules endpoint - returns empty push rules
#[handler]
pub async fn pushrules(state: Data<&State>, req: &Request) -> Result<Json<Value>> {
    // Validate access token
    let _uid = super::auth::validate_access_token(&state, req).await?;

    // Return empty push rules
    Ok(Json(json!({
        "global": {
            "content": [],
            "override": [],
            "room": [],
            "sender": [],
            "underride": []
        }
    })))
}

/// Matrix capabilities endpoint - returns server capabilities
#[handler]
pub async fn capabilities(state: Data<&State>, req: &Request) -> Result<Json<Value>> {
    // Validate access token
    let _uid = super::auth::validate_access_token(&state, req).await?;

    // Return minimal capabilities
    Ok(Json(json!({
        "capabilities": {
            "m.change_password": {
                "enabled": false
            },
            "m.room_versions": {
                "default": "6",
                "available": {}
            }
        }
    })))
}

/// Matrix versions endpoint - returns supported API versions
#[handler]
pub async fn versions(_req: &Request) -> Result<Json<Value>> {
    // No authentication required for versions endpoint
    Ok(Json(json!({
        "versions": ["v1.11", "v1.10", "v1.9", "v1.8", "v1.7", "v1.6", "v1.5", "v1.4", "v1.3", "v1.2", "v1.1", "v1.0"],
        "unstable_features": {}
    })))
}
