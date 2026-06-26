use std::{path::Path, sync::Arc};

use anyhow::Result;
use itertools::Itertools;
use poem::{
    middleware::{Compression, Cors, TokioMetrics, Tracing},
    Endpoint, EndpointExt, Route,
};
use rc_msgdb::MsgDb;
use sqlx::migrate::{MigrateDatabase, Migrator};
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::{
    api,
    api::{get_merged_message, init_bot_cache, FrontendUrlConfig, LoginConfig, OrganizationConfig},
    config::KeyConfig,
    create_user::{CreateUser, CreateUserBy},
    state::{forward_chat_messages_to_webhook, BroadcastEvent, Cache},
    Config, SqlitePool, State,
};

pub static MIGRATOR: Migrator = sqlx::migrate!();

pub fn create_random_str(len: usize) -> String {
    String::from_utf8(
        (0..len)
            .map(|_| fastrand::alphanumeric() as u8)
            .collect_vec(),
    )
    .unwrap()
}

pub async fn create_state(config_path: &Path, config: Arc<Config>) -> Result<State> {
    // load key config
    let mut key_config = None;
    let key_config_path = config.system.data_dir.join("key.json");
    if key_config_path.exists() {
        if let Ok(data) = std::fs::read(&key_config_path) {
            if let Ok(cfg) = serde_json::from_slice::<KeyConfig>(&data) {
                key_config = Some(cfg);
            }
        }
    }

    std::fs::create_dir_all(&config.system.data_dir).expect("create data dir");

    if key_config.is_none() {
        let cfg = KeyConfig {
            server_id: create_random_str(32),
            server_key: create_random_str(32),
        };
        std::fs::write(&key_config_path, serde_json::to_vec(&cfg)?)?;
        key_config = Some(cfg);
    }
    let key_config = key_config.unwrap();

    std::fs::create_dir_all(config.system.tmp_dir()).expect("create tmp dir");
    std::fs::create_dir_all(config.system.db_dir()).expect("create db dir");
    std::fs::create_dir_all(config.system.msg_dir()).expect("create message dir");
    std::fs::create_dir_all(config.system.thumbnail_dir()).expect("create thumbnails dir");
    std::fs::create_dir_all(config.system.file_dir()).expect("create file dir");
    std::fs::create_dir_all(config.system.avatar_dir()).expect("create avatars dir");
    std::fs::create_dir_all(config.system.group_avatar_dir()).expect("create group avatars dir");

    // open sqlite db
    let dsn = format!("sqlite:{}", config.system.sqlite_filename().display());
    if !config.system.sqlite_filename().exists() {
        tracing::info!(dsn = dsn.as_str(), "create sqlite db.");
        sqlx::Sqlite::create_database(&dsn).await?;
    }

    tracing::info!(dsn = dsn.as_str(), "open sqlite db.");
    let db_pool = SqlitePool::connect(&dsn).await?;
    MIGRATOR.run(&db_pool).await?;

    // open message db
    tracing::info!(
        path = config.system.msg_dir().display().to_string().as_str(),
        "open message db."
    );
    let msg_db = MsgDb::open(config.system.msg_dir())?;

    let (groups, users) = futures_util::try_join!(
        State::load_groups_cache(&msg_db, &db_pool),
        State::load_users_cache(&db_pool),
    )?;

    let (msg_updated_tx, msg_updated_rx) = mpsc::unbounded_channel();
    let (bot_online_tx, bot_online_rx) = mpsc::unbounded_channel();

    // Initialize E2EE managers
    let device_keys_manager = Arc::new(crate::api::matrix::e2ee::DeviceKeysManager::new(
        db_pool.clone(),
    ));
    let room_encryption_manager = Arc::new(crate::api::matrix::e2ee::RoomEncryptionManager::new(
        db_pool.clone(),
    ));
    let olm_session_manager = Arc::new(crate::api::matrix::e2ee::OlmSessionManager::new(
        db_pool.clone(),
    ));
    let megolm_session_manager = Arc::new(crate::api::matrix::e2ee::MegolmSessionManager::new(
        db_pool.clone(),
    ));
    let server_olm_account_manager = Arc::new(
        crate::api::matrix::e2ee::ServerOlmAccountManager::new(db_pool.clone()),
    );

    let state = State {
        key_config: Arc::new(RwLock::new(key_config)),
        config: config.clone(),
        config_path: config_path.to_owned(),
        db_pool,
        msg_db: Arc::new(msg_db),
        cache: Arc::new(RwLock::new(Cache {
            dynamic_config: Default::default(),
            groups,
            users,
        })),
        event_sender: Arc::new(broadcast::channel(128).0),
        pending_oidc: Default::default(),
        msg_updated_channel: Arc::new(msg_updated_tx),
        bot_online_tx: Arc::new(bot_online_tx),
        device_keys_manager,
        room_encryption_manager,
        olm_session_manager,
        megolm_session_manager,
        server_olm_account_manager,
    };

    // load dynamic config
    state
        .initialize_dynamic_config::<OrganizationConfig>()
        .await?;
    state
        .initialize_dynamic_config::<FrontendUrlConfig>()
        .await?;
    state.initialize_dynamic_config::<LoginConfig>().await?;

    // create users
    for user in &config.users {
        let create_user = CreateUser::new(
            &user.name,
            CreateUserBy::Password {
                email: &user.email,
                password: &user.password,
            },
            false,
        )
        .gender(user.gender)
        .set_admin(user.is_admin)
        .language(&user.language);
        let _ = state.create_user(create_user).await;
    }

    // initialize bot cache for matrix sync
    init_bot_cache(&state).await;

    // Initialize server Olm accounts for bot users
    // This creates virtual server devices with Olm identity keys
    // so that bridges can discover them and share Megolm session keys
    {
        let server_key = state.key_config.read().await.server_key.clone();
        let matrix_domain = crate::api::matrix::auth::get_matrix_domain(&state);
        let cache = state.cache.read().await;
        for (uid, user) in &cache.users {
            if user.is_bot {
                if let Err(e) = state
                    .server_olm_account_manager
                    .ensure_account(
                        *uid,
                        &format!("@{}:{}", user.name, matrix_domain),
                        &matrix_domain,
                        &server_key,
                    )
                    .await
                {
                    tracing::error!(
                        "Failed to initialize server Olm account for uid={}: {}",
                        uid,
                        e
                    );
                }
            }
        }
    }

    tokio::spawn(process_msg_updated(state.clone(), msg_updated_rx));
    tokio::spawn(forward_chat_messages_to_webhook(state.clone()));
    tokio::spawn(process_bot_online_state(state.clone(), bot_online_rx));
    Ok(state)
}

