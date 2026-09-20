// SPDX-License-Identifier: GPL-3.0-or-later
//! `redeluge-web`: the Web UI server.
//!
//! It replaces the Python `deluge-web` and nothing else. The daemon on the
//! other end is still whatever is listening on the RPC port, which during the
//! migration is the Python one.

use std::sync::Arc;
use std::time::Duration;

use actix_web::{web, App, HttpServer};
use redeluge_rpc::ClientSettings;
use redeluge_web::auth::{Sessions, StoredPassword};
use redeluge_web::config::{config_dir, ConfigFile};
use redeluge_web::json_api::{self, DEFAULT_SESSION_TIMEOUT};
use redeluge_web::state::{AppState, EventQueue, Settings, SharedState};
use redeluge_web::{assets, bootstrap, hostlist};
use tokio::sync::{Mutex, RwLock};

/// The version the Web UI reports, which is this fork's own.
///
/// Compiled in rather than read from the environment. It used to be an
/// override, from a time when the number in the page and the number the daemon
/// reported were two different things; now that they are one thing, a second
/// source is only a way for them to disagree — and they did, leaving a page
/// titled with whatever a stale `REDELUGE_VERSION` happened to say.
///
/// The image still takes a `VERSION` build argument, for the registry labels.
/// That is what it is for and it says nothing about what is running.
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // `--health-check` connects to the port this server would serve on and
    // says whether anything is listening. It exists so the container image
    // needs no curl for its HEALTHCHECK.
    if std::env::args().any(|arg| arg == "--health-check") {
        let port: u16 = std::env::var("DELUGE_WEB_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8112);
        return match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(_) => Ok(()),
            Err(err) => {
                eprintln!("nothing listening on port {port}: {err}");
                std::process::exit(1);
            }
        };
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config_dir = config_dir();

    // A fresh configuration needs a password and a daemon to talk to, and
    // neither is something anyone should have to write into a file by hand.
    let wanted = bootstrap::Wanted {
        password: std::env::var("DELUGE_WEB_PASSWORD")
            .ok()
            .filter(|value| !value.is_empty()),
        reset_password: std::env::var("DELUGE_WEB_PASSWORD_RESET").as_deref() == Ok("1"),
        daemon_port: env_or("DELUGE_DAEMON_PORT", "58846")
            .parse()
            .unwrap_or(58846),
    };
    match bootstrap::run(&config_dir, &wanted) {
        Ok(outcome) => {
            if outcome.password_set {
                tracing::info!("web password set from DELUGE_WEB_PASSWORD");
            }
            if outcome.host_added {
                tracing::info!("added a host entry for the local daemon");
            }
        }
        Err(err) => tracing::error!(error = %err, "could not prepare the configuration"),
    }

    let web_conf_path = config_dir.join("web.conf");
    let web_config = ConfigFile::load(&web_conf_path).unwrap_or_else(|err| {
        tracing::warn!(error = %err, "could not read web.conf, using defaults");
        ConfigFile {
            path: web_conf_path.clone(),
            version: Default::default(),
            settings: Default::default(),
        }
    });

    let host_config = ConfigFile::load(config_dir.join("hostlist.conf")).unwrap_or_else(|err| {
        tracing::warn!(error = %err, "could not read hostlist.conf");
        ConfigFile {
            path: config_dir.join("hostlist.conf"),
            version: Default::default(),
            settings: Default::default(),
        }
    });
    let hosts = hostlist::load(&host_config);

    if !assets::contains("index.html") {
        tracing::error!("the Web UI assets were not embedded; this binary is unusable");
    }

    let settings = Settings {
        config_dir: config_dir.clone(),
        interface: env_or(
            "DELUGE_WEB_INTERFACE",
            web_config.string("interface").unwrap_or("0.0.0.0"),
        ),
        port: env_or("DELUGE_WEB_PORT", "")
            .parse()
            .ok()
            .or_else(|| {
                web_config
                    .integer("port")
                    .and_then(|p| u16::try_from(p).ok())
            })
            .unwrap_or(8112),
        base: normalise_base(&env_or(
            "DELUGE_WEB_BASE",
            web_config.string("base").unwrap_or("/"),
        )),
        session_timeout: web_config
            .integer("session_timeout")
            .and_then(|seconds| u64::try_from(seconds).ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_SESSION_TIMEOUT),
        theme: web_config
            .string("theme")
            .unwrap_or(redeluge_web::routes::DEFAULT_THEME)
            .to_owned(),
        default_daemon: web_config
            .string("default_daemon")
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        version: VERSION.to_owned(),
    };

    let password =
        StoredPassword::from_config(web_config.string("pwd_salt"), web_config.string("pwd_sha1"));
    if matches!(password, StoredPassword::Unset) {
        tracing::warn!(
            "no web password is set; nobody can log in. Set DELUGE_WEB_PASSWORD \
             and restart, or set one from another client."
        );
    }

    let bind = format!("{}:{}", settings.interface, settings.port);
    let base = settings.base.clone();

    let state: SharedState = Arc::new(AppState {
        settings,
        password: RwLock::new(password),
        sessions: Mutex::new(Sessions::new()),
        login_throttle: Mutex::new(redeluge_web::throttle::Throttle::new()),
        hosts: RwLock::new(hosts),
        daemon: RwLock::new(None),
        client_settings: ClientSettings::default(),
        events: Mutex::new(EventQueue::default()),
        events_ready: tokio::sync::Notify::new(),
        web_config: RwLock::new(web_config),
        slow_stats: Mutex::new(Default::default()),
        baselines: Mutex::new(Default::default()),
        verifications: tokio::sync::Semaphore::new(redeluge_web::state::VERIFICATIONS_AT_ONCE),
        daemon_methods: Mutex::new(None),
    });

    // Connect up front when the configuration names a daemon, so the first page
    // load shows torrents instead of a connection manager.
    if let Some(host_id) = state.settings.default_daemon.clone() {
        match json_api::connect_to(&host_id, &state).await {
            Ok(()) => tracing::info!(host_id, "connected to the default daemon"),
            Err(err) => tracing::warn!(host_id, error = %err, "could not connect at startup"),
        }
    }

    spawn_session_sweeper(state.clone());
    spawn_daemon_supervisor(state.clone());
    spawn_upload_sweeper(state.clone());

    tracing::info!(%bind, base = %base, "serving the Web UI");

    let server_state = state.clone();
    HttpServer::new(move || {
        App::new()
            // The bundle is 300 KB of JavaScript and the page pulls a few
            // hundred assets. The feature was enabled and the middleware was
            // not, so nothing was ever compressed.
            .wrap(actix_web::middleware::Compress::default())
            .app_data(web::Data::new(server_state.clone()))
            // Deluge sends large status responses and accepts torrent files; the
            // default 256 KiB payload cap is too small for either.
            .app_data(web::JsonConfig::default().limit(8 * 1024 * 1024))
            .configure(redeluge_web::routes::configure)
    })
    .bind(&bind)?
    .run()
    .await
}

