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

use std::time::{Duration, Instant};

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

        // There is no plugin system, and the interface for one is gone. What
        // is left is the Label plugin's name, because the daemon answers its
        // methods: a client that asks what is enabled and then calls `label.*`
        // must get an answer that agrees with what happens next.
        "web.get_plugins" => Ok(json!({
            "enabled_plugins": ["Label"],
            "available_plugins": ["Label"],
        })),
        "web.get_plugin_info" => Ok(json!({})),
        "web.get_plugin_resources" => Err(ApiError::local("no plugin system")),
        "web.register_event_listener" => web_register_event(call, state, true).await,
        "web.deregister_event_listener" => web_register_event(call, state, false).await,
        "web.get_events" => web_get_events(state).await,

        // Adding a torrent. The dialog uploads or downloads a file, asks about
        // it, and then adds it, so all three steps live here.
        "web.get_torrent_info" => web_get_torrent_info(call, state).await,
        "web.get_magnet_info" => web_get_magnet_info(call).await,
        "web.download_torrent_from_url" => web_download_torrent(call, state).await,
        "web.add_torrents" => web_add_torrents(call, state).await,

        // Two conveniences over the daemon's own calls, in the shapes the
        // front end reads.
        "web.get_torrent_status" => web_get_torrent_status(call, state).await,
        "web.get_torrent_files" => web_get_torrent_files(call, state).await,

        // The connection manager.
        "web.add_host" => web_add_host(call, state).await,
        "web.edit_host" => web_edit_host(call, state).await,
        "web.remove_host" => web_remove_host(call, state).await,
        "web.start_daemon" => Err(ApiError::local(
            "the daemon is not started by the Web UI; it is a service of its own",
        )),
        "web.stop_daemon" => web_stop_daemon(call, state).await,

        // Everything in the daemon's namespaces goes to the daemon, and
        // `label.*` is one of them: Radarr, Sonarr and the rest reach this
        // server rather than the daemon's own port, so a namespace that is not
        // forwarded here is a namespace they cannot call. `redeluge.*` is this
        // fork's own, and the interface calls it from here like any other.
        method
            if method.starts_with("core.")
                || method.starts_with("daemon.")
                || method.starts_with("label.")
                || method.starts_with("redeluge.") =>
        {
            // Writing the configuration changes the rate limits the status bar
            // shows, so what is held about them stops being true here rather
            // than when it happens to expire.
            if method == "core.set_config" {
                state.slow_stats.lock().await.clear();
            }
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

    let peer = request
        .connection_info()
        .realip_remote_addr()
        .unwrap_or("unknown")
        .to_owned();

    // Guessing is the whole attack against a single shared password, so a run
    // of wrong ones has to cost time. Checked before the password is verified:
    // scrypt is deliberately slow, and doing that work for a client that is
    // already over its budget is the denial of service, not the defence.
    let now = std::time::Instant::now();
    if let Err(wait) = state.login_throttle.lock().await.check(&peer, now) {
        tracing::warn!(client = %peer, seconds = wait.as_secs(),
            "web login refused, too many attempts");
        return Err(ApiError::local(format!(
            "too many failed attempts, try again in {} seconds",
            wait.as_secs().max(1)
        )));
    }

    let stored = state.password.read().await.clone();
    if !stored.verify(password) {
        state.login_throttle.lock().await.failed(&peer, now);
        tracing::warn!(client = %peer, "web login failed");
        return Ok(Dispatched {
            result: Json::Bool(false),
            new_session: None,
        });
    }
    state.login_throttle.lock().await.succeeded(&peer);

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

    // A pin, if one is configured for this host. Without it the client
    // encrypts and verifies nothing, which is what the Python client did and
    // is defensible over loopback and nowhere else.
    let mut settings = state.client_settings.clone();
    match pinned_fingerprint(state, &host.id).await {
        Some(sha256) => {
            settings.tls = redeluge_rpc::tls::TlsMode::Pinned { sha256 };
            tracing::debug!(host = %host.host, "pinning the daemon certificate");
        }
        None if !is_loopback(&host.host) => {
            tracing::warn!(
                host = %host.host,
                "connecting to a remote daemon without a pinned certificate;                  set daemon_fingerprints in web.conf to the daemon's own                  sha256, which it prints at startup"
            );
        }
        None => {}
    }

    let client = redeluge_rpc::Client::connect(&host.host, host.port, settings)
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
    // Another daemon has another address, another disk and other limits.
    state.slow_stats.lock().await.clear();
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
                    // Wakes whoever is holding a `web.get_events` open.
                    state.events_ready.notify_waiters();
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
    state.slow_stats.lock().await.clear();
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

/// How long each held answer stays good.
///
/// An external address is the same for days, and asking every two seconds cost
/// a trip through the session thread. Free space moves, but not by anything a
/// status bar has to show within two seconds. The limits are emptied the
/// moment somebody writes the configuration, so their age only matters when
/// another client changes them.
const EXTERNAL_IP_FOR: Duration = Duration::from_secs(60);
const FREE_SPACE_FOR: Duration = Duration::from_secs(15);
const LIMITS_FOR: Duration = Duration::from_secs(30);

/// The held value, if it is still young enough to send.
fn fresh<T>(slot: &Option<(Instant, T)>, ttl: Duration) -> Option<&T> {
    slot.as_ref()
        .filter(|(at, _)| at.elapsed() < ttl)
        .map(|(_, value)| value)
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

    // The three below are asked for only when what is held has gone stale.
    // Each is a round trip on a connection the daemon serves one call at a
    // time, and none of them changes at the rate a poll runs: this is a third
    // of what a poll used to cost, spent on answers that were already known.
    let (mut cached_free, mut cached_ip, mut cached_limits) = {
        let slow = state.slow_stats.lock().await;
        (
            fresh(&slow.free_space, FREE_SPACE_FOR).cloned(),
            fresh(&slow.external_ip, EXTERNAL_IP_FOR).cloned(),
            fresh(&slow.limits, LIMITS_FOR).cloned(),
        )
    };

    if cached_free.is_none() {
        if let Some(free) = call_daemon("core.get_free_space", vec![], state).await {
            cached_free = Some(rencode_to_json(&free));
        }
    }
    if cached_ip.is_none() {
        if let Some(ip) = call_daemon("core.get_external_ip", vec![], state).await {
            cached_ip = Some(rencode_to_json(&ip));
        }
    }
    // The front end reads these three from the core config, which it also
    // fetches separately; supplying them keeps the status bar populated on the
    // first poll rather than after the second.
    if cached_limits.is_none() {
        if let Some(config) = call_daemon("core.get_config", vec![], state).await {
            let mut limits = Map::new();
            for (from, to) in [
                ("max_download_speed", "max_download"),
                ("max_upload_speed", "max_upload"),
                ("max_connections_global", "max_num_connections"),
            ] {
                if let Some(value) = config.get(from) {
                    limits.insert(to.to_owned(), rencode_to_json(value));
                }
            }
            cached_limits = Some(limits);
        }
    }

    {
        let now = Instant::now();
        let mut slow = state.slow_stats.lock().await;
        if let Some(free) = cached_free.clone() {
            slow.free_space = Some((now, free));
        }
        if let Some(ip) = cached_ip.clone() {
            slow.external_ip = Some((now, ip));
        }
        if let Some(limits) = cached_limits.clone() {
            slow.limits = Some((now, limits));
        }
    }

    if let Some(free) = cached_free {
        stats.insert("free_space".to_owned(), free);
    }
    if let Some(ip) = cached_ip {
        stats.insert("external_ip".to_owned(), ip);
    }
    for (key, value) in cached_limits.unwrap_or_default() {
        stats.insert(key, value);
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
    // The theme is read live rather than from the startup snapshot: the
    // interface reads it back here after setting it, and a stale value made
    // the theme combo show the previous choice until the server restarted.
    let theme = current_theme(state).await;
    Ok(json!({
        "theme": theme,
        "base": settings.base,
        "sidebar_show_zero": state.web_config.read().await.boolean("sidebar_show_zero").unwrap_or(false),
        "sidebar_multiple_filters": state.web_config.read().await.boolean("sidebar_multiple_filters").unwrap_or(true),
        "show_session_speed": state.web_config.read().await.boolean("show_session_speed").unwrap_or(false),
        "show_sidebar": state.web_config.read().await.boolean("show_sidebar").unwrap_or(true),
        "first_login": false,
        // Always empty: there is one language. The key stays because a client
        // written against the Python server reads it.
        "language": "",
        "session_timeout": settings.session_timeout.as_secs(),
        // How often the interface polls, in milliseconds. Deluge had this
        // hardcoded in five places in its JavaScript; here it is one setting,
        // so a busy daemon or a slow link can be given a longer interval.
        "poll_interval": poll_interval(state).await,
        "interface": settings.interface,
        "port": settings.port,
        "https": false,
        "default_daemon": settings.default_daemon.clone().unwrap_or_default(),
        // Host id to certificate fingerprint. The connection manager reads and
        // writes this, which is the only way to pin a remote daemon without
        // editing the file by hand.
        "daemon_fingerprints": state
            .web_config
            .read()
            .await
            .get("daemon_fingerprints")
            .cloned()
            .unwrap_or_else(|| json!({})),
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
    // Pairs, not names. The interface loads these straight into a combo box
    // whose store has two fields; a flat list of strings makes ExtJS read each
    // string as a row and take its first character as the value, which is how
    // choosing a theme ended up setting the theme to "g".
    let themes: Vec<Json> = theme_names()
        .into_iter()
        .map(|name| json!([name, capitalise(&name)]))
        .collect();
    Ok(json!(themes))
}

/// How often the interface should poll, in milliseconds.
///
/// Bounded rather than trusted: zero would spin, and an hour would look like
/// the interface had stopped working.
pub async fn poll_interval(state: &SharedState) -> u64 {
    const DEFAULT: u64 = 2000;
    state
        .web_config
        .read()
        .await
        .integer("poll_interval")
        .map(|value| (value as u64).clamp(500, 60_000))
        .unwrap_or(DEFAULT)
}

/// The theme in force: what was configured, if it has a stylesheet.
pub async fn current_theme(state: &SharedState) -> String {
    let configured = state
        .web_config
        .read()
        .await
        .string("theme")
        .unwrap_or(&state.settings.theme)
        .to_owned();

    if crate::assets::contains(&format!("themes/css/xtheme-{configured}.css")) {
        configured
    } else {
        crate::routes::DEFAULT_THEME.to_owned()
    }
}

/// Every theme that has a stylesheet, sorted.
fn theme_names() -> Vec<String> {
    let mut themes: Vec<String> = crate::assets::files()
        .keys()
        .filter_map(|path| path.strip_prefix("themes/css/xtheme-"))
        .filter_map(|name| name.strip_suffix(".css"))
        .map(str::to_owned)
        .collect();
    themes.sort();
    themes
}

/// What the interface shows for a theme: `gray` becomes `Gray`.
fn capitalise(name: &str) -> String {
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

async fn web_set_theme(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let theme = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.set_theme takes a name"))?;

    // A theme with no stylesheet would leave the page asking for a file that
    // does not exist, which is worse than ignoring the request.
    if !crate::assets::contains(&format!("themes/css/xtheme-{theme}.css")) {
        return Err(ApiError::local(format!("no such theme: {theme}")));
    }

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

/// How long `web.get_events` waits for something to report.
///
/// Comfortably inside the browser's own request timeout, which is thirty
/// seconds in the Ext JS the front end is built on: a poll that the browser
/// gives up on first is a poll that looks like a failure.
const EVENT_HOLD: std::time::Duration = std::time::Duration::from_secs(25);

/// The events that have arrived since the last poll.
///
/// A long poll. The front end asks again the instant it is answered, which is
/// the right shape for an endpoint that blocks and a busy loop for one that
/// does not: answering empty straight away produced about twenty requests a
/// second, for as long as a tab was open, on a daemon that had nothing to say.
///
/// So the answer is held until an event arrives or the hold expires. An event
/// still reaches the browser as soon as the daemon reports it.
async fn web_get_events(state: &SharedState) -> ApiResult {
    // Registered before the queue is looked at, so an event that arrives
    // between the two is not missed: `notify_waiters` only wakes what is
    // already waiting.
    let woken = state.events_ready.notified();
    tokio::pin!(woken);
    woken.as_mut().enable();

    let mut events = state.events.lock().await.drain();
    if events.is_empty() {
        // Whether this returns because an event arrived or because the hold
        // ran out, the answer is the same: whatever is queued, which may be
        // nothing.
        let _ = tokio::time::timeout(EVENT_HOLD, woken).await;
        events = state.events.lock().await.drain();
    }

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

// ------------------------------------------------------------ adding torrents

/// Reads a staged torrent file and describes it for the add dialog.
async fn web_get_torrent_info(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let filename = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.get_torrent_info takes a filename"))?;

    let path = std::path::PathBuf::from(filename);
    // The path comes from the browser. Only files this server staged are
    // readable through it, or an authenticated client could read anything the
    // server can.
    if !crate::torrentfile::is_staged(&state.settings.config_dir, &path) {
        return Err(ApiError::local("no such uploaded torrent"));
    }

    match crate::torrentfile::read(&path) {
        Ok(info) => Ok(info.to_json(filename)),
        Err(err) => Err(ApiError::local(err.to_string())),
    }
}

/// Describes a magnet link, which has a name and a hash and no files yet.
async fn web_get_magnet_info(call: &JsonRequest) -> ApiResult {
    let uri = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.get_magnet_info takes a magnet uri"))?;

    crate::torrentfile::magnet_info(uri).map_err(|err| ApiError::local(err.to_string()))
}

/// Fetches a `.torrent` by URL into the staging directory.
///
/// The server fetches it, not the browser: the URL may be reachable only from
/// here, which is the point of the feature.
async fn web_download_torrent(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let url = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.download_torrent_from_url takes a url"))?;
    let cookie = call.params.get(1).and_then(Json::as_str).unwrap_or("");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|err| ApiError::local(err.to_string()))?;

    let mut request = client.get(url);
    if !cookie.is_empty() {
        request = request.header(reqwest::header::COOKIE, cookie);
    }

    let response = request
        .send()
        .await
        .map_err(|err| ApiError::local(format!("could not fetch {url}: {err}")))?;
    if !response.status().is_success() {
        return Err(ApiError::local(format!(
            "could not fetch {url}: the server answered {}",
            response.status()
        )));
    }

    let name = crate::torrentfile::safe_name(
        url.rsplit('/')
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or("download.torrent"),
    );
    let bytes = response
        .bytes()
        .await
        .map_err(|err| ApiError::local(err.to_string()))?;

    // Parsed before it is stored, so a URL that answers with an error page is
    // refused here rather than at the next call.
    crate::torrentfile::parse(&bytes)
        .map_err(|err| ApiError::local(format!("{url} is not a torrent file: {err}")))?;

    let staging = crate::torrentfile::staging_dir(&state.settings.config_dir);
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|err| ApiError::local(err.to_string()))?;
    let path = staging.join(&name);
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|err| ApiError::local(err.to_string()))?;

    Ok(Json::String(path.display().to_string()))
}

/// Adds the torrents the dialog collected.
///
/// Each entry is `{"path": ..., "options": {...}}` where the path is a staged
/// file or a magnet link. Returns true if every one was added, which is what
/// the dialog checks.
async fn web_add_torrents(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let entries = call
        .params
        .first()
        .and_then(Json::as_array)
        .ok_or_else(|| ApiError::local("web.add_torrents takes a list"))?
        .clone();

    let mut all_added = true;
    for entry in entries {
        let path = entry.get("path").and_then(Json::as_str).unwrap_or("");
        let options = entry.get("options").cloned().unwrap_or_else(|| json!({}));
        if path.is_empty() {
            all_added = false;
            continue;
        }

        let outcome = if path.starts_with("magnet:") {
            forward(
                "core.add_torrent_magnet",
                &[Json::String(path.to_owned()), options],
                state,
            )
            .await
        } else {
            let file = std::path::PathBuf::from(path);
            if !crate::torrentfile::is_staged(&state.settings.config_dir, &file) {
                tracing::warn!(
                    path,
                    "refused to add a torrent from outside the staging area"
                );
                all_added = false;
                continue;
            }
            match tokio::fs::read(&file).await {
                Ok(bytes) => {
                    let name = file
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "torrent".to_owned());
                    let added =
                        forward_bytes("core.add_torrent_file", name, bytes, options, state).await;
                    // The staged copy has done its job either way; the daemon
                    // keeps its own copy of what it added.
                    let _ = tokio::fs::remove_file(&file).await;
                    added
                }
                Err(err) => Err(ApiError::local(err.to_string())),
            }
        };

        if let Err(err) = outcome {
            tracing::warn!(path, error = %err.message, "could not add a torrent");
            all_added = false;
        }
    }

    Ok(Json::Bool(all_added))
}

/// `core.add_torrent_file` with the dump as bytes rather than as text.
///
/// rencode carries bytes, so there is no base64 step: the JSON side never sees
/// the file, only this does.
async fn forward_bytes(
    method: &str,
    filename: String,
    dump: Vec<u8>,
    options: Json,
    state: &SharedState,
) -> ApiResult {
    let guard = state.daemon.read().await;
    let connection = guard
        .as_ref()
        .ok_or_else(|| ApiError::remote("not connected to a daemon"))?;

    let args = vec![
        Value::Str(filename),
        Value::Bytes(dump),
        json_to_rencode(&options),
    ];
    match connection.client.call(method, args).await {
        Ok(value) => Ok(rencode_to_json(&value)),
        Err(redeluge_rpc::client::Error::Remote(failure)) => {
            Err(ApiError::remote(failure.to_string()))
        }
        Err(err) => Err(ApiError::remote(err.to_string())),
    }
}

// ------------------------------------------------------------- torrent views

/// One torrent's status, which the details panel asks for by itself.
async fn web_get_torrent_status(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let id = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.get_torrent_status takes a torrent id"))?;
    let keys = call.params.get(1).cloned().unwrap_or_else(|| json!([]));

    forward(
        "core.get_torrent_status",
        &[Json::String(id.to_owned()), keys],
        state,
    )
    .await
}

/// The file tree of a torrent that has been added.
///
/// The same shape as `files_tree` in the add dialog, built from what the
/// daemon reports rather than from the torrent file, because priorities and
/// progress only exist once it is running.
async fn web_get_torrent_files(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let id = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.get_torrent_files takes a torrent id"))?;

    let status = forward(
        "core.get_torrent_status",
        &[
            Json::String(id.to_owned()),
            json!(["files", "file_progress", "file_priorities"]),
        ],
        state,
    )
    .await?;

    let files = status
        .get("files")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    let progress = status
        .get("file_progress")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    let priorities = status
        .get("file_priorities")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();

    let mut root = serde_json::Map::new();
    for file in &files {
        let path = file.get("path").and_then(Json::as_str).unwrap_or_default();
        let index = file.get("index").and_then(Json::as_u64).unwrap_or(0) as usize;
        let size = file.get("size").and_then(Json::as_i64).unwrap_or(0);
        if path.is_empty() {
            continue;
        }
        insert_file(
            &mut root,
            path,
            index,
            size,
            progress.get(index).and_then(Json::as_f64).unwrap_or(0.0),
            priorities.get(index).and_then(Json::as_i64).unwrap_or(4),
        );
    }

    Ok(json!({ "contents": Json::Object(root) }))
}

fn insert_file(
    into: &mut serde_json::Map<String, Json>,
    path: &str,
    index: usize,
    size: i64,
    progress: f64,
    priority: i64,
) {
    let (head, rest) = match path.split_once('/') {
        Some((head, rest)) => (head, Some(rest)),
        None => (path, None),
    };
    if head.is_empty() {
        return;
    }

    match rest {
        None => {
            into.insert(
                head.to_owned(),
                json!({
                    "type": "file",
                    "index": index,
                    "size": size,
                    "progress": progress,
                    "priority": priority,
                    "path": path,
                }),
            );
        }
        Some(rest) => {
            let entry = into
                .entry(head.to_owned())
                .or_insert_with(|| json!({"type": "dir", "contents": {}, "size": 0}));
            if entry.get("contents").is_none() {
                *entry = json!({"type": "dir", "contents": {}, "size": 0});
            }
            if let Some(total) = entry.get("size").and_then(Json::as_i64) {
                entry["size"] = json!(total + size);
            }
            if let Some(Json::Object(contents)) = entry.get_mut("contents") {
                insert_file(contents, rest, index, size, progress, priority);
            }
        }
    }
}

// -------------------------------------------------------- connection manager

async fn web_add_host(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let host = call.params.first().and_then(Json::as_str).unwrap_or("");
    let port = call.params.get(1).and_then(Json::as_u64).unwrap_or(58846);
    let username = call.params.get(2).and_then(Json::as_str).unwrap_or("");
    let password = call.params.get(3).and_then(Json::as_str).unwrap_or("");

    if host.is_empty() {
        return Err(ApiError::local("a host is required"));
    }
    let port = u16::try_from(port).map_err(|_| ApiError::local("a port is 1 to 65535"))?;

    let entry = crate::state::Host {
        id: crate::hostlist::new_id(),
        host: host.to_owned(),
        port,
        username: username.to_owned(),
        password: password.to_owned(),
    };
    let id = entry.id.clone();

    {
        let mut hosts = state.hosts.write().await;
        if hosts
            .iter()
            .any(|existing| existing.host == entry.host && existing.port == entry.port)
        {
            return Err(ApiError::local("that host is already in the list"));
        }
        hosts.push(entry);
    }
    save_hosts(state).await?;

    // Deluge answers [success, id] here, which is what the dialog reads.
    Ok(json!([true, id]))
}

async fn web_edit_host(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let id = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.edit_host takes a host id"))?;
    let host = call.params.get(1).and_then(Json::as_str).unwrap_or("");
    let port = call.params.get(2).and_then(Json::as_u64).unwrap_or(58846);
    let username = call.params.get(3).and_then(Json::as_str).unwrap_or("");
    let password = call.params.get(4).and_then(Json::as_str).unwrap_or("");
    let port = u16::try_from(port).map_err(|_| ApiError::local("a port is 1 to 65535"))?;

    {
        let mut hosts = state.hosts.write().await;
        let Some(entry) = hosts.iter_mut().find(|entry| entry.id == id) else {
            return Err(ApiError::local("no such host"));
        };
        entry.host = host.to_owned();
        entry.port = port;
        entry.username = username.to_owned();
        entry.password = password.to_owned();
    }
    save_hosts(state).await?;
    Ok(Json::Bool(true))
}

async fn web_remove_host(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let id = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.remove_host takes a host id"))?;

    let removed = {
        let mut hosts = state.hosts.write().await;
        let before = hosts.len();
        hosts.retain(|entry| entry.id != id);
        hosts.len() != before
    };
    if !removed {
        return Err(ApiError::local("no such host"));
    }

    // A connection to the host that has just been removed would otherwise
    // stay up with nothing naming it.
    {
        let mut daemon = state.daemon.write().await;
        if daemon.as_ref().is_some_and(|c| c.host_id == id) {
            *daemon = None;
        }
    }
    save_hosts(state).await?;
    Ok(Json::Bool(true))
}

/// Asks a daemon to shut down. Only one this server knows about.
async fn web_stop_daemon(call: &JsonRequest, state: &SharedState) -> ApiResult {
    let id = call
        .params
        .first()
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::local("web.stop_daemon takes a host id"))?;

    let connected_to = state
        .daemon
        .read()
        .await
        .as_ref()
        .map(|connection| connection.host_id.clone());
    if connected_to.as_deref() != Some(id) {
        return Err(ApiError::local("not connected to that daemon"));
    }

    forward("daemon.shutdown", &[], state).await?;
    *state.daemon.write().await = None;
    Ok(Json::Bool(true))
}

async fn save_hosts(state: &SharedState) -> Result<(), ApiError> {
    let hosts = state.hosts.read().await.clone();
    crate::hostlist::save(&state.settings.config_dir, &hosts)
        .await
        .map_err(|err| ApiError::local(format!("could not write hostlist.conf: {err}")))
}

/// The pinned fingerprint for a host, from `web.conf`.
///
/// `daemon_fingerprints` is an object of host id to lower-case hex sha256. The
/// daemon prints its own fingerprint at startup, which is where the value
/// comes from.
async fn pinned_fingerprint(state: &SharedState, host_id: &str) -> Option<[u8; 32]> {
    let config = state.web_config.read().await;
    let raw = config
        .settings
        .get("daemon_fingerprints")?
        .get(host_id)?
        .as_str()?
        .trim()
        .replace(':', "")
        .to_lowercase();

    let bytes = hex::decode(&raw).ok()?;
    if bytes.len() != 32 {
        tracing::warn!(
            host_id,
            "the pinned fingerprint is not a sha256, ignoring it"
        );
        return None;
    }
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(&bytes);
    Some(sha256)
}

/// Whether an address is the machine this is running on.
fn is_loopback(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

/// Whether a request carries a live admin session.
///
/// `POST /upload` is not a JSON-RPC call and so does not go through `dispatch`,
/// but it writes files and must not be open.
pub async fn is_authenticated(request: &HttpRequest, state: &SharedState) -> bool {
    current_session(request, state)
        .await
        .is_some_and(|(_, level)| level >= AUTH_LEVEL_ADMIN)
}

/// Everything answered here rather than by the daemon.
pub const LOCAL_METHODS: &[&str] = &[
    "auth.change_password",
    "auth.check_session",
    "auth.delete_session",
    "auth.login",
    "system.listMethods",
    "web.add_host",
    "web.add_torrents",
    "web.connect",
    "web.connected",
    "web.deregister_event_listener",
    "web.disconnect",
    "web.download_torrent_from_url",
    "web.edit_host",
    "web.get_config",
    "web.get_events",
    "web.get_host_status",
    "web.get_hosts",
    "web.get_languages",
    "web.get_magnet_info",
    "web.get_plugin_info",
    "web.get_plugins",
    "web.get_themes",
    "web.get_torrent_files",
    "web.get_torrent_info",
    "web.get_torrent_status",
    "web.register_event_listener",
    "web.remove_host",
    "web.set_config",
    "web.set_theme",
    "web.start_daemon",
    "web.stop_daemon",
    "web.update_ui",
    "webutils.get_languages",
    "webutils.get_themes",
];

/// How long a session lives when `web.conf` does not say.
pub const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(3600);