/// Process bot online state updates and timeout
async fn process_bot_online_state(state: State, mut rx: mpsc::UnboundedReceiver<(i64, bool)>) {
    use crate::api::message::UserStateChangedMessage;
    use tokio::time::{Duration, Instant};

    // Track bot online state with timers: uid -> deadline_instant
    let bot_timers: Arc<parking_lot::Mutex<std::collections::HashMap<i64, Instant>>> =
        Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new()));

    const BOT_OFFLINE_TIMEOUT: Duration = Duration::from_secs(60);

    // Spawn a task to check timers periodically
    let state_clone = state.clone();
    let bot_timers_clone = bot_timers.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;

            let now = Instant::now();
            let mut to_offline = Vec::new();

            {
                let bot_timers = bot_timers_clone.lock();
                for (&uid, &deadline) in bot_timers.iter() {
                    if now >= deadline {
                        to_offline.push(uid);
                    }
                }
            }

            // Set bots to offline if their timers expired
            for uid in to_offline {
                let mut cache = state_clone.cache.write().await;
                if let Some(user) = cache.users.get_mut(&uid) {
                    if user.is_bot {
                        user.bot_online = false;
                        tracing::info!(uid = %uid, "Bot went offline due to timeout");
                        // Broadcast user state changed event
                        let _ = state_clone.event_sender.send(Arc::new(
                            crate::state::BroadcastEvent::UserStateChanged(
                                UserStateChangedMessage {
                                    uid,
                                    online: Some(false),
                                },
                            ),
                        ));
                    }
                }
                // Remove from timers
                bot_timers_clone.lock().remove(&uid);
            }
        }
    });

    // Process incoming messages to update bot online state
    while let Some((uid, should_go_online)) = rx.recv().await {
        let now = tokio::time::Instant::now();

        if should_go_online {
            // Set bot online and start/reset timer
            {
                let mut cache = state.cache.write().await;
                if let Some(user) = cache.users.get_mut(&uid) {
                    if user.is_bot {
                        user.bot_online = true;
                        tracing::info!(uid = %uid, "Bot went online");
                        // Broadcast user state changed event
                        let _ = state.event_sender.send(Arc::new(
                            crate::state::BroadcastEvent::UserStateChanged(
                                UserStateChangedMessage {
                                    uid,
                                    online: Some(true),
                                },
                            ),
                        ));
                    }
                }
            }
            // Set timer for 60 seconds from now
            bot_timers.lock().insert(uid, now + BOT_OFFLINE_TIMEOUT);
        } else {
            // Bot explicitly goes offline
            {
                let mut cache = state.cache.write().await;
                if let Some(user) = cache.users.get_mut(&uid) {
                    if user.is_bot {
                        user.bot_online = false;
                        tracing::info!(uid = %uid, "Bot went offline explicitly");
                        // Broadcast user state changed event
                        let _ = state.event_sender.send(Arc::new(
                            crate::state::BroadcastEvent::UserStateChanged(
                                UserStateChangedMessage {
                                    uid,
                                    online: Some(false),
                                },
                            ),
                        ));
                    }
                }
            }
            // Remove timer
            bot_timers.lock().remove(&uid);
        }
    }
}