/// Drops expired sessions, so a long-running server does not accumulate them.
/// Reconnects to the daemon when the connection goes away.
///
/// The daemon restarting is the ordinary case: a container update, a crash, an
/// operator. Before this, the Web UI connected once at startup and then said
/// "connection lost" until it was restarted itself.
///
/// There is no heartbeat in DelugeRPC, so a dead connection is only visible as
/// a closed channel. The backoff exists so that a daemon that is down for an
/// hour does not mean an hour of connection attempts every five seconds.
fn spawn_daemon_supervisor(state: SharedState) {
    tokio::spawn(async move {
        const MIN_WAIT: Duration = Duration::from_secs(5);
        const MAX_WAIT: Duration = Duration::from_secs(60);
        let mut wait = MIN_WAIT;

        loop {
            tokio::time::sleep(wait).await;

            // Which daemon to reconnect to: the one that was connected, or the
            // configured default if nothing ever connected.
            let lost = {
                let guard = state.daemon.read().await;
                match guard.as_ref() {
                    Some(connection) if connection.client.is_closed() => {
                        Some(connection.host_id.clone())
                    }
                    Some(_) => None,
                    None => state.settings.default_daemon.clone(),
                }
            };

            let Some(host_id) = lost else {
                wait = MIN_WAIT;
                continue;
            };

            // Drop the dead connection first, so anything asking in the
            // meantime is told plainly rather than timing out on a dead socket.
            {
                let mut guard = state.daemon.write().await;
                if guard.as_ref().is_some_and(|c| c.client.is_closed()) {
                    tracing::warn!(host_id, "the daemon connection went away");
                    *guard = None;
                }
            }

            match json_api::connect_to(&host_id, &state).await {
                Ok(()) => {
                    tracing::info!(host_id, "reconnected to the daemon");
                    wait = MIN_WAIT;
                }
                Err(err) => {
                    tracing::debug!(host_id, error = %err, "could not reconnect yet");
                    wait = (wait * 2).min(MAX_WAIT);
                }
            }
        }
    });
}

/// Removes staged uploads nothing came back for.
///
/// The add dialog leaves a file behind whenever it is cancelled after the
/// upload, and nothing else ever removes them.
fn spawn_upload_sweeper(state: SharedState) {
    tokio::spawn(async move {
        const KEEP_FOR: Duration = Duration::from_secs(3600);
        let mut ticker = tokio::time::interval(Duration::from_secs(600));
        loop {
            ticker.tick().await;
            let removed =
                redeluge_web::upload::sweep_staging(&state.settings.config_dir, KEEP_FOR).await;
            if removed > 0 {
                tracing::debug!(removed, "stale uploads removed");
            }
        }
    });
}

fn spawn_session_sweeper(state: SharedState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            let dropped = state.sessions.lock().await.sweep();
            if dropped > 0 {
                tracing::debug!(dropped, "expired sessions removed");
            }
            // What each session was last told about the torrents is as big as
            // the library, so it goes when the session does.
            {
                let sessions = state.sessions.lock().await;
                state
                    .baselines
                    .lock()
                    .await
                    .keep_only(&|id: &str| sessions.holds(id));
            }
            let forgotten = state
                .login_throttle
                .lock()
                .await
                .sweep(std::time::Instant::now());
            if forgotten > 0 {
                tracing::debug!(forgotten, "login budgets forgotten");
            }
        }
    });
}

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_owned())
}

/// A base path always has one leading and one trailing slash, so joining it to
/// an asset path never produces `//` or a missing separator.
fn normalise_base(base: &str) -> String {
    let trimmed = base.trim().trim_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        format!("/{trimmed}/")
    }
}
