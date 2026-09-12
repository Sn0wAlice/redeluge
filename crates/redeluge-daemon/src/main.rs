// SPDX-License-Identifier: GPL-3.0-or-later
//! `redeluged`: the daemon.
//!
//! Starts libtorrent, restores the torrents from the previous run, and listens
//! for DelugeRPC. Any client that speaks the protocol can drive it: the Rust
//! Web UI, or the Python one during the migration.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use redeluge_daemon::auth::AuthManager;
use redeluge_daemon::config::Config;
use redeluge_daemon::core::{Core, REPORTED_VERSION};
use redeluge_daemon::events::Event;
use redeluge_daemon::manager::Manager;
use redeluge_daemon::rpc::{tls, Server, ServerConfig};
use redeluge_libtorrent::SessionSettings;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config_dir = config_dir();
    std::fs::create_dir_all(&config_dir)?;
    tracing::info!(
        version = REPORTED_VERSION,
        libtorrent = %redeluge_libtorrent::libtorrent_version(),
        config = %config_dir.display(),
        "redeluged starting"
    );

    let config = Config::load(&config_dir)?;
    let auth = AuthManager::open(&config_dir)?;

    // Where the daemon listens is a decision, not a default: allow_remote off
    // means loopback, which is what an unattended install should be.
    let port = config.integer("daemon_port").unwrap_or(58846) as u16;
    let host = if config.boolean("allow_remote").unwrap_or(false) {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    let address = format!("{host}:{port}").parse()?;

    let (events, _) = tokio::sync::broadcast::channel::<Event>(4096);
    let settings = SessionSettings {
        user_agent: format!(
            "redeluge/{REPORTED_VERSION} libtorrent/{}",
            redeluge_libtorrent::libtorrent_version()
        ),
        ..SessionSettings::default()
    };
    let manager = Manager::start(config_dir.clone(), settings, events.clone())?;

    let core = Core::new(manager.clone(), config, auth, config_dir.clone());
    core.load_country_database().await;
    core.save_config().await;
    core.apply_config().await;
    core.restore().await;

    let acceptor = tls::into_acceptor(tls::server_config(&config_dir)?);
    if let Ok(fingerprint) = tls::fingerprint(&config_dir) {
        tracing::info!(sha256 = %fingerprint, "daemon certificate");
    }

    let server = Server::bind(
        ServerConfig {
            address,
            ..ServerConfig::default()
        },
        acceptor,
    )
    .await?;

    // Events from the manager go out to every interested client.
    {
        let server = Arc::clone(&server);
        let mut incoming = events.subscribe();
        tokio::spawn(async move {
            loop {
                match incoming.recv().await {
                    Ok(event) => server.broadcast(event),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::warn!(missed, "the daemon fell behind on its own events");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    spawn_auth_watcher(Arc::clone(&core));
    redeluge_daemon::features::spawn(Arc::clone(&core));
    manager.announce(Event::SessionStarted);

    let serving = tokio::spawn({
        let server = Arc::clone(&server);
        let core = Arc::clone(&core);
        async move { server.serve(core).await }
    });

    // Ctrl-C and SIGTERM both mean stop, and stopping means writing state
    // before the process goes away.
    let shutdown = async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => tracing::info!("interrupted"),
            _ = term.recv() => tracing::info!("terminated"),
            _ = core.shutdown.notified() => tracing::info!("shutdown requested by a client"),
        }
    };
    shutdown.await;

    tracing::info!("saving state before exit");
    core.save_config().await;
    let saved = manager
        .with(|state| {
            // Ask for resume data for everything, then write what has arrived.
            for id in state.session.torrent_hashes() {
                let _ = state.session.save_resume_data(&id, true);
            }
            state.mark_dirty();
            let _ = state.save_state();
        })
        .await;
    if saved.is_err() {
        tracing::warn!("the torrent manager was already gone");
    }

    // Resume data arrives on alerts, so the manager needs a moment to collect
    // it before the process exits.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let written = manager.with(|state| state.save_resume_data()).await;
    match written {
        Ok(Ok(count)) => tracing::info!(count, "wrote resume data"),
        Ok(Err(err)) => tracing::warn!(error = %err, "could not write resume data"),
        Err(_) => {}
    }

    serving.abort();
    tracing::info!("stopped");
    Ok(())
}

/// Re-reads the auth file when it changes, so an account can be added without
/// restarting the daemon.
fn spawn_auth_watcher(core: Arc<Core>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(10));
        loop {
            ticker.tick().await;
            let mut auth = core.auth.lock().await;
            match auth.reload_if_changed() {
                Ok(true) => tracing::info!("auth file reloaded"),
                Ok(false) => {}
                Err(err) => tracing::warn!(error = %err, "could not reload the auth file"),
            }
        }
    });
}

fn config_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("DELUGE_CONFIG_DIR") {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("deluge");
        }
    }
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => PathBuf::from(home).join(".config").join("deluge"),
        _ => PathBuf::from("/config"),
    }
}
