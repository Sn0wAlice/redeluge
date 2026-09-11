// SPDX-License-Identifier: GPL-3.0-or-later
//! `redeluge-web`: the Web UI server.
//!
//! It replaces the Python `deluge-web` and nothing else. The daemon on the
//! other end is still whatever is listening on the RPC port, which during the
//! migration is the Python one.

use std::sync::Arc;
use std::time::Duration;

use actix_web::http::header;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use redeluge_rpc::ClientSettings;
use redeluge_web::auth::{Sessions, StoredPassword};
use redeluge_web::config::{config_dir, ConfigFile};
use redeluge_web::json_api::{self, DEFAULT_SESSION_TIMEOUT};
use redeluge_web::state::{AppState, EventQueue, Settings, SharedState};
use redeluge_web::{assets, bootstrap, hostlist, index};
use tokio::sync::{Mutex, RwLock};

/// The version the Web UI reports, matching what the daemon tells clients.
///
/// Not this crate's own version: clients compare it against the Deluge they
/// know how to speak, and a page titled 0.1.0 is a page that looks broken.
/// Keep "dev" out of it, or the Web UI asks for unbundled source assets.
const DELUGE_COMPATIBLE_VERSION: &str = "2.2.1";

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
        theme: web_config.string("theme").unwrap_or("gray").to_owned(),
        default_daemon: web_config
            .string("default_daemon")
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        version: env_or("REDELUGE_VERSION", DELUGE_COMPATIBLE_VERSION),
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
        hosts: RwLock::new(hosts),
        daemon: RwLock::new(None),
        client_settings: ClientSettings::default(),
        events: Mutex::new(EventQueue::default()),
        web_config: RwLock::new(web_config),
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

    tracing::info!(%bind, base = %base, "serving the Web UI");

    let server_state = state.clone();
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(server_state.clone()))
            // Deluge sends large status responses and accepts torrent files; the
            // default 256 KiB payload cap is too small for either.
            .app_data(web::JsonConfig::default().limit(8 * 1024 * 1024))
            .route("/json", web::post().to(json_api::handle))
            .route("/", web::get().to(serve_index))
            .route("/render/{name}", web::get().to(serve_render))
            // Anything else is an embedded asset, or the page again so the
            // front end's own routing works on a reload.
            .default_service(web::get().to(serve_asset))
    })
    .bind(&bind)?
    .run()
    .await
}

/// Renders the page. Everything the browser does after this is a JSON call.
async fn serve_index(request: HttpRequest, state: web::Data<SharedState>) -> HttpResponse {
    let state = state.get_ref();

    let Some(raw) = assets::get("index.html") else {
        return HttpResponse::InternalServerError().body("Web UI assets are missing");
    };
    let template = String::from_utf8_lossy(raw);

    let debug = request
        .query_string()
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .any(|(key, value)| key == "debug" && matches!(value, "true" | "yes" | "on" | "1"));

    let theme = state.web_config.read().await;
    let theme = theme
        .string("theme")
        .unwrap_or(&state.settings.theme)
        .to_owned();

    // Only the keys the page's inline script reads; the rest arrive from
    // web.get_config once ExtJS is running.
    let js_config = serde_json::json!({
        "theme": theme,
        "sidebar_show_zero": false,
        "sidebar_multiple_filters": true,
        "show_session_speed": false,
        "base": state.settings.base,
        "first_login": false,
    });

    match index::render_index(
        &template,
        &state.settings.base,
        &state.settings.version,
        &theme,
        &js_config,
        debug,
    ) {
        Ok(html) => HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(html),
        Err(err) => {
            tracing::error!(error = %err, "could not render the index template");
            HttpResponse::InternalServerError().body("Could not render the Web UI")
        }
    }
}

/// Serves a `render/` template.
///
/// These are HTML fragments the front end fetches and drops into the page. They
/// carry `${_("...")}` markers, which the Python server resolved through Mako
/// and gettext on every request; English only makes that substitution the
/// identity, but it still has to happen or the markers reach the browser.
async fn serve_render(path: web::Path<String>, state: web::Data<SharedState>) -> HttpResponse {
    let name = path.into_inner();
    // No path traversal: the name indexes the embedded set and nothing else.
    let known = assets::contains(&format!("render/{name}"));
    let (file, status) = if known {
        (format!("render/{name}"), actix_web::http::StatusCode::OK)
    } else {
        (
            "render/404.html".to_owned(),
            actix_web::http::StatusCode::NOT_FOUND,
        )
    };

    let Some(raw) = assets::get(&file) else {
        return HttpResponse::NotFound().body("not found");
    };

    let context = redeluge_web::template::Context::new()
        .set("version", state.get_ref().settings.version.clone());

    match redeluge_web::template::render(&String::from_utf8_lossy(raw), &context) {
        Ok(html) => HttpResponse::build(status)
            .insert_header((header::CONTENT_TYPE, "text/html; charset=utf-8"))
            .body(html),
        Err(err) => {
            tracing::error!(file, error = %err, "could not render a template");
            HttpResponse::InternalServerError().body("template error")
        }
    }
}

/// Serves one embedded asset, or the page when the path names no asset.
async fn serve_asset(request: HttpRequest, state: web::Data<SharedState>) -> HttpResponse {
    let base = &state.get_ref().settings.base;
    let path = request.path();
    // Strip the base prefix so the same asset resolves under a subpath.
    let relative = path
        .strip_prefix(base.as_str())
        .unwrap_or_else(|| path.trim_start_matches('/'));

    match assets::get(relative) {
        Some(bytes) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, assets::content_type(relative)))
            // The assets are versioned with the binary, so a long cache is safe
            // and saves the browser a few hundred requests per page load.
            .insert_header((header::CACHE_CONTROL, "public, max-age=3600"))
            .body(bytes),
        None => serve_index(request, state).await,
    }
}

/// Drops expired sessions, so a long-running server does not accumulate them.
fn spawn_session_sweeper(state: SharedState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            let dropped = state.sessions.lock().await.sweep();
            if dropped > 0 {
                tracing::debug!(dropped, "expired sessions removed");
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
