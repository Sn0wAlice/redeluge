// SPDX-License-Identifier: GPL-3.0-or-later
//! `POST /json`, the endpoint the Web UI talks to.
//!
//! JSON-RPC v1: `{"method": ..., "params": [...], "id": N}` in, and
//! `{"result": ..., "error": ..., "id": N}` out. The error codes are the Python
//! server's, because the ExtJS front end reads them:
//!
//! | code | meaning |
//! |------|---------|
//! | 1 | not authenticated |
//! | 2 | unknown method |
//! | 3 | the call failed here |
//! | 4 | the daemon refused the call |
//!
//! Methods in the `core.` and `daemon.` namespaces belong to the daemon and are
//! forwarded. Everything else is about the web session and is answered here.

use std::time::Duration;

use actix_web::{cookie::Cookie, http::header, web, HttpRequest, HttpResponse};
use redeluge_rencode::Value;
use serde::Deserialize;
use serde_json::{json, Map, Value as Json};

use crate::auth::hash_password;
use crate::convert::{json_to_rencode, rencode_to_json};
use crate::state::{DaemonConnection, SharedState};

/// Authorisation levels, as the daemon numbers them.
pub const AUTH_LEVEL_NONE: i64 = 0;
pub const AUTH_LEVEL_ADMIN: i64 = 10;

/// The cookie the Web UI expects. Changing the name logs everyone out.
const SESSION_COOKIE: &str = "_session_id";

#[derive(Debug, Deserialize)]
pub struct JsonRequest {
    pub method: String,
    #[serde(default)]
    pub params: Vec<Json>,
    pub id: Json,
}

/// A failure with the code the front end reads.
struct ApiError {
    code: i64,
    message: String,
}

impl ApiError {
    fn not_authenticated() -> Self {
        Self {
            code: 1,
            message: "Not authenticated".to_owned(),
        }
    }

    fn unknown_method() -> Self {
        Self {
            code: 2,
            message: "Unknown method".to_owned(),
        }
    }

    fn local(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            message: message.into(),
        }
    }

    fn remote(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            message: message.into(),
        }
    }

    fn to_json(&self) -> Json {
        json!({"message": self.message, "code": self.code})
    }
}

type ApiResult = Result<Json, ApiError>;

/// What a dispatched call produced: its result, and a session it may have
/// created. Logging in is the only call that mints one, and the cookie has to
/// be set by the handler rather than smuggled through the JSON body.
struct Dispatched {
    result: Json,
    new_session: Option<String>,
}

/// Handles one JSON-RPC call.
pub async fn handle(
    request: HttpRequest,
    payload: web::Json<JsonRequest>,
    state: web::Data<SharedState>,
) -> HttpResponse {
    let state = state.get_ref().clone();
    let call = payload.into_inner();
    let id = call.id.clone();

    let session = current_session(&request, &state).await;
    let level = session.as_ref().map_or(AUTH_LEVEL_NONE, |s| s.1);

    let outcome = dispatch(&request, &call, &state, level).await;

    let mut response = HttpResponse::Ok();
    response.insert_header((header::CONTENT_TYPE, "application/json"));

    match outcome {
        Ok(dispatched) => {
            // A fresh session takes precedence; otherwise extend the one that
            // came in, which is the sliding expiry the Python server does.
            if let Some(new_id) = dispatched.new_session.as_deref() {
                response.cookie(session_cookie(new_id, &state));
            } else if let Some((id, _)) = session {
                response.cookie(session_cookie(&id, &state));
            }
            response.json(json!({"result": dispatched.result, "error": null, "id": id}))
        }
        Err(error) => {
            if let Some((id, _)) = session {
                response.cookie(session_cookie(&id, &state));
            }
            response.json(json!({"result": null, "error": error.to_json(), "id": id}))
        }
    }
}

