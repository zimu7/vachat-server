#![allow(clippy::large_enum_variant)]
#![allow(clippy::uninlined_format_args)]

mod api;
mod api_key;
mod config;
mod create_user;
mod middleware;
mod password;
mod self_signed;
mod server;
mod state;
#[cfg(test)]
mod test_harness;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use clap::Parser;
use poem::{
    listener::{Listener, TcpListener},
    EndpointExt, RouteScheme, Server,
};
use serde::Deserialize;
use sqlx::SqlitePool;
use tokio::runtime::Runtime;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

use crate::{
    config::{Config, TlsConfig},
    state::State,
};

#[derive(Debug, Default, Deserialize)]
struct EnvironmentVars {
    data_dir: Option<PathBuf>,
    wwwroot_dir: Option<PathBuf>,
}

impl EnvironmentVars {
    fn merge_to_config(self, mut config: Config) -> Config {
        if let Some(data_dir) = self.data_dir {
            config.system.data_dir = data_dir;
        }
        if let Some(wwwroot_dir) = self.wwwroot_dir {
            config.system.wwwroot_dir = Some(wwwroot_dir);
        }

        config
    }
}

#[derive(Debug, Parser)]
#[clap(name = "vachat", author, version, about)]
struct Options {
    /// Path of the config file
    #[clap(default_value = "config/config.toml")]
    pub config: PathBuf,
    /// Start a daemon in the background
    #[cfg(not(windows))]
    #[clap(long = "daemon")]
    pub daemon: bool,
    /// Create pid file, lock it exclusive and write daemon pid.
    #[cfg(not(windows))]
    #[clap(long = "pid.file")]
    pub pid_file: Option<PathBuf>,
    /// Standard output file of the daemon
    #[clap(long = "stdout")]
    pub stdout: Option<PathBuf>,
    /// Server domain
    #[clap(long = "network.domain")]
    network_domain: Vec<String>,
    /// Listener bind address
    #[clap(long = "network.bind")]
    network_bind: Option<String>,
    /// Tls type (none, self_signed, certificate, acme_http_01,
    /// acme_tls_alpn_01)
    #[clap(long = "network.tls.type")]
    network_tls_type: Option<String>,
    /// Certificate file path
    #[clap(long = "network.tls.cert")]
    network_tls_cert_path: Option<String>,
    /// Certificate key path
    #[clap(long = "network.tls.key")]
    network_tls_key_path: Option<String>,
    /// Listener bind address for AcmeHTTP_01
    #[clap(long = "network.tls.acme.http_bind")]
    network_tls_acme_http_bind: Option<String>,
    /// Frontend url
    #[clap(long = "network.frontend_url")]
    frontend_url: Option<String>,
    /// Acme directory url
    #[clap(
        long = "network.tls.acme.directory_url",
        default_value = "https://acme-v02.api.letsencrypt.org/directory"
    )]
    network_tls_acme_directory_url: String,
    /// Cache path for certificates
    #[clap(long = "network.tls.acme.cache_path")]
    network_tls_acme_cache_path: Option<String>,
}

