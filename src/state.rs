use std::{
    any::Any,
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::{NaiveDate, Utc};
use futures_util::{StreamExt, TryStreamExt};
use itertools::Itertools;
use num_enum::{FromPrimitive, IntoPrimitive};
use openidconnect::{core::CoreClient, CsrfToken, Nonce, PkceCodeVerifier};
use poem::{
    error::{BadRequest, InternalServerError},
    http::StatusCode,
};
use poem_openapi::{types::ToJSON, Enum};
use rc_msgdb::MsgDb;
use reqwest::Client;
use serde::{de::DeserializeOwned, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use walkdir::WalkDir;

use crate::{
    api::{
        get_merged_message, ChatMessage, DateTime, Group, GroupAnnouncement, GroupChangedMessage,
        KickFromGroupReason, KickReason, LangId, MessageDetail, PinnedMessage, UpdateAction, User,
        UserDevice, UserInfo, UserSettingsChangedMessage, UserStateChangedMessage, UserUpdateLog,
    },
    config::KeyConfig,
    Config,
};

#[derive(Debug, Copy, Clone)]
pub enum GroupType {
    Public,
    Private { owner: i64 },
}

impl GroupType {
    pub fn is_public(&self) -> bool {
        matches!(self, GroupType::Public)
    }

    pub fn owner(&self) -> Option<i64> {
        match self {
            GroupType::Public => None,
            GroupType::Private { owner } => Some(*owner),
        }
    }
}

pub struct CacheGroup {
    pub ty: GroupType,
    pub name: String,
    pub description: String,
    pub members: BTreeSet<i64>,
    #[allow(dead_code)]
    pub created_at: DateTime,
    #[allow(dead_code)]
    pub updated_at: DateTime,
    pub avatar_updated_at: DateTime,
    pub pinned_messages: Vec<PinnedMessage>,
    pub announcement: Option<GroupAnnouncement>,
}

impl CacheGroup {
    pub fn contains_user(&self, uid: i64) -> bool {
        self.ty.is_public() || self.members.contains(&uid)
    }

    pub fn description_opt(&self) -> Option<String> {
        if !self.description.is_empty() {
            Some(self.description.clone())
        } else {
            None
        }
    }

    pub fn api_group(&self, gid: i64) -> Group {
        Group {
            gid,
            owner: self.ty.owner(),
            name: self.name.clone(),
            description: self.description_opt(),
            members: self.members.iter().copied().collect(),
            is_public: self.ty.is_public(),
            avatar_updated_at: self.avatar_updated_at,
            pinned_messages: self.pinned_messages.clone(),
            announcement: self.announcement.clone(),
        }
    }
}

#[derive(Debug)]
pub struct CacheDevice {
    pub device_token: Option<String>,
    pub sender: Option<mpsc::UnboundedSender<UserEvent>>,
}

#[derive(Debug, Copy, Clone, FromPrimitive, IntoPrimitive, Enum, Eq, PartialEq)]
#[oai(rename_all = "lowercase")]
#[repr(i8)]
pub enum UserStatus {
    Normal = 0,
    #[num_enum(default)]
    Frozen = -1,
}

#[derive(Debug)]
pub struct BotKey {
    pub name: String,
    pub key: String,
    pub created_at: DateTime,
    pub last_used: Option<DateTime>,
}

#[derive(Debug, Clone)]
pub struct CacheContactInfo {
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub status: i32,
}

#[derive(Debug)]
pub struct CacheUser {
    pub email: Option<String>,
    pub name: String,
    pub password: Option<String>,
    pub gender: i32,
    pub is_admin: bool,
    pub language: LangId,
    pub create_by: String,
    pub devices: HashMap<String, CacheDevice>,
    pub mute_user: HashMap<i64, Option<DateTime>>,
    pub mute_group: HashMap<i64, Option<DateTime>>,
    pub burn_after_reading_user: HashMap<i64, i64>,
    pub burn_after_reading_group: HashMap<i64, i64>,
    pub read_index_user: HashMap<i64, i64>,
    pub read_index_group: HashMap<i64, i64>,
    pub pinned_chat_user: HashSet<i64>,
    pub pinned_chat_group: HashSet<i64>,
    pub contacts: HashMap<i64, CacheContactInfo>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub avatar_updated_at: DateTime,
    pub status: UserStatus,
    pub is_guest: bool,
    pub webhook_url: Option<String>,
    pub is_bot: bool,
    pub bot_keys: HashMap<i64, BotKey>,
    /// Whether the bot is currently connected via Matrix protocol
    pub bot_online: bool,
}

impl CacheUser {
    pub fn is_online(&self) -> bool {
        // Bot is online if connected via Matrix protocol
        if self.is_bot && self.bot_online {
            return true;
        }
        // Regular user is online if any device has an active sender
        self.devices.values().any(|device| device.sender.is_some())
    }

    pub fn api_user_info(&self, uid: i64) -> UserInfo {
        UserInfo {
            uid,
            email: self.email.clone(),
            name: self.name.clone(),
            gender: self.gender,
            language: self.language.clone(),
            is_admin: self.is_admin,
            is_bot: self.is_bot,
            avatar_updated_at: self.avatar_updated_at,
            create_by: self.create_by.clone(),
            msg_smtp_notify_enable: false,
            birthday: None,
        }
    }

    pub fn api_user(&self, uid: i64) -> User {
        User {
            uid,
            email: self.email.clone(),
            password: Default::default(),
            name: self.name.clone(),
            gender: self.gender,
            is_admin: self.is_admin,
            language: self.language.clone(),
            create_by: self.create_by.clone(),
            in_online: self.is_online(),
            online_devices: self
                .devices
                .iter()
                .map(|(device, cached_device)| UserDevice {
                    device: device.clone(),
                    device_token: cached_device.device_token.clone(),
                    is_online: cached_device.sender.is_some(),
                })
                .collect(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            avatar_updated_at: self.avatar_updated_at,
            status: self.status,
            webhook_url: self.webhook_url.clone(),
            is_bot: self.is_bot,
        }
    }

    pub fn is_user_muted(&self, uid: i64) -> bool {
        let now = DateTime::now();
        match self.mute_user.get(&uid) {
            Some(Some(expired_at)) if expired_at.0 < now.0 => true,
            Some(None) => true,
            _ => false,
        }
    }

    pub fn is_group_muted(&self, gid: i64) -> bool {
        let now = DateTime::now();
        match self.mute_group.get(&gid) {
            Some(Some(expired_at)) if expired_at.0 < now.0 => true,
            Some(None) => true,
            _ => false,
        }
    }

    pub fn burn_after_reading_to_user_expires_in(&self, uid: i64) -> Option<i64> {
        self.burn_after_reading_user.get(&uid).copied()
    }

    pub fn burn_after_reading_to_group_expires_in(&self, gid: i64) -> Option<i64> {
        self.burn_after_reading_group.get(&gid).copied()
    }
}

#[derive(Debug, Clone)]
pub enum BroadcastEvent {
    /// Chat message
    Chat {
        targets: BTreeSet<i64>,
        message: ChatMessage,
    },
    /// Users update log
    UserLog(UserUpdateLog),
    /// Other users joined group
    UserJoinedGroup {
        targets: BTreeSet<i64>,
        gid: i64,
        uid: Vec<i64>,
    },
    /// Other users leaved group
    UserLeavedGroup {
        targets: BTreeSet<i64>,
        gid: i64,
        uid: Vec<i64>,
    },
    /// Join the group
    JoinedGroup {
        targets: BTreeSet<i64>,
        group: Group,
    },
    /// Kick from group
    KickFromGroup {
        targets: BTreeSet<i64>,
        gid: i64,
        reason: KickFromGroupReason,
    },
    /// User state changed
    UserStateChanged(UserStateChangedMessage),
    /// User settings changed
    UserSettingsChanged {
        uid: i64,
        message: UserSettingsChangedMessage,
    },
    /// Group changed
    GroupChanged {
        targets: BTreeSet<i64>,
        msg: GroupChangedMessage,
    },
    /// Pinned message updated
    PinnedMessageUpdated {
        targets: BTreeSet<i64>,
        gid: i64,
        mid: i64,
        msg: Option<PinnedMessage>,
    },
    /// Group announcement changed
    GroupAnnouncementChanged {
        gid: i64,
        announcement: Option<GroupAnnouncement>,
    },
}

#[derive(Debug, Clone)]
pub enum UserEvent {
    /// Kick by other device
    Kick { reason: KickReason },
}

#[derive(Default)]
pub struct Cache {
    pub dynamic_config: HashMap<&'static str, Box<dyn Any + Send + Sync>>,
    pub groups: BTreeMap<i64, CacheGroup>,
    pub users: BTreeMap<i64, CacheUser>,
}

impl Cache {
    fn assign_username_by_name<'a>(&self, name: &'a str) -> Cow<'a, str> {
        if self.check_name_conflict(name) {
            return Cow::Borrowed(name);
        }

        loop {
            let new_name = format!("{}{}", name, fastrand::u32(1111..9999));
            if self.check_name_conflict(&new_name) {
                break Cow::Owned(new_name);
            }
        }
    }

    fn assign_username_by_email<'a>(&self, email: &'a str) -> Cow<'a, str> {
        match email.find('@') {
            Some(idx) if idx > 0 => self.assign_username_by_name(&email[..idx]),
            _ => self.assign_username_by_name(email),
        }
    }

    pub fn assign_username<'a>(
        &self,
        name: Option<&'a str>,
        email: Option<&'a str>,
    ) -> Cow<'a, str> {
        if let Some(name) = name {
            self.assign_username_by_name(name)
        } else if let Some(email) = email {
            self.assign_username_by_email(email)
        } else {
            loop {
                let new_name = format!("User{}", fastrand::u32(111111..999999));
                if self.check_name_conflict(&new_name) {
                    break Cow::Owned(new_name);
                }
            }
        }
    }

    pub fn check_name_conflict(&self, name: &str) -> bool {
        !self
            .users
            .values()
            .any(|user| user.name.eq_ignore_ascii_case(name))
    }

    pub fn check_email_conflict(&self, email: &str) -> bool {
        !self.users.values().any(|user| {
            if let Some(user_email) = &user.email {
                user_email.eq_ignore_ascii_case(email)
            } else {
                false
            }
        })
    }
}