async fn dispatch(
    request: &HttpRequest,
    call: &JsonRequest,
    state: &SharedState,
    level: i64,
) -> Result<Dispatched, ApiError> {
    // Three calls answer before a session exists: logging in, asking whether a
    // session is still good, and the method list the front end fetches on load.
    let open = matches!(
        call.method.as_str(),
        "auth.login" | "auth.check_session" | "system.listMethods"
    );
    if !open && level < AUTH_LEVEL_ADMIN {
        return Err(ApiError::not_authenticated());
    }

    // Logging in is the one call that creates a session, so it is handled
    // apart from the rest, which only ever return a value.
    if call.method == "auth.login" {
        return auth_login(request, call, state).await;
    }

    let result = match call.method.as_str() {
        "auth.check_session" => Ok(Json::Bool(level >= AUTH_LEVEL_ADMIN)),
        "auth.delete_session" => auth_delete_session(request, state).await,
        "auth.change_password" => auth_change_password(call, state).await,

        "system.listMethods" => system_list_methods(state).await,

        "web.connected" => Ok(Json::Bool(state.daemon.read().await.is_some())),
        "web.connect" => web_connect(call, state).await,
        "web.disconnect" => web_disconnect(state).await,
        "web.get_hosts" => web_get_hosts(state).await,
        "web.get_host_status" => web_get_host_status(call, state).await,
        "web.update_ui" => web_update_ui(call, state).await,
        "web.get_config" => web_get_config(state).await,
        "web.set_config" => web_set_config(call, state).await,
        "web.get_themes" | "webutils.get_themes" => web_get_themes(state).await,
        "web.set_theme" => web_set_theme(call, state).await,
        // Deluge exported these two under both names: `web.*` is what the
        // front end calls, `webutils.*` is what the contract records, and a
        // thin client written against the Python server may use either.
        "web.get_languages" | "webutils.get_languages" => Ok(json!([])),

        // The plugin system is gone and so is the interface for it: nothing in
        // the shipped front end calls these any more. They stay because they
        // are in the contract, and a thin client that asks gets the truthful
        // answer rather than an unknown-method error.
        "web.get_plugins" => Ok(json!({
            "enabled_plugins": [],
            "available_plugins": [],
        })),
        "web.get_plugin_info" => Ok(json!({})),
        "web.get_plugin_resources" => Err(ApiError::local("no plugin system")),
        "web.register_event_listener" => web_register_event(call, state, true).await,
        "web.deregister_event_listener" => web_register_event(call, state, false).await,
        "web.get_events" => web_get_events(state).await,

        // Everything in the daemon's namespaces goes to the daemon.
        method if method.starts_with("core.") || method.starts_with("daemon.") => {
            forward(method, &call.params, state).await
        }

        _ => Err(ApiError::unknown_method()),
    }?;

    Ok(Dispatched {
        result,
        new_session: None,
    })
}

// ------------------------------------------------------------------ sessions

/// Returns the session id and its level, if the request carries a live one.
async fn current_session(request: &HttpRequest, state: &SharedState) -> Option<(String, i64)> {
    let raw = request.cookie(SESSION_COOKIE)?;
    let id = strip_checksum(raw.value())?;
    let timeout = state.settings.session_timeout;
    let session = state.sessions.lock().await.touch(&id, timeout)?;
    Some((id, session.level))
}

/// The Python server appends a trivial checksum to the cookie and verifies it.
///
/// It is not a security control, the session id behind it is, but the cookie
/// has to keep the same shape or a browser that logged into the Python server
/// stops working here.
fn strip_checksum(cookie: &str) -> Option<String> {
    if cookie.len() < 5 {
        return None;
    }
    let (id, checksum) = cookie.split_at(cookie.len() - 4);
    if checksum.parse::<u32>().ok()? != checksum_of(id) {
        return None;
    }
    Some(id.to_owned())
}

fn checksum_of(id: &str) -> u32 {
    id.chars().map(|c| c as u32).sum()
}

fn session_cookie(id: &str, state: &SharedState) -> Cookie<'static> {
    let value = format!("{id}{:04}", checksum_of(id) % 10_000);
    Cookie::build(SESSION_COOKIE, value)
        .path(state.settings.base.clone())
        // The Python server sets neither of these. A session cookie readable
        // from JavaScript, or sent on a cross-site request, is a session that
        // can be stolen by a page the user did not mean to trust.
        .http_only(true)
        .same_site(actix_web::cookie::SameSite::Strict)
        .finish()
}

// ---------------------------------------------------------------------- auth

async fn auth_login(
    request: &HttpRequest,
    call: &JsonRequest,
    state: &SharedState,
) -> Result<Dispatched, ApiError> {
    let password = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("auth.login takes a password"))?;

    let stored = state.password.read().await.clone();
    if !stored.verify(password) {
        let peer = request
            .connection_info()
            .realip_remote_addr()
            .unwrap_or("unknown")
            .to_owned();
        tracing::warn!(client = %peer, "web login failed");
        return Ok(Dispatched {
            result: Json::Bool(false),
            new_session: None,
        });
    }

    // A password stored as the old single-round SHA-1 is rewritten as scrypt on
    // the first successful login, so an installation upgrades by being used.
    if stored.needs_upgrade() {
        match hash_password(password) {
            Ok(upgraded) => {
                *state.password.write().await = upgraded;
                if let Err(err) = crate::persist::save_password(state).await {
                    tracing::warn!(error = %err, "could not store the upgraded password");
                }
                tracing::info!("web password upgraded from SHA-1 to scrypt");
            }
            Err(err) => tracing::warn!(error = %err, "could not upgrade the stored password"),
        }
    }

    let timeout = state.settings.session_timeout;
    let id = state
        .sessions
        .lock()
        .await
        .create("admin", AUTH_LEVEL_ADMIN, timeout)
        .map_err(|err| ApiError::local(err.to_string()))?;

    // The cookie carries the session; the body is just the boolean the front
    // end checks.
    Ok(Dispatched {
        result: Json::Bool(true),
        new_session: Some(id),
    })
}