impl Options {
    fn merge_to_config(self, mut config: Config) -> Config {
        config.network.domain.extend(self.network_domain);
        if let Some(network_bind) = self.network_bind {
            config.network.bind = network_bind;
        }

        if let Some(network_tls_type) = self.network_tls_type {
            match network_tls_type.as_str() {
                "none" => config.network.tls = None,
                "self_signed" => config.network.tls = Some(TlsConfig::SelfSigned),
                "certificate" => match (self.network_tls_cert_path, self.network_tls_key_path) {
                    (Some(cert_path), Some(key_path)) => {
                        config.network.tls = Some(TlsConfig::Certificate {
                            cert: None,
                            cert_path: Some(cert_path),
                            key: None,
                            key_path: Some(key_path),
                        });
                    }
                    (None, _) => {
                        tracing::warn!("`network.tls.cert` is required");
                    }
                    (_, None) => {
                        tracing::warn!("`network.tls.key` is required");
                    }
                },
                "acme_http_01" => match self.network_tls_acme_http_bind {
                    Some(http_bind) => {
                        config.network.tls = Some(TlsConfig::AcmeHttp01 {
                            http_bind,
                            directory_url: Some(self.network_tls_acme_directory_url),
                            cache_path: self.network_tls_acme_cache_path,
                        });
                    }
                    None => {
                        tracing::warn!("`network.tls.acme.http_bind` is required");
                    }
                },
                "acme_tls_alpn_01" => {
                    config.network.tls = Some(TlsConfig::AcmeTlsAlpn01 {
                        directory_url: Some(self.network_tls_acme_directory_url),
                        cache_path: self.network_tls_acme_cache_path,
                    });
                }
                _ => {
                    tracing::warn!(
                        r#type = network_tls_type.as_str(),
                        "unknown `network.tls.type`"
                    );
                }
            }
        }

        if let Some(frontend_url) = self.frontend_url {
            config.network.frontend_url = frontend_url;
        }

        config
    }
}

fn read_log_level(config_path: &Path) -> String {
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return "vachat=debug,poem=debug".to_string(),
    };
    match toml::from_str::<toml::Value>(&content) {
        Ok(value) => value
            .get("system")
            .and_then(|s| s.get("log_level"))
            .and_then(|l| l.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "vachat=debug,poem=debug".to_string()),
        Err(_) => "vachat=debug,poem=debug".to_string(),
    }
}

fn init_tracing(with_ansi: bool, log_level: &str) {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", log_level);
    }

    // 创建日志目录
    std::fs::create_dir_all("data/logs").ok();

    // 创建文件 appender（按天滚动）
    let file_appender = tracing_appender::rolling::daily("data/logs", "log.txt");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // 控制台 layer - 使用 ANSI 颜色
    let stdout_layer = fmt::layer()
        .with_ansi(with_ansi)
        .with_filter(EnvFilter::from_default_env());

    // 文件 layer - 禁用 ANSI 颜色（包括 fmt 和 span）
    let file_layer = fmt::layer()
        .with_ansi(false) // 禁用所有 ANSI 颜色
        .with_writer(non_blocking)
        .with_filter(EnvFilter::from_default_env());

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .init();

    // 保持 _guard 在整个程序生命周期中有效
    std::mem::forget(_guard);
}

fn load_config(path: &Path) -> anyhow::Result<Config> {
    let data = std::fs::read(path)?;
    Ok(toml::from_slice(&data)?)
}