pub struct OAuth2State {
    pub client: CoreClient,
    pub issuer: String,
    pub pkce_verifier: PkceCodeVerifier,
    pub csrf_token: CsrfToken,
    pub nonce: Nonce,
}

pub trait DynamicConfig: Serialize + DeserializeOwned + Default {
    type Instance: Send + Sync + 'static;

    fn name() -> &'static str;

    fn create_instance(self, config: &Config) -> Self::Instance;
}

pub struct DynamicConfigEntry<T: DynamicConfig> {
    pub enabled: bool,
    pub config: T,
}

#[derive(Clone)]
pub struct State {
    pub key_config: Arc<RwLock<KeyConfig>>,
    pub config: Arc<Config>,
    #[allow(dead_code)]
    pub config_path: PathBuf,
    pub db_pool: SqlitePool,
    pub msg_db: Arc<MsgDb>,
    pub cache: Arc<RwLock<Cache>>,
    pub event_sender: Arc<broadcast::Sender<Arc<BroadcastEvent>>>,
    pub pending_oidc: Arc<Mutex<HashMap<String, OAuth2State>>>,
    pub msg_updated_channel: Arc<mpsc::UnboundedSender<i64>>,
    #[allow(dead_code)]
    pub invalid_device_tokens: Arc<parking_lot::Mutex<HashSet<String>>>,
    /// Channel to notify bot to go offline (uid, should_go_online)
    pub bot_online_tx: Arc<mpsc::UnboundedSender<(i64, bool)>>,
    /// E2EE device keys manager
    pub device_keys_manager: Arc<crate::api::matrix::e2ee::DeviceKeysManager>,
    /// E2EE room encryption manager
    pub room_encryption_manager: Arc<crate::api::matrix::e2ee::RoomEncryptionManager>,
    /// E2EE Olm session manager
    #[allow(dead_code)]
    pub olm_session_manager: Arc<crate::api::matrix::e2ee::OlmSessionManager>,
    /// E2EE Megolm session manager for room/group encryption
    pub megolm_session_manager: Arc<crate::api::matrix::e2ee::MegolmSessionManager>,
    /// E2EE Server Olm Account manager for bot virtual devices
    pub server_olm_account_manager: Arc<crate::api::matrix::e2ee::ServerOlmAccountManager>,
}

