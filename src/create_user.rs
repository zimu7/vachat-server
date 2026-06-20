use std::sync::Arc;

use poem::error::InternalServerError;
use tokio::sync::{RwLockMappedWriteGuard, RwLockWriteGuard};

use crate::{
    api::{DateTime, LangId, UpdateAction, UserUpdateLog},
    password::hash_password,
    state::{BroadcastEvent, CacheUser, UserStatus},
    State,
};

#[derive(Debug)]
pub enum CreateUserBy<'a> {
    Guest,
    Password {
        email: &'a str,
        password: &'a str,
    },
    OpenIdConnect {
        issuer: &'a str,
        subject: &'a str,
        email: Option<&'a str>,
    },
    ThirdParty {
        thirdparty_uid: &'a str,
        #[allow(dead_code)]
        username: &'a str,
    },
}

impl<'a> CreateUserBy<'a> {
    fn type_name(&self) -> &'static str {
        match self {
            CreateUserBy::Guest => "guest",
            CreateUserBy::Password { .. } => "password",
            CreateUserBy::OpenIdConnect { .. } => "oidc",
            CreateUserBy::ThirdParty { .. } => "thirdparty",
        }
    }

    fn email(&self) -> Option<&'a str> {
        match self {
            CreateUserBy::Guest => None,
            CreateUserBy::Password { email, .. } => Some(*email),
            CreateUserBy::OpenIdConnect { email, .. } => *email,
            CreateUserBy::ThirdParty { .. } => None,
        }
    }

    fn password(&self) -> Option<&'a str> {
        match self {
            CreateUserBy::Guest => None,
            CreateUserBy::Password { password, .. } => Some(*password),
            CreateUserBy::OpenIdConnect { .. } => None,
            CreateUserBy::ThirdParty { .. } => None,
        }
    }
}

#[derive(Debug)]
pub struct CreateUser<'a> {
    name: &'a str,
    gender: i32,
    is_admin: bool,
    language: Option<&'a LangId>,
    avatar: Option<&'a [u8]>,
    create_by: CreateUserBy<'a>,
    webhook_url: Option<&'a str>,
    is_bot: bool,
}

impl<'a> CreateUser<'a> {
    pub fn new(name: &'a str, create_by: CreateUserBy<'a>, is_admin: bool) -> Self {
        Self {
            name,
            gender: 0,
            is_admin,
            language: None,
            avatar: None,
            create_by,
            webhook_url: None,
            is_bot: false,
        }
    }

    pub fn gender(self, gender: i32) -> Self {
        Self { gender, ..self }
    }

    pub fn set_admin(self, is_admin: bool) -> Self {
        Self { is_admin, ..self }
    }

    pub fn set_bot(self, is_bot: bool) -> Self {
        Self { is_bot, ..self }
    }

    pub fn language(self, language: &'a LangId) -> Self {
        Self {
            language: Some(language),
            ..self
        }
    }

    pub fn avatar(self, avatar: &'a [u8]) -> Self {
        Self {
            avatar: Some(avatar),
            ..self
        }
    }

    pub fn webhook_url(self, webhook_url: &'a str) -> Self {
        Self {
            webhook_url: Some(webhook_url),
            ..self
        }
    }
}

#[derive(Debug)]
pub enum CreateUserError {
    NameConflict,
    EmailConflict,
    PoemError(poem::Error),
}

impl From<poem::Error> for CreateUserError {
    fn from(err: poem::Error) -> Self {
        CreateUserError::PoemError(err)
    }
}