async fn auth_delete_session(request: &HttpRequest, state: &SharedState) -> ApiResult {
    let Some(raw) = request.cookie(SESSION_COOKIE) else {
        return Ok(Json::Bool(false));
    };
    let Some(id) = strip_checksum(raw.value()) else {
        return Ok(Json::Bool(false));
    };
    Ok(Json::Bool(state.sessions.lock().await.remove(&id)))
}

async fn auth_change_password(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let old = call.params.first().and_then(Json::as_str).unwrap_or("");
    let new = call
        .params
        .get(1)
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("auth.change_password takes two passwords"))?;

    if !state.password.read().await.verify(old) {
        return Ok(Json::Bool(false));
    }
    if new.is_empty() {
        return Err(ApiError::local("the new password cannot be empty"));
    }

    let hashed = hash_password(new).map_err(|err| ApiError::local(err.to_string()))?;
    *state.password.write().await = hashed;
    crate::persist::save_password(state)
        .await
        .map_err(|err| ApiError::local(err.to_string()))?;

    // Everything signed in with the old password is now stale.
    state.sessions.lock().await.sweep();
    Ok(Json::Bool(true))
}

// ----------------------------------------------------------------- the daemon

async fn forward(method: &str, params: &[Json], state: &SharedState) -> ApiResult {
    let guard = state.daemon.read().await;
    let connection = guard
        .as_ref()
        .ok_or_else(|| ApiError::remote("not connected to a daemon"))?;

    let args: Vec<Value> = params.iter().map(json_to_rencode).collect();
    match connection.client.call(method, args).await {
        Ok(value) => Ok(rencode_to_json(&value)),
        // The daemon's traceback is deliberately not forwarded: it names paths
        // and versions, and the browser has no use for it.
        Err(redeluge_rpc::client::Error::Remote(failure)) => {
            Err(ApiError::remote(failure.to_string()))
        }
        Err(err) => Err(ApiError::remote(err.to_string())),
    }
}

async fn call_daemon(method: &str, args: Vec<Value>, state: &SharedState) -> Option<Value> {
    let guard = state.daemon.read().await;
    let connection = guard.as_ref()?;
    match connection.client.call(method, args).await {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::debug!(method, error = %err, "daemon call failed");
            None
        }
    }
}

async fn web_connect(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let host_id = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.connect takes a host id"))?;

    connect_to(host_id, state).await.map_err(ApiError::remote)?;

    let methods = match call_daemon("daemon.get_method_list", vec![], state).await {
        Some(value) => rencode_to_json(&value),
        None => json!([]),
    };
    Ok(methods)
}

/// Opens a connection to a host from the hostlist and logs in.
pub async fn connect_to(host_id: &str, state: &SharedState) -> Result<(), String> {
    let host = state
        .hosts
        .read()
        .await
        .iter()
        .find(|candidate| candidate.id == host_id)
        .cloned()
        .ok_or_else(|| format!("no such host: {host_id}"))?;

    let client =
        redeluge_rpc::Client::connect(&host.host, host.port, state.client_settings.clone())
            .await
            .map_err(|err| err.to_string())?;

    client
        .login(&host.username, &host.password)
        .await
        .map_err(|err| err.to_string())?;

    // Ask for everything; the queue filters down to what the browser wants.
    let _ = client.set_event_interest(&[]).await;

    spawn_event_pump(client.clone(), state.clone());

    *state.daemon.write().await = Some(DaemonConnection {
        host_id: host.id.clone(),
        client,
    });
    tracing::info!(host = %host.host, port = host.port, "connected to the daemon");
    Ok(())
}