impl State {
    pub async fn load_users_cache(db: &SqlitePool) -> sqlx::Result<BTreeMap<i64, CacheUser>> {
        let mut users = BTreeMap::new();
        let sql = "select uid, email, name, password, gender, is_admin, language, create_by, created_at, updated_at, avatar_updated_at, status, is_guest, webhook_url, is_bot from user";
        let mut stream = sqlx::query_as::<
            _,
            (
                i64,
                Option<String>,
                String,
                Option<String>,
                i32,
                bool,
                LangId,
                String,
                DateTime,
                DateTime,
                DateTime,
                i8,
                bool,
                Option<String>,
                bool,
            ),
        >(sql)
        .fetch(db);
        while let Some(res) = stream.next().await {
            let (
                uid,
                email,
                name,
                password,
                gender,
                is_admin,
                language,
                create_by,
                created_at,
                updated_at,
                avatar_updated_at,
                status,
                is_guest,
                webhook_url,
                is_bot,
            ) = res?;

            let devices = sqlx::query_as::<_, (String, Option<String>)>(
                "select device, device_token from device where uid = ?",
            )
            .bind(uid)
            .fetch_all(db)
            .await?;
            let devices = devices
                .into_iter()
                .map(|(device, device_token)| {
                    (
                        device,
                        CacheDevice {
                            device_token,
                            sender: None,
                        },
                    )
                })
                .collect();

            let sql = "select mute_uid, mute_gid, expired_at from mute where uid = ?";
            let mute = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<DateTime>)>(sql)
                .bind(uid)
                .fetch_all(db)
                .await?;
            let mut mute_user = HashMap::new();
            let mut mute_group = HashMap::new();
            for (uid, gid, expired_at) in mute {
                match (uid, gid) {
                    (Some(uid), None) => {
                        mute_user.insert(uid, expired_at);
                    }
                    (None, Some(gid)) => {
                        mute_group.insert(gid, expired_at);
                    }
                    _ => {}
                }
            }

            let sql =
                "select target_uid, target_gid, expires_in from burn_after_reading where uid = ?";
            let burn_after_reading = sqlx::query_as::<_, (Option<i64>, Option<i64>, i64)>(sql)
                .bind(uid)
                .fetch_all(db)
                .await?;
            let mut burn_after_reading_user = HashMap::new();
            let mut burn_after_reading_group = HashMap::new();
            for (uid, gid, expires_in) in burn_after_reading {
                match (uid, gid) {
                    (Some(uid), None) => {
                        burn_after_reading_user.insert(uid, expires_in);
                    }
                    (None, Some(gid)) => {
                        burn_after_reading_group.insert(gid, expires_in);
                    }
                    _ => {}
                }
            }

            let sql = "select target_uid, target_gid, mid from read_index where uid = ?";
            let read_index = sqlx::query_as::<_, (Option<i64>, Option<i64>, i64)>(sql)
                .bind(uid)
                .fetch_all(db)
                .await?;
            let mut read_index_user = HashMap::new();
            let mut read_index_group = HashMap::new();
            for (uid, gid, mid) in read_index {
                match (uid, gid) {
                    (Some(uid), None) => {
                        read_index_user.insert(uid, mid);
                    }
                    (None, Some(gid)) => {
                        read_index_group.insert(gid, mid);
                    }
                    _ => {}
                }
            }

            let sql = "select target_uid, target_gid from pinned_chat where uid = ?";
            let pinned_chat = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(sql)
                .bind(uid)
                .fetch_all(db)
                .await?;
            let mut pinned_chat_user = HashSet::new();
            let mut pinned_chat_group = HashSet::new();
            for (uid, gid) in pinned_chat {
                match (uid, gid) {
                    (Some(uid), None) => {
                        pinned_chat_user.insert(uid);
                    }
                    (None, Some(gid)) => {
                        pinned_chat_group.insert(gid);
                    }
                    _ => {}
                }
            }

            let sql = "select id, name, key, created_at, last_used from `bot_key` where uid = ?";
            let bot_keys =
                sqlx::query_as::<_, (i64, String, String, DateTime, Option<DateTime>)>(sql)
                    .bind(uid)
                    .fetch(db)
                    .map_ok(|(id, name, key, created_at, last_used)| {
                        (
                            id,
                            BotKey {
                                name,
                                key,
                                created_at,
                                last_used,
                            },
                        )
                    })
                    .try_collect()
                    .await?;

            let sql =
                "select target_uid, status, created_at, updated_at from contacts where uid = ?";
            let contacts = sqlx::query_as::<_, (i64, i32, DateTime, DateTime)>(sql)
                .bind(uid)
                .fetch_all(db)
                .await?
                .into_iter()
                .map(|(target_uid, status, created_at, updated_at)| {
                    (
                        target_uid,
                        CacheContactInfo {
                            created_at,
                            updated_at,
                            status,
                        },
                    )
                })
                .collect();

            users.insert(
                uid,
                CacheUser {
                    email,
                    name,
                    password,
                    gender,
                    is_admin,
                    language,
                    create_by,
                    devices,
                    mute_user,
                    mute_group,
                    burn_after_reading_user,
                    burn_after_reading_group,
                    read_index_user,
                    read_index_group,
                    pinned_chat_user,
                    pinned_chat_group,
                    contacts,
                    created_at,
                    updated_at,
                    avatar_updated_at,
                    status: status.into(),
                    is_guest,
                    webhook_url,
                    is_bot,
                    bot_keys,
                    bot_online: false,
                },
            );
        }

