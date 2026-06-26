use image::ImageFormat;
use poem::{error::InternalServerError, http::StatusCode, web::Data, Error, Result};
use poem_openapi::{
    payload::{Binary, Json, PlainText},
    types::Email,
    ApiRequest, Object, OpenApi,
};
use poem_openapi::param::Query;
use serde::{Deserialize, Serialize};

use crate::{
    api::{
        message::decode_messages,
        tags::ApiTags,
        token::Token,
        MessageDetail, MessageTarget, UserInfo,
    },
    config::Config,
    create_user::{CreateUser, CreateUserBy},
    state::{DynamicConfig, DynamicConfigEntry},
    State,
};

/// Server metrics
#[derive(Debug, Object)]
pub struct Metrics {
    user_count: usize,
    group_count: usize,
    online_user_count: usize,
    version: String,
}

/// Frontend url
#[derive(Debug, Object, Serialize, Deserialize, Default)]
pub struct FrontendUrlConfig {
    pub url: Option<String>,
}

impl DynamicConfig for FrontendUrlConfig {
    type Instance = Self;

    fn name() -> &'static str {
        "frontend-url"
    }

    fn create_instance(self, _config: &Config) -> Self::Instance {
        self
    }
}

/// Organization info
#[derive(Debug, Object, Serialize, Deserialize)]
pub struct OrganizationConfig {
    name: String,
    description: Option<String>,
}

/// System common config
#[derive(Debug, Object, Serialize)]
pub struct SystemCommonConfig {
    chat_layout_mode: String,
    contact_verification_enable: bool,
    ext_setting: Option<String>,
    max_file_expiry_mode: String,
    msg_smtp_notify_delay_seconds: i32,
    msg_smtp_notify_enable: bool,
    only_admin_can_create_group: bool,
    show_user_online_status: bool,
    webclient_auto_update: bool,
}

impl DynamicConfig for OrganizationConfig {
    type Instance = Self;

    fn name() -> &'static str {
        "organization"
    }

    fn create_instance(self, _config: &Config) -> Self::Instance {
        self
    }
}

impl Default for OrganizationConfig {
    fn default() -> Self {
        Self {
            name: "unknown".to_string(),
            description: None,
        }
    }
}

/// Update common config request (partial update)
#[derive(Debug, Object)]
struct UpdateCommonConfigRequest {
    /// Layout mode, e.g. "Left" or "SelfRight"
    chat_layout_mode: Option<String>,
    contact_verification_enable: Option<bool>,
    /// JSON string, e.g. "{\"enable_msg_url_preview\":true}"
    ext_setting: Option<String>,
    max_file_expiry_mode: Option<String>,
    msg_smtp_notify_delay_seconds: Option<i32>,
    msg_smtp_notify_enable: Option<bool>,
    only_admin_can_create_group: Option<bool>,
    show_user_online_status: Option<bool>,
    webclient_auto_update: Option<bool>,
}

#[derive(ApiRequest)]
enum UploadLogoRequest {
    #[oai(content_type = "image/png")]
    Image(Binary<Vec<u8>>),
}

#[derive(Object)]
struct CreateAdminRequest {
    email: Email,
    name: String,
    password: String,
    gender: i32,
}

/// App version info
#[derive(Debug, Clone, Object, Serialize, Deserialize)]
pub struct AppVersionInfo {
    pub version_code: i16,
    pub version_name: String,
    pub build_time: String,
    pub description: String,
    pub file_url: String,
}

/// File info for admin file list
#[derive(Debug, Clone, Object, Serialize)]
struct AdminFile {
    mid: i64,
    from_uid: i64,
    gid: i64,
    ext: String,
    content_type: String,
    content: String,
    thumbnail: String,
    properties: String,
    created_at: i64,
    expired: bool,
}

pub struct ApiAdminSystem;