fn main() {
    let options: Options = Options::parse();

    // Pre-read log_level from config before tracing is initialized
    let log_level = read_log_level(&options.config);

    #[cfg(not(windows))]
    if options.daemon {
        use std::fs::File;
        use std::io::Write;
        use std::os::unix::io::AsRawFd;

        // First fork
        match unsafe { libc::fork() } {
            -1 => {
                tracing::error!("failed to fork");
                return;
            }
            0 => {} // child continues
            _ => std::process::exit(0), // parent exits
        }

        // Create new session
        if unsafe { libc::setsid() } == -1 {
            tracing::error!("failed to setsid");
            return;
        }

        // Second fork (ensure no controlling terminal)
        match unsafe { libc::fork() } {
            -1 => {
                tracing::error!("failed to second fork");
                return;
            }
            0 => {} // child continues
            _ => std::process::exit(0), // intermediate parent exits
        }

        // Set working directory
        if let Ok(cwd) = std::env::current_dir() {
            unsafe { libc::chdir(cwd.as_os_str().as_encoded_bytes().as_ptr() as *const i8) };
        }

        // Reset file mode creation mask
        unsafe { libc::umask(0) };

        // Redirect stdout
        if let Some(stdout_file) = &options.stdout {
            match File::create(stdout_file) {
                Ok(file) => {
                    let fd = file.as_raw_fd();
                    unsafe {
                        libc::dup2(fd, 1);
                        libc::dup2(fd, 2);
                    }
                }
                Err(err) => {
                    tracing::error!(
                        path = %stdout_file.display(),
                        error = %err,
                        "failed to create file"
                    );
                }
            }
        } else {
            // Redirect stdout/stderr to /dev/null
            let devnull = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const i8, libc::O_RDWR) };
            if devnull != -1 {
                unsafe {
                    libc::dup2(devnull, 0);
                    libc::dup2(devnull, 1);
                    libc::dup2(devnull, 2);
                    libc::close(devnull);
                }
            }
        }

        // Write pid file
        if let Some(pid_file) = &options.pid_file {
            match File::create(pid_file) {
                Ok(mut file) => {
                    let pid = unsafe { libc::getpid() };
                    let _ = write!(file, "{}", pid);
                }
                Err(err) => {
                    tracing::error!(
                        path = %pid_file.display(),
                        error = %err,
                        "failed to create pid file"
                    );
                }
            }
        }

        init_tracing(false, &log_level);
    } else {
        init_tracing(true, &log_level);
    }

    #[cfg(windows)]
    init_tracing(true, &log_level);

    Runtime::new().unwrap().block_on(async move {
        // load config
        tracing::info!(
            current_dir = %std::env::current_dir().unwrap().display(),
            path = %options.config.display(),
            "load configuration file.",
        );

        tracing::info!("log level set to {}", log_level);

        let config_path = options.config.clone();
        let config = Arc::new(match load_config(&config_path) {
            Ok(config) => envy::prefixed("VACHAT_")
                .from_env::<EnvironmentVars>()
                .unwrap_or_default()
                .merge_to_config(options.merge_to_config(config)),
            Err(err) => {
                tracing::error!(
                    path = %config_path.display(),
                    error = %err,
                    "failed to load configuration file."
                );
                return;
            }
        });

        let state = match server::create_state(config_path.parent().unwrap(), config.clone()).await
        {
            Ok(state) => state,
            Err(err) => {
                tracing::error!(
                    error = %err,
                    "failed to create server."
                );
                return;
            }
        };

        let auto_cert = match &config.network.tls {
            Some(tls) => match tls.create_auto_cert(&config.network.domain) {
                Ok(auto_cert) => auto_cert,
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        "failed to create auto certificate manager"
                    );
                    return;
                }
            },
            None => None,
        };

        let app = match &config.network.tls {
            Some(TlsConfig::AcmeHttp01 { .. }) => RouteScheme::new()
                .https(server::create_endpoint(state.clone()).await)
                .http(auto_cert.as_ref().unwrap().http_01_endpoint())
                .boxed(),
            _ => server::create_endpoint(state.clone())
                .await
                .map_to_response()
                .boxed(),
        };

        tokio::spawn({
            let state = state.clone();
            async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    state.clean_mute().await;
                    state.clean_sessions().await;
                    state.sync_bot_key_last_used().await;

                    tokio::task::spawn_blocking({
                        let state = state.clone();
                        move || {
                            state.clean_temp_files();
                            state.clean_files();
                        }
                    });
                }
            }
        });

        tokio::spawn({
            let state = state.clone();
            async move {
                loop {
                    state.clean_guest().await;
                    tokio::time::sleep(Duration::from_secs(60 * 60 * 24)).await;
                }
            }
        });

        let mut listener = TcpListener::bind(config.network.bind.to_string()).boxed();
        if let Some(tls_config) = &config.network.tls {
            listener = match tls_config.transform_listener(listener, auto_cert) {
                Ok(listener) => listener,
                Err(err) => {
                    tracing::error!(error = %err, "failed to create listener");
                    return;
                }
            };
            if let TlsConfig::AcmeHttp01 { http_bind, .. } = &tls_config {
                listener = listener
                    .combine(TcpListener::bind(http_bind.clone()))
                    .boxed();
            }
        }

        Server::new(listener).run(app).await.unwrap();
    });
}