        Ok(users)
    }

    pub async fn load_groups_cache(
        msg_db: &MsgDb,
        db: &SqlitePool,
    ) -> sqlx::Result<BTreeMap<i64, CacheGroup>> {
        let mut groups = BTreeMap::new();

        let sql =
            "select gid, name, description, owner, is_public, created_at, updated_at, avatar_updated_at from `group`";
        let mut stream = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                Option<i64>,
                bool,
                DateTime,
                DateTime,
                DateTime,
            ),
        >(sql)
        .fetch(db);
        while let Some(res) = stream.next().await {
            // load pinned messages
            let sql = "select mid, created_by, created_at from pinned_message where gid = ?";
            let ids = sqlx::query_as::<_, (i64, i64, DateTime)>(sql)
                .fetch_all(db)
                .await?;
            let mut pinned_messages = Vec::new();

            for (mid, created_by, created_at) in ids {
                if let Some(merged_msg) = get_merged_message(msg_db, mid).ok().flatten() {
                    pinned_messages.push(PinnedMessage {
                        mid,
                        created_by,
                        created_at,
                        content: merged_msg.content,
                    });
                }
            }

            pinned_messages.sort_by(|a, b| a.created_at.cmp(&b.created_at));

            let (
                gid,
                name,
                description,
                owner,
                is_public,
                created_at,
                updated_at,
                avatar_updated_at,
            ) = res?;
            groups.insert(
                gid,
                CacheGroup {
                    ty: if is_public {
                        GroupType::Public
                    } else {
                        GroupType::Private {
                            owner: owner.unwrap(),
                        }
                    },
                    name,
                    description,
                    members: Default::default(),
                    created_at,
                    updated_at,
                    avatar_updated_at,
                    pinned_messages,
                    announcement: None,
                },
            );
        }

        let mut stream =
            sqlx::query_as::<_, (i64, i64)>("select gid, uid from group_user").fetch(db);
        while let Some(res) = stream.next().await {
            let (gid, uid) = res?;
            if let Some(conv) = groups.get_mut(&gid) {
                conv.members.insert(uid);
            }
        }

        // load announcements
        let mut stream = sqlx::query_as::<_, (i64, String, i64, DateTime, DateTime)>(
            "select gid, content, created_by, created_at, updated_at from announcement",
        )
        .fetch(db);
        while let Some(res) = stream.next().await {
            let (gid, content, created_by, created_at, updated_at) = res?;
            if let Some(group) = groups.get_mut(&gid) {
                group.announcement = Some(GroupAnnouncement {
                    gid,
                    content,
                    created_by,
                    created_at,
                    updated_at,
                });
            }
        }

        Ok(groups)
    }

    pub async fn clean_mute(&self) {
        let now = DateTime::now();

        // clean in sqlite
        if let Err(err) =
            sqlx::query("delete from mute where expired_at notnull and expired_at < ?")
                .bind(now)
                .execute(&self.db_pool)
                .await
        {
            tracing::error!(error = %err, "failed to query expired mute items");
        }

        // clean in cache
        let mut cache = self.cache.write().await;
        let mut uid_list = Vec::new();
        let mut gid_list = Vec::new();

        for user in cache.users.values_mut() {
            uid_list.clear();
            gid_list.clear();

            for (uid, expired_at) in user.mute_user.iter() {
                if matches!(expired_at, Some(expired_at) if expired_at < &now) {
                    uid_list.push(*uid);
                }
            }

            for (gid, expired_at) in user.mute_group.iter() {
                if matches!(expired_at, Some(expired_at) if expired_at < &now) {
                    gid_list.push(*gid);
                }
            }

            for uid in &uid_list {
                user.mute_user.remove(uid);
            }
            for gid in &gid_list {
                user.mute_group.remove(gid);
            }
        }
    }

    /// Clean expired Olm and Megolm sessions
    /// Sessions older than 90 days are removed
    pub async fn clean_sessions(&self) {
        let retention_days = 90;
        let cutoff: crate::api::DateTime =
            (chrono::Utc::now() - chrono::Duration::days(retention_days)).into();

        // Clean expired Olm outbound sessions
        if let Err(err) =
            sqlx::query("DELETE FROM matrix_olm_outbound_session WHERE last_used_at < ?")
                .bind(cutoff)
                .execute(&self.db_pool)
                .await
        {
            tracing::error!(error = %err, "failed to clean expired Olm outbound sessions");
        }

        // Clean expired Olm inbound sessions
        if let Err(err) =
            sqlx::query("DELETE FROM matrix_olm_inbound_session WHERE last_used_at < ?")
                .bind(cutoff)
                .execute(&self.db_pool)
                .await
        {
            tracing::error!(error = %err, "failed to clean expired Olm inbound sessions");
        }

        // Clean expired Megolm inbound sessions
        if let Err(err) =
            sqlx::query("DELETE FROM matrix_megolm_inbound_session WHERE last_used_at < ?")
                .bind(cutoff)
                .execute(&self.db_pool)
                .await
        {
            tracing::error!(error = %err, "failed to clean expired Megolm inbound sessions");
        }
    }

    pub async fn sync_bot_key_last_used(&self) {
        async fn internal_sync_bot_key_last_used(state: &State) -> anyhow::Result<()> {
            let cache = state.cache.read().await;
            let mut tx = state.db_pool.begin().await?;

            for user in cache.users.values() {
                for (id, bot_key) in &user.bot_keys {
                    sqlx::query("update `bot_key` set last_used = ? where id = ?")
                        .bind(bot_key.last_used)
                        .bind(id)
                        .execute(&mut tx)
                        .await?;
                }
            }

            tx.commit().await?;
            Ok(())
        }

        if let Err(err) = internal_sync_bot_key_last_used(self).await {
            tracing::error!(error = %err, "failed to write last used of bot key");
        }
    }

    pub fn clean_temp_files(&self) {
        let now = SystemTime::now();
        let timeout_dur = Duration::from_secs(self.config.system.upload_timeout_seconds as u64);

        if let Ok(file_list) = self.config.system.tmp_dir().read_dir() {
            for entry in file_list.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("data") {
                    let mut remove = false;

                    if let Ok(modified) = path.metadata().and_then(|md| md.modified()) {
                        remove = now > modified + timeout_dur;
                    }

                    if remove {
                        let _ = std::fs::remove_file(&path);
                        let _ = std::fs::remove_file(path.with_extension("meta"));
                    }
                }
            }
        }
    }

    pub fn clean_files(&self) {
        let now = Utc::now().date_naive();
        clean_file_dir(
            now,
            &self.config.system.thumbnail_dir(),
            self.config.system.file_expiry_days,
        );
        clean_file_dir(
            now,
            &self.config.system.file_dir(),
            self.config.system.file_expiry_days,
        );
        clean_file_dir(
            now,
            &self.config.system.archive_msg_dir(),
            self.config.system.file_expiry_days,
        );
    }

    pub fn save_avatar(&self, uid: i64, data: &[u8]) -> poem::Result<()> {
        let image = image::load_from_memory(data).map_err(BadRequest)?;
        let avatar = image.thumbnail(256, 256);
        let path = self.config.system.avatar_dir().join(format!("{}.png", uid));
        avatar
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(InternalServerError)?;
        Ok(())
    }

    pub fn save_group_avatar(&self, gid: i64, data: &[u8]) -> poem::Result<()> {
        let image = image::load_from_memory(data).map_err(BadRequest)?;
        let avatar = image.thumbnail(256, 256);
        let path = self
            .config
            .system
            .group_avatar_dir()
            .join(format!("{}.png", gid));
        avatar
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(InternalServerError)?;
        Ok(())
    }

    pub async fn load_dynamic_config<T: DynamicConfig>(
        &self,
    ) -> anyhow::Result<DynamicConfigEntry<T>> {
        self.load_dynamic_config_with(|| DynamicConfigEntry {
            enabled: false,
            config: T::default(),
        })
        .await
    }

    pub async fn load_dynamic_config_with<T, F>(
        &self,
        f: F,
    ) -> anyhow::Result<DynamicConfigEntry<T>>
    where
        T: DynamicConfig,
        F: FnOnce() -> DynamicConfigEntry<T>,
    {
        let sql = "select enabled, value from config where name = ?";
        match sqlx::query_as::<_, (bool, String)>(sql)
            .bind(T::name())
            .fetch_optional(&self.db_pool)
            .await?
        {
            Some((enabled, value)) => Ok(DynamicConfigEntry {
                enabled,
                config: serde_json::from_str(&value)?,
            }),
            None => Ok(f()),
        }
    }

    pub async fn set_dynamic_config<T: DynamicConfig>(
        &self,
        entry: DynamicConfigEntry<T>,
    ) -> anyhow::Result<()> {
        let sql = r#"
        insert into config (name, enabled, value) values (?, ?, ?)
            on conflict (name) do update set enabled = excluded.enabled, value = excluded.value
        "#;
        sqlx::query(sql)
            .bind(T::name())
            .bind(entry.enabled)
            .bind(serde_json::to_string(&entry.config)?)
            .execute(&self.db_pool)
            .await?;

        if entry.enabled {
            let instance = entry.config.create_instance(&self.config);
            self.cache
                .write()
                .await
                .dynamic_config
                .insert(T::name(), Box::new(Arc::new(instance)));
        } else {
            self.cache.write().await.dynamic_config.remove(T::name());
        }

        Ok(())
    }

    pub async fn initialize_dynamic_config<T>(&self) -> anyhow::Result<()>
    where
        T: DynamicConfig,
    {
        self.initialize_dynamic_config_with(|| DynamicConfigEntry {
            enabled: false,
            config: T::default(),
        })
        .await
    }

    pub async fn initialize_dynamic_config_with<T, F>(&self, f: F) -> anyhow::Result<()>
    where
        T: DynamicConfig,
        F: FnOnce() -> DynamicConfigEntry<T>,
    {
        let entry = self.load_dynamic_config_with::<T, _>(f).await?;
        if entry.enabled {
            let instance = entry.config.create_instance(&self.config);
            self.cache
                .write()
                .await
                .dynamic_config
                .insert(T::name(), Box::new(Arc::new(instance)));
        }
        Ok(())
    }

    pub async fn get_dynamic_config_instance<T: DynamicConfig>(&self) -> Option<Arc<T::Instance>> {
        self.cache
            .read()
            .await
            .dynamic_config
            .get(T::name())
            .and_then(|instance| {
                instance
                    .downcast_ref::<Arc<T::Instance>>()
                    .map(Clone::clone)
            })
    }

    pub async fn clean_guest(&self) {
        if let Ok(users) = sqlx::query_as::<_, (i64,)>(
            "select uid from user where is_guest = true and datetime('now', '-7 days') >= created_at",
        )
        .fetch_all::<_>(&self.db_pool)
        .await
        {
            for (uid,) in users {
                let _ = self.delete_user(uid).await;
            }
        }
    }

    pub async fn delete_user(&self, uid: i64) -> poem::Result<()> {
        let mut cache = self.cache.write().await;
        let is_guest = match cache.users.get(&uid) {
            Some(user) => user.is_guest,
            None => return Err(poem::Error::from(StatusCode::NOT_FOUND)),
        };

        // begin transaction
        let mut tx = self.db_pool.begin().await.map_err(InternalServerError)?;

        // delete from user table
        sqlx::query("delete from user where uid = ?")
            .bind(uid)
            .execute(&mut tx)
            .await
            .map_err(InternalServerError)?;

        let log_id = if !is_guest {
            // insert into user_log table
            let log_id = sqlx::query("insert into user_log (uid, action) values (?, ?)")
                .bind(uid)
                .bind(UpdateAction::Delete)
                .execute(&mut tx)
                .await
                .map_err(InternalServerError)?
                .last_insert_rowid();
            Some(log_id)
        } else {
            None
        };

        // commit transaction
        tx.commit().await.map_err(InternalServerError)?;

        // update cache
        if let Some(cached_user) = cache.users.remove(&uid) {
            // close all subscriptions
            for device in cached_user.devices.into_values() {
                if let Some(sender) = device.sender {
                    let _ = sender.send(UserEvent::Kick {
                        reason: KickReason::DeleteUser,
                    });
                }
            }
        }

        let mut removed_groups_id = Vec::new();
        let mut removed_groups = Vec::new();
        let mut exit_from_private_group = Vec::new();
        let mut exit_from_public_group = Vec::new();

        for (gid, group) in cache.groups.iter_mut() {
            match group.ty {
                GroupType::Public => {
                    exit_from_public_group.push(*gid);
                }
                GroupType::Private { owner } if owner == uid => {
                    removed_groups_id.push(*gid);
                }
                GroupType::Private { .. } => {
                    group.members.remove(&uid);
                    exit_from_private_group.push((*gid, group.members.clone()));
                }
            }
        }

        for gid in &removed_groups_id {
            removed_groups.extend(cache.groups.remove(gid).map(|group| (*gid, group)));
        }
        for user in cache.users.values_mut() {
            user.read_index_user.remove(&uid);
            user.pinned_chat_user.remove(&uid);
        }
        for user in cache.users.values_mut() {
            for gid in &removed_groups_id {
                user.read_index_group.remove(gid);
                user.pinned_chat_group.remove(gid);
            }
        }

        // broadcast event
        if let Some(log_id) = log_id {
            let _ = self
                .event_sender
                .send(Arc::new(BroadcastEvent::UserLog(UserUpdateLog {
                    log_id,
                    action: UpdateAction::Delete,
                    uid,
                    email: None,
                    name: None,
                    gender: None,
                    language: None,
                    is_admin: None,
                    is_bot: None,
                    avatar_updated_at: None,
                })));

            for (gid, group) in removed_groups {
                let _ = self
                    .event_sender
                    .send(Arc::new(BroadcastEvent::KickFromGroup {
                        targets: group.members.iter().copied().collect(),
                        gid,
                        reason: KickFromGroupReason::GroupDeleted,
                    }));
            }

            for (gid, members) in exit_from_private_group {
                let _ = self
                    .event_sender
                    .send(Arc::new(BroadcastEvent::UserLeavedGroup {
                        targets: members,
                        gid,
                        uid: vec![uid],
                    }));
            }

            for gid in exit_from_public_group {
                let _ = self
                    .event_sender
                    .send(Arc::new(BroadcastEvent::UserLeavedGroup {
                        targets: cache.users.keys().copied().collect(),
                        gid,
                        uid: vec![uid],
                    }));
            }
        }

        Ok(())
    }
}