#[OpenApi(prefix_path = "/admin/system", tag = "ApiTags::AdminSystem")]
impl ApiAdminSystem {
    /// Get the server version
    #[oai(path = "/version", method = "get")]
    async fn version(&self) -> PlainText<&'static str> {
        PlainText(env!("CARGO_PKG_VERSION"))
    }

    /// Check app update
    #[oai(path = "/check_update", method = "get")]
    async fn check_update(
        &self,
        state: Data<&State>,
        platform: Query<Option<String>>,
    ) -> Result<Json<Option<AppVersionInfo>>> {
        let path = state.config.system.data_dir.join("version.json");
        if !path.exists() {
            tracing::warn!("version.json not found: {}", path.display());
            return Ok(Json(None));
        }
        let bytes = std::fs::read(&path).unwrap_or_default();
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("version.json is not UTF-8 encoded: {}", e);
                return Ok(Json(None));
            }
        };
        let versions: std::collections::HashMap<String, AppVersionInfo> =
            serde_json::from_str(&content).unwrap_or_default();
        let key = match platform.0.as_deref() {
            Some(p) => p.to_lowercase(),
            None => return Ok(Json(None)),
        };
        Ok(Json(versions.get(&key).cloned()))
    }

    /// Get change log
    #[oai(path = "/change_log", method = "get")]
    async fn change_log(&self, state: Data<&State>) -> PlainText<String> {
        let path = state.config.system.data_dir.join("change_log.txt");
        if !path.exists() {
            tracing::warn!("change_log.txt not found: {}", path.display());
            return PlainText(String::new());
        }
        let bytes = std::fs::read(&path).unwrap_or_default();
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("change_log.txt is not UTF-8 encoded: {}", e);
                String::new()
            }
        };
        let result = content
            .lines()
            .take(500)
            .collect::<Vec<&str>>()
            .join("\n");
        PlainText(result)
    }

    /// Create administrator user
    #[oai(path = "/create_admin", method = "post")]
    async fn create_admin_user(
        &self,
        state: Data<&State>,
        mut req: Json<CreateAdminRequest>,
    ) -> Result<Json<UserInfo>> {
        if !state.cache.read().await.users.is_empty() {
            return Err(poem::Error::from_status(StatusCode::FORBIDDEN));
        }

        req.email.0 = req.email.0.to_lowercase();
        let (uid, user) = match state
            .create_user(
                CreateUser::new(
                    &req.name,
                    CreateUserBy::Password {
                        email: &req.email,
                        password: &req.password,
                    },
                    true,
                )
                .gender(req.gender),
            )
            .await
        {
            Ok(res) => res,
            Err(_) => return Err(poem::Error::from_status(StatusCode::FORBIDDEN)),
        };

        // update user.is_admin
        Ok(Json(user.api_user_info(uid)))
    }

    /// Returns `true` means that the server has been initialized
    #[oai(path = "/initialized", method = "get")]
    async fn initialized(&self, state: Data<&State>) -> Json<bool> {
        let cache = state.cache.read().await;
        Json(!cache.users.is_empty())
    }

    /// Get the system metrics
    #[oai(path = "/metrics", method = "get")]
    async fn get_metrics(&self, state: Data<&State>, token: Token) -> Result<Json<Metrics>> {
        if !token.is_admin {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        let cache = state.cache.read().await;
        Ok(Json(Metrics {
            user_count: cache.users.iter().filter(|user| !user.1.is_guest).count(),
            group_count: cache.groups.len(),
            online_user_count: cache
                .users
                .values()
                .filter(|user| !user.is_guest && user.is_online())
                .count(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }))
    }

    /// Get the organization info
    #[oai(path = "/organization", method = "get")]
    async fn get_organization(&self, state: Data<&State>) -> Result<Json<OrganizationConfig>> {
        let entry = state.load_dynamic_config::<OrganizationConfig>().await?;
        Ok(Json(entry.config))
    }

    /// Set the organization info
    #[oai(path = "/organization", method = "post")]
    async fn set_organization(
        &self,
        state: Data<&State>,
        token: Token,
        req: Json<OrganizationConfig>,
    ) -> Result<()> {
        if !token.is_admin {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }
        state
            .set_dynamic_config(DynamicConfigEntry {
                enabled: true,
                config: req.0,
            })
            .await?;
        Ok(())
    }

    /// Update the organization info (PUT)
    #[oai(path = "/organization", method = "put")]
    async fn update_organization(
        &self,
        state: Data<&State>,
        token: Token,
        req: Json<OrganizationConfig>,
    ) -> Result<()> {
        if !token.is_admin {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }
        state
            .set_dynamic_config(DynamicConfigEntry {
                enabled: true,
                config: req.0,
            })
            .await?;
        Ok(())
    }

    /// Upload the organization logo
    #[oai(path = "/organization/logo", method = "post")]
    async fn upload_organization_logo(
        &self,
        state: Data<&State>,
        token: Token,
        logo: UploadLogoRequest,
    ) -> Result<()> {
        if !token.is_admin {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        let UploadLogoRequest::Image(data) = logo;
        let logo = image::load_from_memory(&data).map_err(InternalServerError)?;
        let logo = logo.thumbnail(240, 240);
        let path = state.config.system.data_dir.join("organization.png");
        logo.save_with_format(path, ImageFormat::Png)
            .map_err(InternalServerError)?;
        Ok(())
    }

    /// Get the frontend url
    #[oai(path = "/frontend_url", method = "get")]
    async fn get_frontend_url(
        &self,
        state: Data<&State>,
        token: Token,
    ) -> Result<PlainText<String>> {
        if !token.is_admin {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        Ok(PlainText(
            state
                .get_dynamic_config_instance::<FrontendUrlConfig>()
                .await
                .and_then(|config| config.url.clone())
                .unwrap_or_default(),
        ))
    }

    /// Update the frontend url
    #[oai(path = "/update_frontend_url", method = "post")]
    async fn update_frontend_url(
        &self,
        state: Data<&State>,
        token: Token,
        frontend_url: PlainText<String>,
    ) -> Result<()> {
        if !token.is_admin {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        let frontend_url = frontend_url.0.trim_end_matches('/');
        let re = regex::Regex::new(r#"^https?://[\w\-\.]+(:\d+)?$"#).unwrap();
        if !re.is_match(frontend_url) {
            return Err(Error::from_string(
                "Bad url format!",
                StatusCode::BAD_REQUEST,
            ));
        }

        state
            .set_dynamic_config(DynamicConfigEntry {
                enabled: true,
                config: FrontendUrlConfig {
                    url: Some(frontend_url.to_string()),
                },
            })
            .await?;
        Ok(())
    }

    /// Update the system common config (partial update, merges into existing)
    #[oai(path = "/common", method = "put")]
    async fn update_common_config(
        &self,
        state: Data<&State>,
        token: Token,
        req: Json<UpdateCommonConfigRequest>,
    ) -> Result<()> {
        if !token.is_admin {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        let req = req.0;

        // 1. Load current config value from DB
        let current_value: Option<String> =
            sqlx::query_scalar("select value from config where name = 'common'")
                .fetch_optional(&state.db_pool)
                .await
                .map_err(InternalServerError)?;

        let mut config: serde_json::Map<String, serde_json::Value> = current_value
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        // 2. Apply partial updates – only touch fields present in the request
        if let Some(v) = req.chat_layout_mode {
            config.insert("chat_layout_mode".to_string(), serde_json::Value::String(v));
        }
        if let Some(v) = req.contact_verification_enable {
            config.insert(
                "contact_verification_enable".to_string(),
                serde_json::Value::Bool(v),
            );
        }
        if let Some(v) = req.max_file_expiry_mode {
            config.insert(
                "max_file_expiry_mode".to_string(),
                serde_json::Value::String(v),
            );
        }
        if let Some(v) = req.msg_smtp_notify_delay_seconds {
            config.insert(
                "msg_smtp_notify_delay_seconds".to_string(),
                serde_json::json!(v),
            );
        }
        if let Some(v) = req.msg_smtp_notify_enable {
            config.insert(
                "msg_smtp_notify_enable".to_string(),
                serde_json::Value::Bool(v),
            );
        }
        if let Some(v) = req.only_admin_can_create_group {
            config.insert(
                "only_admin_can_create_group".to_string(),
                serde_json::Value::Bool(v),
            );
        }
        if let Some(v) = req.show_user_online_status {
            config.insert(
                "show_user_online_status".to_string(),
                serde_json::Value::Bool(v),
            );
        }
        if let Some(v) = req.webclient_auto_update {
            config.insert(
                "webclient_auto_update".to_string(),
                serde_json::Value::Bool(v),
            );
        }

        // ext_setting comes in as a JSON string; parse it and merge key-by-key
        if let Some(ext_str) = req.ext_setting {
            match serde_json::from_str::<serde_json::Value>(&ext_str) {
                Ok(serde_json::Value::Object(new_ext)) => {
                    let ext_entry = config
                        .entry("ext_setting".to_string())
                        .or_insert_with(|| serde_json::Value::Object(Default::default()));
                    if let serde_json::Value::Object(existing_ext) = ext_entry {
                        existing_ext.extend(new_ext);
                    }
                }
                _ => return Err(Error::from_status(StatusCode::BAD_REQUEST)),
            }
        }

        // 3. Write merged config back to DB
        let value_str = serde_json::to_string(&config).map_err(InternalServerError)?;
        sqlx::query(
            r#"insert into config (name, enabled, value) values ('common', true, ?)
               on conflict (name) do update set value = excluded.value"#,
        )
        .bind(value_str)
        .execute(&state.db_pool)
        .await
        .map_err(InternalServerError)?;

        Ok(())
    }

    /// Get the system common config
    #[oai(path = "/common", method = "get")]
    async fn get_common_config(&self, state: Data<&State>) -> Result<Json<SystemCommonConfig>> {
        let current_value: Option<String> =
            sqlx::query_scalar("select value from config where name = 'common'")
                .fetch_optional(&state.db_pool)
                .await
                .map_err(InternalServerError)?;

        let cfg: serde_json::Map<String, serde_json::Value> = current_value
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        Ok(Json(SystemCommonConfig {
            chat_layout_mode: cfg
                .get("chat_layout_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("Left")
                .to_string(),
            contact_verification_enable: cfg
                .get("contact_verification_enable")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            ext_setting: cfg
                .get("ext_setting")
                .and_then(|v| serde_json::to_string(v).ok()),
            max_file_expiry_mode: cfg
                .get("max_file_expiry_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("Off")
                .to_string(),
            msg_smtp_notify_delay_seconds: cfg
                .get("msg_smtp_notify_delay_seconds")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or(0),
            msg_smtp_notify_enable: cfg
                .get("msg_smtp_notify_enable")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            only_admin_can_create_group: cfg
                .get("only_admin_can_create_group")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            show_user_online_status: cfg
                .get("show_user_online_status")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            webclient_auto_update: cfg
                .get("webclient_auto_update")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }))
    }

    /// Get all file messages (content_type = "vachat/file")
    #[oai(path = "/files", method = "get")]
    async fn get_files(
        &self,
        state: Data<&State>,
        token: Token,
        #[oai(default)] page: Query<u64>,
        #[oai(default = "default_page_size")] page_size: Query<u64>,
    ) -> Result<Json<Vec<AdminFile>>> {
        if !token.is_admin {
            return Err(Error::from_status(StatusCode::FORBIDDEN));
        }

        let page = page.0.max(1);
        let page_size = page_size.0.min(1000);

        let max_mid = state.msg_db.get_max_msg_id().unwrap_or(None).unwrap_or(0);
        let mut files: Vec<AdminFile> = Vec::new();
        let mut cursor = max_mid;
        let batch_size: usize = 500;

        while cursor > 0 {
            let msgs = state
                .msg_db
                .messages()
                .fetch_messages_before_rev(cursor, batch_size)
                .map_err(InternalServerError)?;
            if msgs.is_empty() {
                break;
            }

            if let Some((last_mid, _)) = msgs.last() {
                cursor = last_mid - 1;
            } else {
                break;
            }

            for chat_msg in decode_messages(msgs) {
                if let MessageDetail::Normal(normal) = &chat_msg.payload.detail {
                    if normal.content.content_type == "vachat/file" {
                        let props = normal.content.properties.as_ref();
                        let file_content_type = props
                            .and_then(|p| p.get("content_type"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("application/octet-stream");
                        let name = props
                            .and_then(|p| p.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let ext = std::path::Path::new(name)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_string();

                        let is_image = file_content_type.starts_with("image/");
                        let thumbnail = if is_image {
                            normal.content.content.clone()
                        } else {
                            String::new()
                        };

                        let expired = normal
                            .expires_in
                            .map(|expires_in| {
                                (chrono::Utc::now().timestamp_millis()
                                    - chat_msg.payload.created_at.timestamp_millis())
                                    / 1000
                                    > expires_in
                            })
                            .unwrap_or(false);

                        let gid = match &chat_msg.payload.target {
                            MessageTarget::Group(g) => g.gid,
                            _ => 0,
                        };

                        let properties_str = props
                            .map(|p| serde_json::to_string(p).unwrap_or_default())
                            .unwrap_or_default();

                        files.push(AdminFile {
                            mid: chat_msg.mid,
                            from_uid: chat_msg.payload.from_uid,
                            gid,
                            ext,
                            content_type: file_content_type.to_string(),
                            content: normal.content.content.clone(),
                            thumbnail,
                            properties: properties_str,
                            created_at: chat_msg.payload.created_at.timestamp_millis(),
                            expired,
                        });
                    }
                }
            }
        }

        let start = ((page - 1) as usize) * (page_size as usize);
        let end = (start + page_size as usize).min(files.len());
        let paged = if start < files.len() {
            files[start..end].to_vec()
        } else {
            Vec::new()
        };

        Ok(Json(paged))
    }
}

fn default_page_size() -> u64 {
    100
}

#[test]
fn test_frontend_url() {
    let re = regex::Regex::new(r#"^https?://[\w\-\.]+(:\d+)?$"#).unwrap();
    assert!(re.is_match("http://1.2.3.4:4000"));
    assert!(re.is_match("http://domain.com"));
    assert!(re.is_match("http://domain.com:3000"));
    assert!(re.is_match("https://domain.com:3000"));
    assert!(re.is_match("http://127.0.0.1"));
    assert!(re.is_match("http://127.0.0.1:3000"));
    assert!(re.is_match("https://127.0.0.1:3000"));
    assert!(!re.is_match("ftp://127.0.0.1:3000"));
}

#[test]
fn test_replace_config() {
    let a = r#"frontend_url = "http://a.com/""#;
    let re = regex::Regex::new(r#"frontend_url\s*=\s*".*?""#).unwrap();
    let b = re.replace(a, format!(r#"frontend_url = "{}""#, "http://b.com/"));
    assert_eq!(b, r#"frontend_url = "http://b.com/""#);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::test_harness::TestServer;

    #[tokio::test]
    async fn set_organization() {
        let server = TestServer::new().await;
        let admin_token = server.login_admin().await;

        let resp = server.get("/api/admin/system/organization").send().await;
        resp.assert_status_is_ok();
        resp.assert_json(&json!({
            "name": "unknown",
            "description": null,
        }))
        .await;

        server
            .post("/api/admin/system/organization")
            .header("X-API-Key", &admin_token)
            .body_json(&json!({
                "name": "abc",
                "description": "def"
            }))
            .send()
            .await
            .assert_status_is_ok();

        let resp = server.get("/api/admin/system/organization").send().await;
        resp.assert_status_is_ok();
        resp.assert_json(&json!({
            "name": "abc",
            "description": "def",
        }))
        .await;
    }

    #[tokio::test]
    async fn test_update_frontend_url() {
        let server = TestServer::new().await;
        let admin_token = server.login_admin().await;

        let resp = server
            .post("/api/admin/system/update_frontend_url")
            .header("X-API-Key", &admin_token)
            .content_type("text/plain")
            .body("http://1.2.3.4:4000")
            .send()
            .await;
        resp.assert_status_is_ok();
    }
}