async fn process_msg_updated(state: State, mut rx: mpsc::UnboundedReceiver<i64>) {
    while let Some(mid) = rx.recv().await {
        // process pinned messages
        if let Ok(merged_msg) = get_merged_message(&state.msg_db, mid) {
            let mut cache = state.cache.write().await;
            let Cache { groups, users, .. } = &mut *cache;

            for (gid, group) in groups {
                if let Some((idx, pinned_msg)) = group
                    .pinned_messages
                    .iter_mut()
                    .enumerate()
                    .find(|(_, pinned_msg)| pinned_msg.mid == mid)
                {
                    let targets = if group.ty.is_public() {
                        users.iter().map(|(uid, _)| *uid).collect()
                    } else {
                        group.members.clone()
                    };

                    match merged_msg {
                        Some(merged_msg) => {
                            // pinned message updated

                            // update cache
                            pinned_msg.content = merged_msg.content;

                            // broadcast
                            let _ = state.event_sender.send(Arc::new(
                                BroadcastEvent::PinnedMessageUpdated {
                                    targets,
                                    gid: *gid,
                                    mid,
                                    msg: Some(pinned_msg.clone()),
                                },
                            ));
                        }
                        None => {
                            // pinned message deleted

                            // update cache
                            group.pinned_messages.remove(idx);

                            // update database
                            if let Err(err) =
                                sqlx::query("delete from pinned_message where gid = ? and mid = ?")
                                    .bind(gid)
                                    .bind(mid)
                                    .execute(&state.db_pool)
                                    .await
                            {
                                tracing::error!(
                                    gid = gid,
                                    mid = mid,
                                    error = %err,
                                    "failed to delete pinned message"
                                );
                            }

                            // broadcast
                            let _ = state.event_sender.send(Arc::new(
                                BroadcastEvent::PinnedMessageUpdated {
                                    targets,
                                    gid: *gid,
                                    mid,
                                    msg: None,
                                },
                            ));
                        }
                    }
                    break;
                }
            }
        }
    }
}