fn clean_file_dir(now: NaiveDate, path: &Path, expiry_days: i64) {
    let mut remove_dirs = Vec::new();
    let mut iter = WalkDir::new(path).into_iter();

    while let Some(Ok(entry)) = iter.next() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        if let Ok(p) = entry_path.strip_prefix(path) {
            if let Some(date) = p
                .to_str()
                .map(|name| name.split(std::path::MAIN_SEPARATOR).collect_vec())
                .filter(|s| s.len() == 3)
                .and_then(|s| {
                    let year = s[0].parse::<i32>().ok();
                    let month = s[1].parse::<u32>().ok();
                    let day = s[2].parse::<u32>().ok();
                    match (year, month, day) {
                        (Some(year), Some(month), Some(day)) => Some((year, month, day)),
                        _ => None,
                    }
                })
                .and_then(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d))
            {
                if (now - date).num_days() > expiry_days {
                    remove_dirs.push(entry_path.to_path_buf());
                }
            }
        }
    }

    for p in remove_dirs {
        let _ = std::fs::remove_dir_all(p);
    }
}

pub(crate) async fn forward_chat_messages_to_webhook(state: State) {
    let client = Client::new();
    let mut rx = state.event_sender.subscribe();

    while let Ok(event) = rx.recv().await {
        if let BroadcastEvent::Chat { targets, message } = &*event {
            if !matches!(
                &message.payload.detail,
                MessageDetail::Normal(_) | MessageDetail::Reply(_)
            ) {
                continue;
            }

            let webhook_urls = state
                .cache
                .read()
                .await
                .users
                .iter()
                .filter_map(|(uid, user)| {
                    if !targets.contains(uid) {
                        None
                    } else {
                        user.webhook_url.as_ref().cloned()
                    }
                })
                .collect::<Vec<_>>();
            let msg_json = message.to_json_string();

            for webhook_url in webhook_urls {
                let client = client.clone();
                let msg_json = msg_json.clone();

                tokio::spawn(async move {
                    for _ in 0..3 {
                        if client
                            .post(&webhook_url)
                            .header("content-type", "application/json")
                            .body(msg_json.clone())
                            .send()
                            .await
                            .and_then(|resp| resp.error_for_status())
                            .is_ok()
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use chrono::NaiveDate;
    use itertools::Itertools;
    use serde_json::json;

    use crate::{state::clean_file_dir, test_harness::TestServer, State};

    #[tokio::test]
    async fn test_clean_mute() {
        let server = TestServer::new().await;
        let admin_token = server.login_admin().await;
        let uid1 = server.create_user(&admin_token, "user1@zimu.pub").await;

        let resp = server
            .post("/api/group")
            .header("X-API-Key", &admin_token)
            .body_json(&json!({
                "name": "test",
                "members": [uid1]
            }))
            .send()
            .await;
        resp.assert_status_is_ok();
        let json = resp.json().await;
        let gid = json.value().object().get("gid").i64();

        // mute uid1 and token1
        let resp = server
            .post("/api/user/mute")
            .header("X-API-Key", &admin_token)
            .body_json(&json!({
                "add_users": [{
                    "uid": uid1,
                    "expired_in": 6,
                }],
                "add_groups": [{
                    "gid": gid,
                    "expired_in": 3,
                }]
            }))
            .send()
            .await;
        resp.assert_status_is_ok();

        async fn check(state: &State, mut items: Vec<(i64, Vec<i64>, Vec<i64>)>) {
            // check in cache
            let cache = state.cache.read().await;

            for (uid, users, groups) in &mut items {
                let mut exists_users = cache
                    .users
                    .get(uid)
                    .unwrap()
                    .mute_user
                    .keys()
                    .copied()
                    .collect_vec();
                let mut exists_groups = cache
                    .users
                    .get(uid)
                    .unwrap()
                    .mute_group
                    .keys()
                    .copied()
                    .collect_vec();

                users.sort_unstable();
                groups.sort_unstable();
                exists_users.sort_unstable();
                exists_groups.sort_unstable();

                assert_eq!(users, &exists_users);
                assert_eq!(groups, &exists_groups);
            }

            for (uid, users, groups) in &items {
                let mute = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
                    "select mute_uid, mute_gid from mute where uid = ?",
                )
                .bind(uid)
                .fetch_all(&state.db_pool)
                .await
                .unwrap();

                let mut exists_users = Vec::new();
                let mut exists_groups = Vec::new();

                for (uid, gid) in mute {
                    match (uid, gid) {
                        (Some(uid), None) => {
                            exists_users.push(uid);
                        }
                        (None, Some(gid)) => {
                            exists_groups.push(gid);
                        }
                        _ => {}
                    }
                }

                exists_users.sort_unstable();
                exists_groups.sort_unstable();
                assert_eq!(users, &exists_users);
                assert_eq!(groups, &exists_groups);
            }
        }

        server.state().clean_mute().await;
        check(server.state(), vec![(1, vec![uid1], vec![gid])]).await;

        tokio::time::sleep(Duration::from_secs(4)).await;
        server.state().clean_mute().await;
        check(server.state(), vec![(1, vec![uid1], vec![])]).await;

        tokio::time::sleep(Duration::from_secs(3)).await;
        server.state().clean_mute().await;
        check(server.state(), vec![]).await;
    }

    #[test]
    fn test_clear_file_dir() {
        fn create_dirs(path: &Path, dirs: &[i32]) {
            for d in dirs.iter().copied() {
                let year = d / 10000;
                let month = d / 100 % 100;
                let day = d % 100;

                let dpath = path
                    .join(format!("{}", year))
                    .join(format!("{}", month))
                    .join(format!("{}", day));
                let _ = std::fs::create_dir_all(dpath);
            }
        }

        fn check_dirs(path: &Path, dirs: &[(i32, bool)]) {
            for (d, exists) in dirs.iter().copied() {
                let year = d / 10000;
                let month = d / 100 % 100;
                let day = d % 100;

                let dpath = path
                    .join(format!("{}", year))
                    .join(format!("{}", month))
                    .join(format!("{}", day));
                assert_eq!(
                    dpath.exists(),
                    exists,
                    "{}-{}-{} = {}",
                    year,
                    month,
                    day,
                    exists
                );
            }
        }

        let path = tempfile::tempdir().unwrap();
        let dirs = vec![
            20220101, 20220102, 20220103, 20220104, 20220105, 20220106, 20220107, 20220108,
        ];
        create_dirs(path.path(), &dirs);

        clean_file_dir(
            NaiveDate::from_ymd_opt(2022, 1, 10).unwrap(),
            path.path(),
            7,
        );
        check_dirs(
            path.path(),
            &[
                (20220101, false),
                (20220102, false),
                (20220103, true),
                (20220104, true),
                (20220105, true),
                (20220106, true),
                (20220107, true),
                (20220108, true),
            ],
        );

        clean_file_dir(
            NaiveDate::from_ymd_opt(2022, 1, 12).unwrap(),
            path.path(),
            7,
        );

        check_dirs(
            path.path(),
            &[
                (20220103, false),
                (20220104, false),
                (20220105, true),
                (20220106, true),
                (20220107, true),
                (20220108, true),
            ],
        );
    }
}
