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
    /// What each browser session was last told about the torrents.
    pub baselines: Mutex<Baselines>,
    /// How many passwords may be verified at once.
    ///
    /// scrypt is deliberately slow and deliberately hungry: the parameters
    /// here cost about a seventh of a second and thirty-two mebibytes per
    /// attempt. Ten of those at once — which is what ten worker threads
    /// answering a flood of logins would do — is a third of a gigabyte and a
    /// server that answers nothing else while it lasts. The per-address
    /// throttle bounds one client; this bounds every client at once.
    pub verifications: tokio::sync::Semaphore,
    /// The daemon's method list, which does not change while it is connected.
    ///
    /// `system.listMethods` is answered before a session exists — the front
    /// end asks for it on the way to the login window — and answering it used
    /// to mean a round trip to the daemon. Cleared when the connection is.
    pub daemon_methods: Mutex<Option<serde_json::Value>>,
}

/// How many passwords may be verified at the same time.
pub const VERIFICATIONS_AT_ONCE: usize = 2;

/// What one browser session has already been sent.
///
/// The torrent list is almost entirely the same from one poll to the next: a
/// few speeds move, a progress bar advances, and the other thirty-odd fields
/// of every torrent are the same bytes they were two seconds ago. On a library
/// of five thousand that is four megabytes of JSON, twice a second, to say
/// that almost nothing happened.
///
/// So the answer is compared with the last one and only the differences are
/// sent. This is kept here rather than in the daemon because this is the hop
/// that costs: the daemon is usually on the same machine, and the browser is
/// usually not.
#[derive(Debug)]
pub struct Baseline {
    /// Bumped on every answer. The browser sends back the one it holds, and a
    /// mismatch — a reload, a dropped answer, a second tab — means it cannot
    /// apply a difference and is sent the whole list instead.
    pub epoch: u64,
    /// What was asked for. A client that changes its columns or its filters is
    /// asking a different question, and the difference between two different
    /// questions is not a difference.
    pub keys: serde_json::Value,
    pub filters: serde_json::Value,
    /// The torrents as they were last sent, by id.
    pub torrents: std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>>,
}

/// The baselines, by session id.
///
/// Bounded: a browser session is cheap to make and each of these is as big as
/// the library. Four is more than the tabs anybody has open, and the one
/// dropped is the one whose session has been quiet longest.
#[derive(Debug, Default)]
pub struct Baselines {
    entries: std::collections::HashMap<String, (Instant, Baseline)>,
}

/// How many sessions may hold one at a time.
const MAX_BASELINES: usize = 4;

impl Baselines {
    pub fn get(&mut self, session: &str) -> Option<&mut Baseline> {
        self.entries.get_mut(session).map(|(seen, baseline)| {
            *seen = Instant::now();
            baseline
        })
    }

    pub fn put(&mut self, session: &str, baseline: Baseline) {
        self.entries
            .insert(session.to_owned(), (Instant::now(), baseline));
        while self.entries.len() > MAX_BASELINES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (seen, _))| *seen)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    /// Forgets the sessions that are gone, so a server that has been up for a
    /// month is not holding a copy of the library per session it ever had.
    pub fn keep_only(&mut self, live: &dyn Fn(&str) -> bool) {
        self.entries.retain(|id, _| live(id));
    }

    pub fn forget(&mut self, session: &str) {
        self.entries.remove(session);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
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
    /// Whether this daemon answers `redeluge.update_ui`, once we have asked.
    ///
    /// `Unknown` until the first poll, which asks; after that a poll is one
    /// call rather than three. Held on the connection rather than on the
    /// server, so connecting to a different daemon asks again — the other end
    /// may be a Deluge daemon, or an older build of this one, and neither
    /// knows the call.
    pub combined_poll: CombinedPoll,
}

/// Whether the daemon on the other end can answer a whole poll at once.
#[derive(Debug)]
pub struct CombinedPoll(std::sync::atomic::AtomicU8);

impl Default for CombinedPoll {
    fn default() -> Self {
        Self(std::sync::atomic::AtomicU8::new(Self::UNKNOWN))
    }
}

impl CombinedPoll {
    const UNKNOWN: u8 = 0;
    const YES: u8 = 1;
    const NO: u8 = 2;

    /// What the last answer was, or nothing if it has not been asked.
    pub fn known(&self) -> Option<bool> {
        match self.0.load(std::sync::atomic::Ordering::Relaxed) {
            Self::YES => Some(true),
            Self::NO => Some(false),
            _ => None,
        }
    }

    pub fn set(&self, answers: bool) {
        self.0.store(
            if answers { Self::YES } else { Self::NO },
            std::sync::atomic::Ordering::Relaxed,
        );
    }
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

#[cfg(test)]
mod baseline_tests {
    use super::*;

    fn baseline(epoch: u64) -> Baseline {
        Baseline {
            epoch,
            keys: serde_json::json!(["name"]),
            filters: serde_json::json!({}),
            torrents: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn what_a_session_was_told_comes_back_to_it() {
        let mut baselines = Baselines::default();
        baselines.put("session-a", baseline(7));
        assert_eq!(baselines.get("session-a").map(|it| it.epoch), Some(7));
        assert!(baselines.get("session-b").is_none());
    }

    #[test]
    fn only_a_few_sessions_may_hold_one_at_a_time() {
        // Each of these is as big as the library, and a browser session is
        // cheap to make. The one dropped is the one nobody has polled with
        // for longest, which is the tab that was closed.
        let mut baselines = Baselines::default();
        for round in 0..10u64 {
            baselines.put(&format!("session-{round}"), baseline(round));
        }
        assert_eq!(baselines.len(), MAX_BASELINES);
        assert!(
            baselines.get("session-9").is_some(),
            "the most recent session lost its baseline"
        );
        assert!(
            baselines.get("session-0").is_none(),
            "the oldest session kept a copy of the library"
        );
    }

    #[test]
    fn a_session_that_has_gone_takes_its_copy_with_it() {
        let mut baselines = Baselines::default();
        baselines.put("live", baseline(1));
        baselines.put("expired", baseline(1));

        baselines.keep_only(&|id: &str| id == "live");

        assert!(baselines.get("live").is_some());
        assert!(baselines.get("expired").is_none());
    }

    #[test]
    fn forgetting_one_leaves_the_others() {
        let mut baselines = Baselines::default();
        baselines.put("a", baseline(1));
        baselines.put("b", baseline(1));
        baselines.forget("a");
        assert!(baselines.get("a").is_none());
        assert!(baselines.get("b").is_some());
        assert!(!baselines.is_empty());
    }
}