pub async fn create_endpoint(state: State) -> impl Endpoint {
    let mut api_service = state.config.network.domain.iter().fold(
        api::create_api_service().server("http://localhost:3000/api"),
        |acc, domain| acc.server(format!("https://{}/api", domain)),
    );
    let frontend_url = state
        .get_dynamic_config_instance::<FrontendUrlConfig>()
        .await
        .and_then(|config| config.url.clone())
        .or_else(|| {
            if !state.config.network.frontend_url.is_empty() {
                Some(state.config.network.frontend_url.clone())
            } else {
                None
            }
        });
    if let Some(frontend_url) = frontend_url {
        api_service = api_service.server(if frontend_url.ends_with('/') {
            format!("{}api", frontend_url)
        } else {
            format!("{}/api", frontend_url)
        });
    };

    let metrics = TokioMetrics::new();

    // Create poem native routes for Matrix endpoints with special characters
    let matrix_routes = api::create_api_routes();

    let wwwroot_dir = state.config.system.wwwroot_dir();
    tracing::info!("wwwroot directory: {}", wwwroot_dir.display());

    let route = Route::new()
        .nest(
            "/",
            poem::endpoint::StaticFilesEndpoint::new(wwwroot_dir).index_file("index.html"),
        )
        .at("/health", poem::endpoint::make_sync(|_| ()))
        .at("/metrics", metrics.exporter())
        .nest("/_matrix", matrix_routes);

    // 根据配置决定是否启用 Swagger 文档端点
    let route = if state.config.network.enable_swagger {
        route
            .nest("/api/doc", api_service.swagger_ui())
            .nest("/api/doc2", api_service.rapidoc())
            .nest("/api/swagger", api_service.swagger_ui())
            .nest("/api/doc3", api_service.redoc())
            .nest("/api/spec", api_service.spec_endpoint())
            .nest("/api", api_service)
    } else {
        route.nest("/api", api_service)
    };

    route
        .with(Compression::new())
        .with(Tracing)
        .with(Cors::new().allow_credentials(true))
        .with(metrics)
        .inspect_err(|err: &sqlx::Error| {
            tracing::error!(error = %err, "sqlite error");
        })
        .data(state)
}

#[cfg(test)]
mod tests {
    use poem::{
        listener::{Acceptor, Listener, TcpListener},
        Server,
    };
    use reqwest::{Certificate, StatusCode};

    use super::*;
    use crate::{
        config::{NetworkConfig, SystemConfig, TlsConfig},
        self_signed::create_self_signed_config,
    };

    #[tokio::test]
    async fn test_tls_server() {
        let tempdir = tempfile::TempDir::new().unwrap();
        let config = Config {
            system: SystemConfig {
                data_dir: tempdir.path().to_path_buf(),
                wwwroot_dir: None,
                token_expiry_seconds: 60 * 60,
                refresh_token_expiry_seconds: 60 * 60,
                upload_avatar_limit: 1024 * 1024,
                upload_timeout_seconds: 300,
                file_expiry_days: 30 * 3,
                max_favorite_archives: 100,
                log_level: String::new(),
            },
            network: NetworkConfig {
                domain: Vec::new(),
                bind: "127.0.0.1:0".to_string(),
                tls: Some(TlsConfig::SelfSigned),
                frontend_url: "https://127.0.0.1:3000".to_string(),
                enable_swagger: false,
                matrix_domain: None,
            },
            users: vec![],
        };
        let state = create_state(tempdir.path(), Arc::new(config))
            .await
            .unwrap();
        let ep = create_endpoint(state).await;
        let acceptor = TcpListener::bind("127.0.0.1:0")
            .rustls(create_self_signed_config())
            .into_acceptor()
            .await
            .unwrap();
        let addr = acceptor.local_addr().remove(0);
        tokio::spawn(async move {
            Server::new_with_acceptor(acceptor).run(ep).await.unwrap();
        });
        let port = addr.as_socket_addr().unwrap().port();

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .add_root_certificate(Certificate::from_pem(include_bytes!("../cert/ca.crt")).unwrap())
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .build()
            .unwrap();
        let url = format!("https://localhost:{}/health", port);
        let resp = client.get(url).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