impl State {
    pub async fn create_user(
        &self,
        create_user: CreateUser<'_>,
    ) -> Result<(i64, RwLockMappedWriteGuard<'_, CacheUser>), CreateUserError> {
        let email = create_user.create_by.email();
        let raw_password = create_user.create_by.password();
        let language = create_user.language.cloned().unwrap_or_default();
        let mut cache = self.cache.write().await;
        let is_guest = matches!(&create_user.create_by, CreateUserBy::Guest);
        if !cache.check_name_conflict(create_user.name) {
            return Err(CreateUserError::NameConflict);
        }
        if let Some(email) = email {
            if !cache.check_email_conflict(email) {
                return Err(CreateUserError::EmailConflict);
            }
        }

        let now = DateTime::now();

        // Get server_key for password hashing
        let server_key = self.key_config.read().await.server_key.clone();

        // Hash password if present
        let hashed_password = raw_password.map(|p| hash_password(p, &server_key));

        // update sqlite
        let mut tx = self.db_pool.begin().await.map_err(InternalServerError)?;

        // insert into user table
        let avatar_updated_at = if create_user.avatar.is_some() {
            now
        } else {
            DateTime::zero()
        };
        let sql = "insert into user (name, password, email, gender, language, is_admin, create_by, avatar_updated_at, status, created_at, updated_at, is_guest, webhook_url, is_bot) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
        let uid = sqlx::query(sql)
            .bind(create_user.name)
            .bind(&hashed_password)
            .bind(email)
            .bind(create_user.gender)
            .bind(&language)
            .bind(create_user.is_admin)
            .bind(create_user.create_by.type_name())
            .bind(avatar_updated_at)
            .bind(i8::from(UserStatus::Normal))
            .bind(now)
            .bind(now)
            .bind(is_guest)
            .bind(create_user.webhook_url)
            .bind(create_user.is_bot)
            .execute(&mut tx)
            .await
            .map_err(InternalServerError)?
            .last_insert_rowid();

        if let Some(avatar) = create_user.avatar {
            let _ = self.save_avatar(uid, avatar);
        }

        match &create_user.create_by {
            CreateUserBy::OpenIdConnect {
                issuer, subject, ..
            } => {
                // insert into openid_connect
                let sql = "insert into openid_connect (issuer, subject, uid) values (?, ?, ?)";
                sqlx::query(sql)
                    .bind(issuer)
                    .bind(subject)
                    .bind(uid)
                    .execute(&mut tx)
                    .await
                    .map_err(InternalServerError)?;
            }
            CreateUserBy::ThirdParty { thirdparty_uid, .. } => {
                let sql = "insert into third_party_users (userid, uid) values (?, ?)";
                sqlx::query(sql)
                    .bind(thirdparty_uid)
                    .bind(uid)
                    .execute(&mut tx)
                    .await
                    .map_err(InternalServerError)?;
            }
            _ => {}
        }

        let log_id = if !is_guest {
            // insert into user_log table
            let sql = "insert into user_log (uid, action, email, name, gender, language, avatar_updated_at, is_admin, is_bot) values (?, ?, ?, ?, ?, ?, ?, ?, ?)";
            let log_id = sqlx::query(sql)
                .bind(uid)
                .bind(UpdateAction::Create)
                .bind(email)
                .bind(create_user.name)
                .bind(create_user.gender)
                .bind(&language)
                .bind(avatar_updated_at)
                .bind(create_user.is_admin)
                .bind(create_user.is_bot)
                .execute(&mut tx)
                .await
                .map_err(InternalServerError)?
                .last_insert_rowid();
            Some(log_id)
        } else {
            None
        };

        tx.commit().await.map_err(InternalServerError)?;

        // update cache
        cache.users.insert(
            uid,
            CacheUser {
                email: email.map(ToString::to_string),
                name: create_user.name.to_string(),
                password: hashed_password,
                gender: create_user.gender,
                is_admin: create_user.is_admin,
                language: language.clone(),
                create_by: create_user.create_by.type_name().to_string(),
                created_at: now,
                updated_at: now,
                avatar_updated_at,
                devices: Default::default(),
                mute_user: Default::default(),
                mute_group: Default::default(),
                burn_after_reading_user: Default::default(),
                burn_after_reading_group: Default::default(),
                read_index_user: Default::default(),
                read_index_group: Default::default(),
                pinned_chat_user: Default::default(),
                pinned_chat_group: Default::default(),
                contacts: Default::default(),
                status: UserStatus::Normal,
                is_guest,
                webhook_url: create_user.webhook_url.map(ToString::to_string),
                is_bot: create_user.is_bot,
                bot_keys: Default::default(),
                bot_online: false,
            },
        );

        if let Some(log_id) = log_id {
            // broadcast event
            let _ = self
                .event_sender
                .send(Arc::new(BroadcastEvent::UserLog(UserUpdateLog {
                    log_id,
                    action: UpdateAction::Create,
                    uid,
                    email: email.map(ToString::to_string),
                    name: Some(create_user.name.to_string()),
                    gender: create_user.gender.into(),
                    language: Some(language.clone()),
                    is_admin: Some(create_user.is_admin),
                    is_bot: Some(create_user.is_bot),
                    avatar_updated_at: Some(avatar_updated_at),
                })));

            for (gid, group) in cache.groups.iter() {
                if group.ty.is_public() {
                    let _ = self
                        .event_sender
                        .send(Arc::new(BroadcastEvent::UserJoinedGroup {
                            targets: cache.users.keys().copied().collect(),
                            gid: *gid,
                            uid: vec![uid],
                        }));
                }
            }
        }

        Ok((
            uid,
            RwLockWriteGuard::map(cache, |cache| cache.users.get_mut(&uid).unwrap()),
        ))
    }
}
