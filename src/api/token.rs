use std::{ops::Deref, time::Duration};

use bytes::Bytes;
use chrono::Utc;
use futures_util::TryFutureExt;
use hmac::{Mac, NewMac};
use openidconnect::{
    core::{
        CoreAuthenticationFlow, CoreClient, CoreClientRegistrationRequest, CoreIdTokenClaims,
        CoreIdTokenVerifier, CoreJwsSigningAlgorithm, CoreProviderMetadata,
    },
    registration::EmptyAdditionalClientMetadata,
    AuthorizationCode, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge, RedirectUrl, Scope,
};
use poem::{
    error::{BadRequest, InternalServerError, ServiceUnavailable},
    http::StatusCode,
    web::Data,
    Error, Request, Result,
};
use poem_openapi::{
    auth::ApiKey, param::Header, payload::Json, types::Example, ApiResponse, Object, OpenApi,
    SecurityScheme, Union,
};
use rc_token::{parse_token, TokenType};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::{
    api::{
        admin_login::{LoginConfig, WhoCanSignUp},
        tags::ApiTags,
        user::UserInfo,
        DateTime, KickReason,
    },
    create_user::{CreateUser, CreateUserBy},
    middleware::guest_forbidden,
    password::verify_password,
    state::{CacheDevice, OAuth2State, UserEvent, UserStatus},
    State,
};

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct CurrentUser {
    pub uid: i64,
    pub device: String,
    pub is_admin: bool,
    pub is_guest: bool,
}

/// ApiKey authorization
#[derive(SecurityScheme)]
#[oai(
    type = "api_key",
    key_name = "X-API-Key",
    in = "header",
    checker = "api_checker"
)]
pub struct Token(pub CurrentUser);

impl Deref for Token {
    type Target = CurrentUser;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// ApiKey authorization
#[derive(SecurityScheme)]
#[oai(
    type = "api_key",
    key_name = "api-key",
    in = "query",
    checker = "api_checker"
)]
pub struct TokenInQuery(pub CurrentUser);

impl Deref for TokenInQuery {
    type Target = CurrentUser;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

async fn api_checker(req: &Request, api_key: ApiKey) -> Option<CurrentUser> {
    let state = req.extensions().get::<State>().unwrap();
    let key_config = state.key_config.read().await;
    let (token_type, current_user): (_, CurrentUser) =
        parse_token(&key_config.server_key, &api_key.key, true).ok()?;
    if token_type != TokenType::AccessToken {
        return None;
    }
    Some(current_user)
}

#[derive(Debug, Object)]
struct LoginCredentialPassword {
    /// Username or email
    account: String,

    /// Password
    password: String,
}

#[derive(Debug, Object)]
struct LoginCredentialOpenIdConnect {
    code: String,
    #[oai(rename = "state")]
    oidc_state: String,
}

#[derive(Debug, Object)]
struct LoginCredentialThirdParty {
    key: String,
}

/// Login credential
#[derive(Debug, Union)]
#[oai(discriminator_name = "type")]
enum LoginCredential {
    #[oai(mapping = "password")]
    Password(LoginCredentialPassword),
    #[oai(mapping = "oidc")]
    OpenIdConnect(LoginCredentialOpenIdConnect),
    #[oai(mapping = "thirdparty")]
    ThirdParty(LoginCredentialThirdParty),
}

/// Login request
#[derive(Debug, Object)]
#[oai(example)]
struct LoginRequest {
    /// Credential
    credential: LoginCredential,

    /// Device id
    #[oai(default = "default_device")]
    device: String,