/// Moves daemon events into the queue the browser polls.
fn spawn_event_pump(client: redeluge_rpc::Client, state: SharedState) {
    tokio::spawn(async move {
        let mut events = client.events();
        loop {
            match events.recv().await {
                Ok(event) => {
                    let args = event.args.iter().map(rencode_to_json).collect();
                    state.events.lock().await.push(event.name, args);
                }
                // Lagging means the browser stopped polling; the queue bound
                // has already discarded the oldest, so carrying on is correct.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::debug!(missed, "event queue lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

async fn web_disconnect(state: &SharedState) -> ApiResult {
    *state.daemon.write().await = None;
    Ok(Json::String("Connection was closed cleanly.".to_owned()))
}

async fn web_get_hosts(state: &SharedState) -> ApiResult {
    let hosts = state.hosts.read().await;
    Ok(Json::Array(
        hosts
            .iter()
            .map(|host| json!([host.id, host.host, host.port, host.username]))
            .collect(),
    ))
}

async fn web_get_host_status(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let host_id = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.get_host_status takes a host id"))?;

    let connected = state
        .daemon
        .read()
        .await
        .as_ref()
        .is_some_and(|connection| connection.host_id == host_id);

    if connected {
        let version = call_daemon("daemon.info", vec![], state)
            .await
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        return Ok(json!([host_id, "Connected", version]));
    }

    // Anything not currently connected is reported offline rather than probed:
    // probing every host on each poll makes the list slow and noisy.
    Ok(json!([host_id, "Offline", ""]))
}

async fn web_update_ui(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let keys = call.params.first().cloned().unwrap_or(json!([]));
    let filters = call.params.get(1).cloned().unwrap_or(json!({}));

    let connected = state.daemon.read().await.is_some();
    let mut stats = Map::new();
    let mut info = Map::new();
    info.insert("connected".to_owned(), Json::Bool(connected));

    if !connected {
        info.insert("torrents".to_owned(), Json::Null);
        info.insert("filters".to_owned(), Json::Null);
        info.insert("stats".to_owned(), Json::Object(stats));
        return Ok(Json::Object(info));
    }

    let torrents = call_daemon(
        "core.get_torrents_status",
        vec![json_to_rencode(&filters), json_to_rencode(&keys)],
        state,
    )
    .await;

    let filter_tree = call_daemon("core.get_filter_tree", vec![], state).await;

    let wanted = Value::List(
        [
            "peer.num_peers_connected",
            "payload_download_rate",
            "payload_upload_rate",
            "download_rate",
            "upload_rate",
            "dht.dht_nodes",
            "net.has_incoming_connections",
        ]
        .into_iter()
        .map(|name| Value::Str(name.to_owned()))
        .collect(),
    );
    let session = call_daemon("core.get_session_status", vec![wanted], state).await;

    if let Some(session) = session.as_ref() {
        let number = |key: &str| session.get(key).map(rencode_to_json).unwrap_or(json!(0));
        let as_f64 = |key: &str| match session.get(key) {
            Some(Value::Float32(value)) => f64::from(*value),
            Some(Value::Float64(value)) => *value,
            Some(Value::Int(value)) => *value as f64,
            _ => 0.0,
        };

        stats.insert(
            "num_connections".to_owned(),
            number("peer.num_peers_connected"),
        );
        stats.insert("upload_rate".to_owned(), number("payload_upload_rate"));
        stats.insert("download_rate".to_owned(), number("payload_download_rate"));
        stats.insert(
            "download_protocol_rate".to_owned(),
            json!(as_f64("download_rate") - as_f64("payload_download_rate")),
        );
        stats.insert(
            "upload_protocol_rate".to_owned(),
            json!(as_f64("upload_rate") - as_f64("payload_upload_rate")),
        );
        stats.insert("dht_nodes".to_owned(), number("dht.dht_nodes"));
        stats.insert(
            "has_incoming_connections".to_owned(),
            number("net.has_incoming_connections"),
        );
    }

    if let Some(free) = call_daemon("core.get_free_space", vec![], state).await {
        stats.insert("free_space".to_owned(), rencode_to_json(&free));
    }
    if let Some(ip) = call_daemon("core.get_external_ip", vec![], state).await {
        stats.insert("external_ip".to_owned(), rencode_to_json(&ip));
    }

    // The front end reads these three from the core config, which it also
    // fetches separately; supplying them keeps the status bar populated on the
    // first poll rather than after the second.
    if let Some(config) = call_daemon("core.get_config", vec![], state).await {
        for (from, to) in [
            ("max_download_speed", "max_download"),
            ("max_upload_speed", "max_upload"),
            ("max_connections_global", "max_num_connections"),
        ] {
            if let Some(value) = config.get(from) {
                stats.insert(to.to_owned(), rencode_to_json(value));
            }
        }
    }

    info.insert(
        "torrents".to_owned(),
        torrents.as_ref().map(rencode_to_json).unwrap_or(Json::Null),
    );
    info.insert(
        "filters".to_owned(),
        filter_tree
            .as_ref()
            .map(rencode_to_json)
            .unwrap_or(Json::Null),
    );
    info.insert("stats".to_owned(), Json::Object(stats));
    Ok(Json::Object(info))
}

async fn web_get_config(state: &SharedState) -> ApiResult {
    let settings = &state.settings;
    Ok(json!({
        "theme": settings.theme,
        "base": settings.base,
        "sidebar_show_zero": state.web_config.read().await.boolean("sidebar_show_zero").unwrap_or(false),
        "sidebar_multiple_filters": state.web_config.read().await.boolean("sidebar_multiple_filters").unwrap_or(true),
        "show_session_speed": state.web_config.read().await.boolean("show_session_speed").unwrap_or(false),
        "show_sidebar": state.web_config.read().await.boolean("show_sidebar").unwrap_or(true),
        "first_login": false,
        "language": "",
        "session_timeout": settings.session_timeout.as_secs(),
        "interface": settings.interface,
        "port": settings.port,
        "https": false,
        "default_daemon": settings.default_daemon.clone().unwrap_or_default(),
    }))
}

async fn web_set_config(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let Some(Json::Object(changes)) = call.params.first() else {
        return Err(ApiError::local("web.set_config takes an object"));
    };

    let mut config = state.web_config.write().await;
    for (key, value) in changes {
        // Deliberately narrow: how the server is reached is the container's
        // business, and letting the browser change it can lock everyone out.
        if matches!(
            key.as_str(),
            "interface" | "port" | "https" | "pkey" | "cert"
        ) {
            tracing::info!(key, "refusing to change a server binding from the Web UI");
            continue;
        }
        config.settings.insert(key.clone(), value.clone());
    }
    let snapshot = config.clone();
    drop(config);

    crate::persist::save_config(&snapshot)
        .await
        .map_err(|err| ApiError::local(err.to_string()))?;
    Ok(Json::Null)
}

async fn web_get_themes(_state: &SharedState) -> ApiResult {
    let mut themes: Vec<String> = crate::assets::files()
        .keys()
        .filter_map(|path| path.strip_prefix("themes/css/xtheme-"))
        .filter_map(|name| name.strip_suffix(".css"))
        .map(str::to_owned)
        .collect();
    themes.sort();
    Ok(json!(themes))
}

async fn web_set_theme(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let theme = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.set_theme takes a name"))?;

    let mut config = state.web_config.write().await;
    config
        .settings
        .insert("theme".to_owned(), Json::String(theme.to_owned()));
    let snapshot = config.clone();
    drop(config);

    crate::persist::save_config(&snapshot)
        .await
        .map_err(|err| ApiError::local(err.to_string()))?;
    Ok(Json::Null)
}

async fn web_register_event(call: &JsonRequest, state: &SharedState, register: bool) -> ApiResult {
    let name = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("an event name is required"))?;

    let mut queue = state.events.lock().await;
    if register {
        queue.register(name);
    } else {
        queue.deregister(name);
    }
    Ok(Json::Null)
}

async fn web_get_events(state: &SharedState) -> ApiResult {
    let events = state.events.lock().await.drain();
    Ok(Json::Array(
        events
            .into_iter()
            .map(|(name, args)| json!([name, args]))
            .collect(),
    ))
}

async fn system_list_methods(state: &SharedState) -> ApiResult {
    let mut methods: Vec<String> = LOCAL_METHODS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();

    if let Some(value) = call_daemon("daemon.get_method_list", vec![], state).await {
        if let Some(items) = value.as_list() {
            methods.extend(
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned)),
            );
        }
    }
    methods.sort();
    methods.dedup();
    Ok(json!(methods))
}

/// Everything answered here rather than by the daemon.
pub const LOCAL_METHODS: &[&str] = &[
    "auth.change_password",
    "auth.check_session",
    "auth.delete_session",
    "auth.login",
    "system.listMethods",
    "web.connect",
    "web.connected",
    "web.deregister_event_listener",
    "web.disconnect",
    "web.get_config",
    "web.get_events",
    "web.get_host_status",
    "web.get_hosts",
    "web.get_languages",
    "web.get_plugin_info",
    "web.get_plugins",
    "web.get_themes",
    "web.register_event_listener",
    "web.set_config",
    "web.set_theme",
    "web.update_ui",
    "webutils.get_languages",
    "webutils.get_themes",
];

/// How long a session lives when `web.conf` does not say.
pub const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(3600);
