// SPDX-License-Identifier: GPL-3.0-or-later
//! What the server holds while it runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use redeluge_rpc::{Client, ClientSettings};
use tokio::sync::{Mutex, RwLock};

use crate::auth::{Sessions, StoredPassword};
use crate::config::ConfigFile;

/// A daemon the Web UI can connect to, from `hostlist.conf`.
#[derive(Debug, Clone)]
pub struct Host {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

/// Settings the server needs, gathered from `web.conf` and the environment.
#[derive(Debug, Clone)]
pub struct Settings {
    pub config_dir: PathBuf,
    pub interface: String,
    pub port: u16,
    /// Path prefix, for serving under a reverse proxy subpath. Always ends in `/`.
    pub base: String,
    pub session_timeout: Duration,
    pub theme: String,
    /// Host id to connect to without being asked.
    pub default_daemon: Option<String>,
    /// Version reported to the browser, which shows it in the page title.
    pub version: String,
}

/// Shared across every request.
pub struct AppState {
    pub settings: Settings,
    pub password: RwLock<StoredPassword>,
    pub sessions: Mutex<Sessions>,
    /// Failed-login budgets, one per client address.
    pub login_throttle: Mutex<crate::throttle::Throttle>,
    pub hosts: RwLock<Vec<Host>>,
    /// The daemon connection, once one has been made.
    pub daemon: RwLock<Option<DaemonConnection>>,
    pub client_settings: ClientSettings,
    /// Events the browser asked to be told about, and those that have arrived.
    pub events: Mutex<EventQueue>,
    /// Woken whenever an event reaches the queue.
    ///
    /// `web.get_events` is a long poll: the browser asks, and the answer is
    /// held until there is something to say. Without this it answered empty
    /// straight away and the front end, which re-asks the moment it is
    /// answered, went round about twenty times a second for as long as the tab
    /// was open.
    pub events_ready: tokio::sync::Notify,
    /// Raw `web.conf`, so unknown keys survive a read/write cycle.
    pub web_config: RwLock<ConfigFile>,
    /// Daemon answers the status bar shows and that hardly ever change.
    pub slow_stats: Mutex<SlowStats>,
}

/// The parts of a poll that are not worth asking for every two seconds.
///
/// Each one is a round trip on a connection the daemon serves one call at a
/// time, and two of them go through the session thread, so they were a third
/// of what a poll cost. None of them changes at the rate they were asked for:
/// an external address is the same for days, free space moves slowly, and the
/// three rate limits only change when somebody changes them, which is when
/// this is emptied.
#[derive(Debug, Default)]
pub struct SlowStats {
    pub free_space: Option<(Instant, serde_json::Value)>,
    pub external_ip: Option<(Instant, serde_json::Value)>,
    pub limits: Option<(Instant, serde_json::Map<String, serde_json::Value>)>,
}

impl SlowStats {
    /// Forgets everything, for when the answers may no longer be true: a
    /// different daemon, or a configuration somebody has just written.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// A live connection plus which host it is to.
pub struct DaemonConnection {
    pub host_id: String,
    pub client: Client,
}

/// Events the browser has registered for.
///
/// The Web UI polls `web.get_events`, so events have to be held between polls.
/// The queue is bounded: a browser tab that registers for an event and then
/// stops polling must not grow this without limit.
#[derive(Debug, Default)]
pub struct EventQueue {
    registered: Vec<String>,
    pending: Vec<(String, Vec<serde_json::Value>)>,
}

/// Most events a queue holds before the oldest are dropped.
const MAX_PENDING_EVENTS: usize = 512;

impl EventQueue {
    pub fn register(&mut self, name: &str) {
        if !self.registered.iter().any(|known| known == name) {
            self.registered.push(name.to_owned());
        }
    }

    pub fn deregister(&mut self, name: &str) {
        self.registered.retain(|known| known != name);
        self.pending.retain(|(known, _)| known != name);
    }

    pub fn is_registered(&self, name: &str) -> bool {
        self.registered.iter().any(|known| known == name)
    }

    pub fn push(&mut self, name: String, args: Vec<serde_json::Value>) {
        if !self.is_registered(&name) {
            return;
        }
        if self.pending.len() >= MAX_PENDING_EVENTS {
            self.pending.remove(0);
        }
        self.pending.push((name, args));
    }

    /// Takes everything queued, which is what a poll does.
    pub fn drain(&mut self) -> Vec<(String, Vec<serde_json::Value>)> {
        std::mem::take(&mut self.pending)
    }

    pub fn registered(&self) -> &[String] {
        &self.registered
    }
}

pub type SharedState = Arc<AppState>;