    /// FCM device token
    device_token: Option<String>,
}

impl Example for LoginRequest {
    fn example() -> Self {
        LoginRequest {
            credential: LoginCredential::Password(LoginCredentialPassword {
                account: "admin@zimu.pub".to_string(),
                password: "123456".to_string(),
            }),
            device: "web".to_string(),
            device_token: None,
        }
    }
}

fn default_device() -> String {
    "unknown".to_string()
}

/// Token response
#[derive(Debug, Object)]
pub struct LoginResponse {
    /// Server id
    server_id: String,
    /// Access token
    token: String,
    /// Refresh token
    refresh_token: String,
    /// The access token expired in seconds
    expired_in: i64,
    /// User info
    user: UserInfo,
}

#[derive(ApiResponse)]
pub enum LoginApiResponse {
    /// Login success
    #[oai(status = 200)]
    Ok(Json<LoginResponse>),
    /// Login method does not supported
    #[oai(status = 403)]
    LoginMethodNotSupported,
    /// Invalid account or password
    #[oai(status = 401)]
    InvalidAccount,
    /// User does not exists
    #[oai(status = 404)]
    UserDoesNotExist,
    /// User has been frozen
    #[oai(status = 423)]
    Frozen,
    /// Email collision
    #[oai(status = 409)]
    EmailConflict,
    /// Account not associated
    #[oai(status = 410)]
    AccountNotAssociated,
}

/// Bind credential
#[derive(Debug, Union)]
#[oai(discriminator_name = "type")]
enum BindCredential {
    #[oai(mapping = "oidc")]
    OpenIdConnect(LoginCredentialOpenIdConnect),
}

/// Bind request
#[derive(Debug, Object)]
struct BindRequest {
    /// Credential
    credential: BindCredential,
}

#[derive(ApiResponse)]
enum BindApiResponse {
    /// Login success
    #[oai(status = 200)]
    Ok,
    /// Login method does not supported
    #[oai(status = 403)]
    #[allow(dead_code)]
    LoginMethodNotSupported,
    /// Invalid credential
    #[oai(status = 401)]
    InvalidCredential,
    #[oai(status = 409)]
    Exists,
}

/// Credentials response
#[derive(Debug, Object)]
struct CredentialsResponse {
    password: bool,
    oidc: Vec<String>,
}

/// Renew token request
#[derive(Debug, Object)]
struct RenewTokenRequest {
    token: String,
    refresh_token: String,
}

/// Renew token response
#[derive(Debug, Object)]
struct RenewTokenResponse {
    /// Access token
    token: String,
    /// Refresh token
    refresh_token: String,
    /// The access token expired in seconds
    expired_in: i64,
}

#[derive(ApiResponse)]
enum RenewTokenApiResponse {
    /// Renew success
    #[oai(status = 200)]
    Ok(Json<RenewTokenResponse>),
    /// Illegal token
    #[oai(status = 401)]
    IllegalToken,
}

#[derive(ApiResponse)]
enum LogoutApiResponse {
    /// Logout success
    #[oai(status = 200)]
    Ok,
    /// Illegal token
    #[oai(status = 401)]
    IllegalToken,
}

#[derive(Debug, Object)]
struct OpenIdAuthorizeRequest {
    issuer: String,
    redirect_uri: String,
}

#[derive(Debug, Object)]
struct OpenIdAuthorizeResponse {
    url: String,
}

#[derive(Debug, Object)]
struct CreateThirdPartyKeyRequest {
    userid: String,
    username: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ThirdPartyLoginInfo {
    userid: String,
    username: String,
    expired_at: chrono::DateTime<Utc>,
}

/// Update device token request
#[derive(Debug, Object)]
struct UpdateDeviceTokenRequest {
    device_token: Option<String>,
}

pub struct ApiToken;

#[OpenApi(prefix_path = "/token", tag = "ApiTags::Token")]
impl ApiToken {
    /// OpenId authorize
    #[oai(path = "/openid/authorize", method = "post")]
    async fn openid_authorize(
        &self,
        state: Data<&State>,
        req: Json<OpenIdAuthorizeRequest>,
    ) -> Result<Json<OpenIdAuthorizeResponse>> {
        let mut provider_metadata = None;
        let login_cfg = state
            .get_dynamic_config_instance::<LoginConfig>()
            .await
            .unwrap_or_default();
        if !login_cfg
            .oidc
            .iter()
            .any(|oidc| oidc.domain == req.issuer && oidc.enable)
        {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        for url in [
            format!("https://{}", req.0.issuer),
            format!("https://{}/", req.0.issuer),
        ] {
            let issuer_url = IssuerUrl::new(url).map_err(BadRequest)?;
            if let Ok(metadata) = CoreProviderMetadata::discover_async(
                issuer_url.clone(),
                openidconnect::reqwest::async_http_client,
            )
            .await
            {
                provider_metadata = Some((issuer_url, metadata));
                break;
            }
        }

        let (issuer_url, provider_metadata) =
            provider_metadata.ok_or_else(|| Error::from_status(StatusCode::SERVICE_UNAVAILABLE))?;
        let redirect_uri = RedirectUrl::new(req.0.redirect_uri).map_err(BadRequest)?;

        let registration_endpoint = provider_metadata
            .registration_endpoint()
            .ok_or_else(|| Error::from_status(StatusCode::SERVICE_UNAVAILABLE))?;
        tracing::debug!(
            issuer_url = issuer_url.as_str(),
            redirect_uri = redirect_uri.as_str(),
            registration_endpoint = registration_endpoint.as_str(),
            "registration endpoint",
        );

        let registration_response = CoreClientRegistrationRequest::new(
            vec![redirect_uri.clone()],
            EmptyAdditionalClientMetadata::default(),
        )
        .register_async(
            registration_endpoint,
            openidconnect::reqwest::async_http_client,
        )
        .await
        .map_err(ServiceUnavailable)?;

        tracing::debug!(
            issuer_url = issuer_url.as_str(),
            redirect_uri = redirect_uri.as_str(),
            "openid authorize",
        );

        let client = CoreClient::new(
            registration_response.client_id().clone(),
            registration_response.client_secret().cloned(),
            provider_metadata.issuer().clone(),
            provider_metadata.authorization_endpoint().clone(),
            provider_metadata.token_endpoint().cloned(),
            provider_metadata.userinfo_endpoint().cloned(),
            provider_metadata.jwks().clone(),
        )
        .set_redirect_uri(redirect_uri);

        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let (auth_url, csrf_token, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("openid".to_string()))
            .set_pkce_challenge(pkce_challenge)
            .url();

        tracing::debug!(
            authorize_url = auth_url.as_str(),
            "authorization url generated"
        );

        state.pending_oidc.lock().await.insert(
            csrf_token.secret().to_string(),
            OAuth2State {
                client,
                issuer: req.0.issuer,
                pkce_verifier,
                csrf_token: csrf_token.clone(),
                nonce,
            },
        );

        tokio::spawn({
            let state = state.clone();
            async move {
                // timeout
                tokio::time::sleep(Duration::from_secs(150)).await;
                state.pending_oidc.lock().await.remove(csrf_token.secret());
            }
        });

        Ok(Json(OpenIdAuthorizeResponse {
            url: auth_url.to_string(),
        }))
    }

    /// Create a key for third-party user login.
    #[oai(method = "post", path = "/create_third_party_key")]
    async fn create_third_party_key(
        &self,
        state: Data<&State>,
        req: Json<CreateThirdPartyKeyRequest>,
        #[oai(name = "X-SECRET")] secret: Header<String>,
    ) -> Result<Json<String>> {
        let key_config = state.key_config.read().await;
        if secret.0 != key_config.third_party_secret {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        let info = ThirdPartyLoginInfo {
            userid: req.0.userid,
            username: req.0.username,
            expired_at: chrono::Utc::now() + chrono::Duration::seconds(60 * 2),
        };
        let key_content = serde_json::to_vec(&info).unwrap();
        let mut buf_sig = {
            let mut mac =
                hmac::Hmac::<Sha256>::new_from_slice(key_config.server_key.as_bytes()).unwrap();
            mac.update(&key_content);
            mac.finalize().into_bytes().to_vec()
        };
        buf_sig.extend_from_slice(&key_content);
        Ok(Json(hex::encode(&buf_sig)))
    }

    /// Login as guest
    #[oai(path = "/login_guest", method = "get")]
    async fn login_guest(&self, state: Data<&State>) -> Result<LoginApiResponse> {
        let login_cfg = state
            .get_dynamic_config_instance::<LoginConfig>()
            .await
            .unwrap_or_default();
        if !login_cfg.guest {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        let name = state.cache.read().await.assign_username(None, None);
        let (uid, _) = state
            .create_user(CreateUser::new(&name, CreateUserBy::Guest, false))
            .await
            .map_err(|err| {
                Error::from_string(format!("{:?}", err), StatusCode::INTERNAL_SERVER_ERROR)
            })?;
        do_login(&state, uid, "guest_device", None).await
    }

    /// Login
    #[oai(path = "/login", method = "post")]
    async fn login(
        &self,
        state: Data<&State>,
        req: Json<LoginRequest>,
        _request: &Request,
    ) -> Result<LoginApiResponse> {
        let login_cfg = state
            .get_dynamic_config_instance::<LoginConfig>()
            .await
            .unwrap_or_default();

        let can_register = !matches!(login_cfg.who_can_sign_up, WhoCanSignUp::InvitationOnly);

        let uid = match req.0.credential {
            // login with password
            LoginCredential::Password(LoginCredentialPassword { account, password })
                if login_cfg.password =>
            {
                let account = account.to_lowercase();
                let cache = state.cache.read().await;
                let server_key = state.key_config.read().await.server_key.clone();
                let uid = match cache.users.iter().find(|(_, user)| {
                    user.email
                        .as_ref()
                        .map_or(false, |e| e.eq_ignore_ascii_case(&account))
                        || user.name.eq_ignore_ascii_case(&account)
                }) {
                    Some((uid, cached_user)) => {
                        if let Some(stored_hash) = &cached_user.password {
                            if !verify_password(&password, &server_key, stored_hash) {
                                return Ok(LoginApiResponse::InvalidAccount);
                            }
                        } else {
                            return Ok(LoginApiResponse::InvalidAccount);
                        }
                        *uid
                    }
                    None => return Ok(LoginApiResponse::UserDoesNotExist),
                };
                uid
            }

            // login with open id connect
            LoginCredential::OpenIdConnect(LoginCredentialOpenIdConnect {
                code,
                oidc_state,
                ..
            }) => {
                let item = {
                    let mut pending_oidc = state.pending_oidc.lock().await;
                    match pending_oidc.remove(&oidc_state) {
                        Some(item) => item,
                        None => {
                            tracing::debug!("oidc id does not exist");
                            return Ok(LoginApiResponse::InvalidAccount);
                        }
                    }
                };

                tracing::debug!("wait for oauth2 response");

                let issuer = item.issuer.clone();
                let id_token = match oidc_get_id_token(item, code, oidc_state).await {
                    Ok(id_token) => id_token,
                    Err(_) => return Ok(LoginApiResponse::InvalidAccount),
                };

                let subject = id_token.subject().to_string();
                let email = if id_token.email_verified().unwrap_or_default() {
                    id_token.email().map(|email| email.to_string())
                } else {
                    None
                };
                let name = id_token
                    .name()
                    .and_then(|name| name.iter().next().map(|(_, name)| name.to_string()));
                let picture = id_token.picture().and_then(|picture| {
                    picture
                        .iter()
                        .next()
                        .map(|(_, picture)| picture.to_string())
                });

                let sql = "select uid from openid_connect where issuer = ? and subject = ?";
                match sqlx::query_as::<_, (i64,)>(sql)
                    .bind(&issuer)
                    .bind(&subject)
                    .fetch_optional(&state.db_pool)
                    .await
                    .map_err(InternalServerError)?
                {
                    Some((uid,)) => uid,
                    // create a new user
                    None => {
                        if !can_register {
                            return Ok(LoginApiResponse::AccountNotAssociated);
                        }

                        // download avatar
                        let avatar = match &picture {
                            Some(url) => download_avatar(url).await.ok(),
                            None => None,
                        };

                        let name = state
                            .cache
                            .write()
                            .await
                            .assign_username(name.as_deref(), email.as_deref());

                        let mut create_user = CreateUser::new(
                            &name,
                            CreateUserBy::OpenIdConnect {
                                issuer: &issuer,
                                subject: &subject,
                                email: email.as_deref(),
                            },
                            false,
                        );
                        if let Some(avatar) = &avatar {
                            create_user = create_user.avatar(avatar);
                        }

                        match state.create_user(create_user).await {
                            Ok((uid, _)) => uid,
                            Err(_) => return Ok(LoginApiResponse::InvalidAccount),
                        }
                    }
                }
            }

            LoginCredential::ThirdParty(LoginCredentialThirdParty { key })
                if login_cfg.third_party =>
            {
                let data = match hex::decode(&key) {
                    Ok(data) => data,
                    Err(_) => return Ok(LoginApiResponse::InvalidAccount),
                };
                let key_config = state.key_config.read().await;
                let mut mac =
                    hmac::Hmac::<Sha256>::new_from_slice(key_config.server_key.as_bytes()).unwrap();
                mac.update(&data[32..]);
                if mac.verify(&data[..32]).is_err() {
                    return Ok(LoginApiResponse::InvalidAccount);
                }
                let info: ThirdPartyLoginInfo = match serde_json::from_slice(&data[32..]) {
                    Ok(info) => info,
                    Err(_) => return Ok(LoginApiResponse::InvalidAccount),
                };
                if info.expired_at < Utc::now() {
                    return Ok(LoginApiResponse::InvalidAccount);
                }

                let sql = "select uid from third_party_users where userid = ?";
                match sqlx::query_as::<_, (i64,)>(sql)
                    .bind(&info.userid)
                    .fetch_optional(&state.db_pool)
                    .await
                    .map_err(InternalServerError)?
                {
                    Some((uid,)) => uid,
                    // create a new user
                    None => {
                        let name = state
                            .cache
                            .write()
                            .await
                            .assign_username(Some(&info.username), None);
                        let create_user = CreateUser::new(
                            &name,
                            CreateUserBy::ThirdParty {
                                thirdparty_uid: &info.userid,
                                username: &info.username,
                            },
                            false,
                        );
                        match state.create_user(create_user).await {
                            Ok((uid, _)) => uid,
                            Err(_) => return Ok(LoginApiResponse::InvalidAccount),
                        }
                    }
                }
            }
            _ => return Ok(LoginApiResponse::LoginMethodNotSupported),
        };

        do_login(&state, uid, &req.0.device, req.0.device_token.as_deref()).await
    }

    /// Bind credential
    #[oai(path = "/bind", method = "post", transform = "guest_forbidden")]
    async fn bind(
        &self,
        state: Data<&State>,
        token: Token,
        req: Json<BindRequest>,
    ) -> Result<BindApiResponse> {
        let _login_cfg = state
            .get_dynamic_config_instance::<LoginConfig>()
            .await
            .unwrap_or_default();

        let BindCredential::OpenIdConnect(LoginCredentialOpenIdConnect {
            code, oidc_state, ..
        }) = req.0.credential;

        let item = {
            let mut pending_oidc = state.pending_oidc.lock().await;
            match pending_oidc.remove(&oidc_state) {
                Some(item) => item,
                None => {
                    tracing::debug!("oidc id does not exist");
                    return Ok(BindApiResponse::InvalidCredential);
                }
            }
        };

        if get_credentials_openid(&state, token.uid)
            .await?
            .iter()
            .any(|issuer| &item.issuer == issuer)
        {
            return Ok(BindApiResponse::Exists);
        }

        tracing::debug!("wait for oauth2 response");

        let issuer = item.issuer.clone();
        let id_token = match oidc_get_id_token(item, code, oidc_state).await {
            Ok(id_token) => id_token,
            Err(_) => return Ok(BindApiResponse::InvalidCredential),
        };

        let sql = "insert into openid_connect (issuer, subject, uid) values (?, ?, ?)";
        sqlx::query(sql)
            .bind(&issuer)
            .bind(id_token.subject().as_str())
            .bind(token.uid)
            .execute(&state.db_pool)
            .await
            .map_err(InternalServerError)?;

        Ok(BindApiResponse::Ok)
    }

    /// Get the credentials of current user
    #[oai(path = "/credentials", method = "get")]
    async fn credentials(
        &self,
        state: Data<&State>,
        token: Token,
    ) -> Result<Json<CredentialsResponse>> {
        let cache = state.cache.read().await;
        let cached_user = cache
            .users
            .get(&token.uid)
            .ok_or_else(|| Error::from(StatusCode::UNAUTHORIZED))?;

        Ok(Json(CredentialsResponse {
            password: cached_user.password.is_some(),
            oidc: get_credentials_openid(&state, token.uid).await?,
        }))
    }

    /// Renew the refresh token
    #[oai(path = "/renew", method = "post")]
    async fn renew(
        &self,
        state: Data<&State>,
        req: Json<RenewTokenRequest>,
    ) -> Result<RenewTokenApiResponse> {
        let key_config = state.key_config.read().await;
        let (token_type1, current_user1): (TokenType, CurrentUser) =
            match rc_token::parse_token(&key_config.server_key, &req.token, false) {
                Ok(res) => res,
                Err(_) => return Ok(RenewTokenApiResponse::IllegalToken),
            };
        if token_type1 != TokenType::AccessToken {
            return Ok(RenewTokenApiResponse::IllegalToken);
        }

        let (token_type2, current_user2): (TokenType, CurrentUser) =
            match rc_token::parse_token(&key_config.server_key, &req.refresh_token, true) {
                Ok(res) => res,
                Err(_) => return Ok(RenewTokenApiResponse::IllegalToken),
            };
        if token_type2 != TokenType::RefreshToken {
            return Ok(RenewTokenApiResponse::IllegalToken);
        }

        if current_user1 != current_user2 {
            return Ok(RenewTokenApiResponse::IllegalToken);
        }

        let (prev_refresh_token,) = match sqlx::query_as::<_, (String,)>(
            "select token from refresh_token where uid = ? and device = ?",
        )
        .bind(current_user1.uid)
        .bind(&current_user1.device)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(InternalServerError)?
        {
            Some(res) => res,
            None => return Ok(RenewTokenApiResponse::IllegalToken),
        };

        if prev_refresh_token != req.refresh_token {
            return Ok(RenewTokenApiResponse::IllegalToken);
        }

        let (refresh_token, token) = rc_token::create_token_pair(
            &key_config.server_key,
            current_user1.clone(),
            state.config.system.refresh_token_expiry_seconds,
            state.config.system.token_expiry_seconds,
        )
        .map_err(InternalServerError)?;

        sqlx::query("update refresh_token set token = ? where uid = ? and device = ?")
            .bind(&refresh_token)
            .bind(current_user1.uid)
            .bind(current_user1.device)
            .execute(&state.db_pool)
            .await
            .map_err(InternalServerError)?;

        Ok(RenewTokenApiResponse::Ok(Json(RenewTokenResponse {
            token,
            refresh_token,
            expired_in: state.config.system.token_expiry_seconds,
        })))
    }

    /// Logout
    #[oai(path = "/logout", method = "get")]
    async fn logout(&self, state: Data<&State>, token: Token) -> Result<LogoutApiResponse> {
        let mut cache = state.cache.write().await;
        let cached_user = match cache.users.get_mut(&token.uid) {
            Some(cached_user) => cached_user,
            None => return Ok(LogoutApiResponse::IllegalToken),
        };

        // begin transaction
        let mut tx = state.db_pool.begin().await.map_err(InternalServerError)?;

        sqlx::query("delete from refresh_token where uid = ? and device = ?")
            .bind(token.uid)
            .bind(&token.device)
            .execute(&mut tx)
            .await
            .map_err(InternalServerError)?;

        sqlx::query("delete from device where uid = ? and device = ?")
            .bind(token.uid)
            .bind(&token.device)
            .execute(&mut tx)
            .await
            .map_err(InternalServerError)?;

        // commit transaction
        tx.commit().await.map_err(InternalServerError)?;

        // close events connection
        if let Some(sender) = cached_user
            .devices
            .get_mut(&token.device)
            .and_then(|device| device.sender.take())
        {
            let _ = sender.send(UserEvent::Kick {
                reason: KickReason::Logout,
            });
        }

        cached_user.devices.remove(&token.device);
        Ok(LogoutApiResponse::Ok)
    }

    /// Update FCM device token
    #[oai(path = "/device_token", method = "put", transform = "guest_forbidden")]
    async fn update_device_token(
        &self,
        state: Data<&State>,
        token: Token,
        req: Json<UpdateDeviceTokenRequest>,
    ) -> Result<()> {
        let mut cache = state.cache.write().await;
        let cached_user = match cache.users.get_mut(&token.uid) {
            Some(cached_user) => cached_user,
            None => return Err(Error::from_status(StatusCode::NOT_FOUND)),
        };

        // update sqlite
        let sql = "update device set device_token = ?, updated_at = ? where uid = ? and device = ?";
        sqlx::query(sql)
            .bind(&req.device_token)
            .bind(DateTime::now())
            .bind(token.uid)
            .bind(&token.device)
            .execute(&state.db_pool)
            .await
            .map_err(InternalServerError)?;

        // update cache
        if let Some(cache_device) = cached_user.devices.get_mut(&token.device) {
            cache_device.device_token = req.0.device_token;
        }

        Ok(())
    }
}

pub async fn do_login(
    state: &State,
    uid: i64,
    device: &str,
    device_token: Option<&str>,
) -> Result<LoginApiResponse> {
    let mut cache = state.cache.write().await;
    let cached_user = match cache.users.get_mut(&uid) {
        Some(cached_user) => cached_user,
        None => return Ok(LoginApiResponse::UserDoesNotExist),
    };

    if cached_user.status == UserStatus::Frozen {
        return Ok(LoginApiResponse::Frozen);
    }

    // update refresh token
    let key_config = state.key_config.read().await;
    let (refresh_token, token) = rc_token::create_token_pair(
        &key_config.server_key,
        CurrentUser {
            uid,
            device: device.to_string(),
            is_admin: cached_user.is_admin,
            is_guest: cached_user.is_guest,
        },
        state.config.system.refresh_token_expiry_seconds,
        state.config.system.token_expiry_seconds,
    )
    .map_err(InternalServerError)?;

    let mut tx = state.db_pool.begin().await.map_err(InternalServerError)?;

    sqlx::query(
        r#"
        insert into refresh_token (uid, device, token) values (?, ?, ?)
            on conflict (uid, device) do update set token = excluded.token
        "#,
    )
    .bind(uid)
    .bind(device)
    .bind(&refresh_token)
    .execute(&mut tx)
    .await
    .map_err(InternalServerError)?;

    // update device token
    sqlx::query(
        r#"
        insert into device (uid, device, device_token) values (?, ?, ?)
            on conflict (uid, device) do update set device_token = excluded.device_token
        "#,
    )
    .bind(uid)
    .bind(device)
    .bind(device_token)
    .execute(&mut tx)
    .await
    .map_err(InternalServerError)?;

    tx.commit().await.map_err(InternalServerError)?;

    cached_user
        .devices
        .entry(device.to_string())
        .and_modify(|device| {
            device.device_token = device_token.map(ToString::to_string);
        })
        .or_insert_with(|| CacheDevice {
            device_token: device_token.map(ToString::to_string),
            sender: None,
        });

    Ok(LoginApiResponse::Ok(Json(LoginResponse {
        server_id: key_config.server_id.clone(),
        token,
        refresh_token,
        expired_in: state.config.system.token_expiry_seconds,
        user: cached_user.api_user_info(uid),
    })))
}

async fn download_avatar(url: &str) -> anyhow::Result<Bytes> {
    Ok(reqwest::get(url)
        .and_then(|resp| async move { resp.error_for_status() })
        .and_then(|resp| resp.bytes())
        .await?)
}

async fn oidc_get_id_token(
    item: OAuth2State,
    code: String,
    state: String,
) -> anyhow::Result<CoreIdTokenClaims> {
    let code = AuthorizationCode::new(code);

    anyhow::ensure!(&state == item.csrf_token.secret(), "invalid csrf token");

    // Exchange the code with a token.
    let token_response = match item
        .client
        .exchange_code(code)
        .set_pkce_verifier(item.pkce_verifier)
        .request_async(openidconnect::reqwest::async_http_client)
        .await
    {
        Ok(token_response) => token_response,
        Err(err) => {
            tracing::debug!(error = %err, "failed to exchange the code with a token");
            return Err(err.into());
        }
    };

    let id_token_verifier: CoreIdTokenVerifier = item
        .client
        .id_token_verifier()
        .set_other_audience_verifier_fn(|aud| aud.as_str() == "solid")
        .set_allowed_algs(vec![
            CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            CoreJwsSigningAlgorithm::EcdsaP256Sha256,
        ]);
    let id_token = match token_response.extra_fields().id_token() {
        Some(id_token) => id_token,
        None => {
            tracing::debug!("id token does not exist");
            anyhow::bail!("expect id token");
        }
    };
    let id_token_claims: &CoreIdTokenClaims = match id_token.claims(&id_token_verifier, &item.nonce)
    {
        Ok(id_token_claims) => id_token_claims,
        Err(err) => {
            tracing::debug!(error = %err, "failed to verify claims");
            return Err(err.into());
        }
    };

    tracing::debug!("success get id token");
    Ok(id_token_claims.clone())
}

async fn get_credentials_openid(state: &State, uid: i64) -> anyhow::Result<Vec<String>> {
    let sql = "select issuer from openid_connect where uid = ?";
    Ok(sqlx::query_as::<_, (String,)>(sql)
        .bind(uid)
        .fetch_all(&state.db_pool)
        .await?
        .into_iter()
        .map(|(issuer,)| issuer)
        .collect())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use poem::http::StatusCode;
    use serde_json::json;

    use crate::test_harness::TestServer;

    async fn login(server: &TestServer) -> String {
        let resp = server
            .post("/api/token/login")
            .body_json(&json!({
                "credential": {
                    "type": "password",
                    "account": "admin@zimu.pub",
                    "password": "123456",
                },
                "device": "iphone",
                "device_token": "test",
            }))
            .send()
            .await;
        resp.assert_status_is_ok();
        resp.json()
            .await
            .value()
            .object()
            .get("token")
            .string()
            .to_string()
    }

    #[tokio::test]
    async fn test_login() {
        let server = TestServer::new().await;

        let resp = server
            .post("/api/token/login")
            .body_json(&json!({
                "credential": {
                    "type": "password",
                    "account": "admin@zimu.pub",
                    "password": "123456",
                },
                "device": "iphone",
                "device_token": "test",
            }))
            .send()
            .await;
        resp.assert_status_is_ok();
        let json = resp.json().await;
        let obj = json.value().object();
        assert_eq!(server.parse_token(obj.get("token").string()).await.uid, 1);
        assert_eq!(
            server
                .parse_token(obj.get("refresh_token").string())
                .await
                .uid,
            1
        );
    }

    #[tokio::test]
    async fn test_renew() {
        let server = TestServer::new().await;

        // login
        let resp = server
            .post("/api/token/login")
            .body_json(&json!({
                "credential": {
                    "type": "password",
                    "account": "admin@zimu.pub",
                    "password": "123456",
                },
                "device": "iphone",
                "device_token": "test",
            }))
            .send()
            .await;
        resp.assert_status_is_ok();
        let json = resp.json().await;
        let obj = json.value().object();
        let token = obj.get("token").string();
        let refresh_token = obj.get("refresh_token").string();

        // renew
        let resp = server
            .post("/api/token/renew")
            .body_json(&json!({
                "token": token,
                "refresh_token": refresh_token,
            }))
            .send()
            .await;
        resp.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_logout() {
        let server = TestServer::new().await;

        // login
        let token = login(&server).await;

        // logout
        let resp = server
            .get("/api/token/logout")
            .header("X-API-Key", &token)
            .send()
            .await;
        resp.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_renew_with_expired_token() {
        let server = TestServer::new_with_config(|cfg| {
            cfg.system.token_expiry_seconds = 3;
        })
        .await;

        // login
        let resp = server
            .post("/api/token/login")
            .body_json(&json!({
                "credential": {
                    "type": "password",
                    "account": "admin@zimu.pub",
                    "password": "123456",
                },
                "device": "iphone",
                "device_token": "test",
            }))
            .send()
            .await;
        resp.assert_status_is_ok();
        let json = resp.json().await;
        let obj = json.value().object();
        let token = obj.get("token").string();
        let refresh_token = obj.get("refresh_token").string();

        tokio::time::sleep(Duration::from_secs(5)).await;

        // use the old token
        let resp = server
            .get("/api/user/me")
            .header("X-API-Key", token)
            .send()
            .await;
        resp.assert_status(StatusCode::UNAUTHORIZED);

        // renew
        let resp = server
            .post("/api/token/renew")
            .body_json(&json!({
                "token": token,
                "refresh_token": refresh_token,
            }))
            .send()
            .await;
        resp.assert_status_is_ok();
        let json = resp.json().await;
        let obj = json.value().object();
        let new_token = obj.get("token").string();

        // use the new token
        let resp = server
            .get("/api/user/me")
            .header("X-API-Key", new_token)
            .send()
            .await;
        resp.assert_status_is_ok();
    }

    #[tokio::test]
    async fn test_update_device_token() {
        let server = TestServer::new().await;
        let admin_token = server.login_admin_with_device("web").await;

        let resp = server
            .put("/api/token/device_token")
            .header("X-API-Key", &admin_token)
            .body_json(&json!({
                "device_token": "abc"
            }))
            .send()
            .await;
        resp.assert_status_is_ok();

        assert_eq!(
            server
                .state()
                .cache
                .read()
                .await
                .users
                .get(&1)
                .unwrap()
                .devices
                .get("web")
                .unwrap()
                .device_token
                .as_deref(),
            Some("abc")
        );

        let device_token2 = sqlx::query_as::<_, (Option<String>,)>(
            "select device_token from device where uid = ? and device = ?",
        )
        .bind(1)
        .bind("web")
        .fetch_one(&server.state().db_pool)
        .await
        .map(|(t,)| t)
        .unwrap();
        assert_eq!(device_token2.as_deref(), Some("abc"));

        let resp = server
            .put("/api/token/device_token")
            .header("X-API-Key", &admin_token)
            .body_json(&json!({ "device_token": null }))
            .send()
            .await;
        resp.assert_status_is_ok();

        assert!(server
            .state()
            .cache
            .read()
            .await
            .users
            .get(&1)
            .unwrap()
            .devices
            .get("web")
            .unwrap()
            .device_token
            .is_none());

        let device_token2 = sqlx::query_as::<_, (Option<String>,)>(
            "select device_token from device where uid = ? and device = ?",
        )
        .bind(1)
        .bind("web")
        .fetch_one(&server.state().db_pool)
        .await
        .map(|(t,)| t)
        .unwrap();
        assert_eq!(device_token2, None);
    }

    #[tokio::test]
    async fn test_login_with_third_party() {
        let server = TestServer::new().await;

        let resp = server
            .post("/api/token/create_third_party_key")
            .body_json(&json!({
                "userid": "u1",
                "username": "usertest"
            }))
            .header(
                "X-SECRET",
                &server.state().key_config.read().await.third_party_secret,
            )
            .send()
            .await;
        resp.assert_status_is_ok();
        let key = resp.json().await.value().string().to_string();

        let resp = server
            .post("/api/token/login")
            .body_json(&json!({
                "credential": {
                    "type": "thirdparty",
                    "key": key,
                }
            }))
            .send()
            .await;
        resp.assert_status_is_ok();
        let json = resp.json().await;
        let obj = json.value().object();
        let user = obj.get("user").object();
        user.get("name").assert_string("usertest");
        let uid1 = user.get("uid").i64();

        let resp = server
            .post("/api/token/create_third_party_key")
            .body_json(&json!({
                "userid": "u1",
                "username": "usertest"
            }))
            .header(
                "X-SECRET",
                &server.state().key_config.read().await.third_party_secret,
            )
            .send()
            .await;
        resp.assert_status_is_ok();
        let key = resp.json().await.value().string().to_string();

        let resp = server
            .post("/api/token/login")
            .body_json(&json!({
                "credential": {
                    "type": "thirdparty",
                    "key": key,
                }
            }))
            .send()
            .await;
        let json = resp.json().await;
        let obj = json.value().object();
        let user = obj.get("user").object();
        let uid2 = user.get("uid").i64();

        assert_eq!(uid1, uid2);
    }
}
