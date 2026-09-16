// SPDX-License-Identifier: GPL-3.0-or-later
//! What each peer has actually done, kept across connections.
//!
//! libtorrent knows what a *connection* has moved and forgets it the moment
//! the connection drops. That is the wrong unit for the two questions somebody
//! actually asks about peers:
//!
//! * **What did this address ever give back?** A peer that takes forty
//!   gibibytes over a week and sends nothing is the one worth knowing about,
//!   and every one of its visits looks unremarkable on its own.
//! * **Which peers carry my content on more than one of my trackers?** That is
//!   cross-seeding, seen from inside. It is neither cheating nor rare — the
//!   bytes are real on both trackers — but it is worth being able to see, not
//!   least to confirm that one's own cross-seeding is working.
//!
//! So a ledger, keyed by address, accumulated from samples and kept on disk.
//! Two things make it honest:
//!
//! Totals are accumulated by **difference**, never copied. libtorrent's
//! per-connection counters reset when a peer reconnects, so a sample smaller
//! than the last one is a new connection and the whole of it is new; a larger
//! one contributes only the increase. Copying the counter would count the same
//! bytes once per sample.
//!
//! What it cannot see is a connection that begins and ends between two
//! samples. Nothing in libtorrent reports a peer's totals as it disconnects —
//! the disconnection alert carries no byte counts — so a sampler is the only
//! mechanism there is, and a peer that connects, takes a megabyte and leaves
//! within fifteen seconds leaves no trace. That bias is in the harmless
//! direction: the peers this exists to find are the ones that stay for hours,
//! and a peer too brief to sample is a peer too brief to have taken anything
//! worth the name. On a local link, where a whole file can move in under a
//! second, it misses almost everything — which is a statement about local
//! links rather than about swarms.
//!
//! And it forgets. Entries not seen for the configured time are dropped, which
//! is what keeps a ledger of a busy library from being a list of every address
//! on the internet. Thirty days by default: long enough to judge a peer, short
//! enough that it is a record of the present rather than a dossier.
//!
//! None of it leaves the daemon. It is what this machine saw on its own
//! connection, for its own operator to look at.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// A day, in seconds.
const DAY: f64 = 86_400.0;

/// The most addresses kept, whatever the time limit says.
///
/// A public torrent can introduce thousands of peers in an afternoon. The
/// oldest go first when this is reached, so the ledger costs a bounded amount
/// of memory no matter what the library does.
pub const CAPACITY: usize = 20_000;

/// The `peers` key of `core.conf`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Off until asked for, like everything else here.
    ///
    /// It is not free: sampling means asking libtorrent for the peer list of
    /// every torrent that has one, every few seconds, and keeping a record per
    /// address. Nobody should pay that for a question they never ask.
    #[serde(default)]
    pub enabled: bool,

    /// How long an address is remembered after it was last seen, in days.
    #[serde(default = "thirty")]
    pub ttl_days: f64,
}

fn thirty() -> f64 {
    30.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_days: thirty(),
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => serde_json::from_value(crate::features::without_nulls(value))
                .unwrap_or_else(|err| {
                    crate::features::warn_malformed("peers", &err.to_string());
                    Self::default()
                }),
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// Bounded, because these come from a client.
    ///
    /// An hour at the bottom rather than zero: a ledger that forgets instantly
    /// is a ledger that answers nothing, and somebody who wants that can turn
    /// the whole thing off. Ten years at the top is the same ceiling the
    /// tracker rules use.
    pub fn sane(&self) -> Self {
        let days = if self.ttl_days.is_finite() {
            self.ttl_days.clamp(1.0 / 24.0, 3_650.0)
        } else {
            thirty()
        };
        Self {
            enabled: self.enabled,
            ttl_days: days,
        }
    }

    pub fn ttl_seconds(&self) -> f64 {
        self.ttl_days * DAY
    }
}

/// What one address has done, across every connection it has made.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Bytes this daemon has sent to the address.
    #[serde(default)]
    pub sent: i64,
    /// Bytes the address has sent this daemon.
    #[serde(default)]
    pub received: i64,
    /// Unix seconds.
    #[serde(default)]
    pub first_seen: f64,
    #[serde(default)]
    pub last_seen: f64,
    /// The client it announced itself as, most recently.
    ///
    /// Not evidence of anything: the string is whatever the peer chose to
    /// send. It is here because it is the only human-readable thing about an
    /// address.
    #[serde(default)]
    pub client: String,

    /// The torrents it has been seen in, by infohash.
    #[serde(default)]
    pub torrents: BTreeSet<String>,

    /// Which of this daemon's contents it carries on more than one torrent.
    ///
    /// Keyed by the content fingerprint, holding the torrents of this daemon
    /// that share it and that this peer was seen in. An entry with two or more
    /// torrents in it is a peer cross-seeding that content alongside us.
    #[serde(default)]
    pub contents: BTreeMap<String, BTreeSet<String>>,

    /// The last per-connection totals seen, by torrent, so the next sample can
    /// be turned into a difference.
    ///
    /// Not part of what anybody reads; kept with the record because it is
    /// meaningless without it and would otherwise have to be a second map with
    /// the same lifetime.
    #[serde(default)]
    pub last_sample: BTreeMap<String, Sample>,
}

/// One connection's counters, as libtorrent last reported them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    #[serde(default)]
    pub sent: i64,
    #[serde(default)]
    pub received: i64,
}

impl Record {
    /// How many of this daemon's contents this peer carries on more than one
    /// torrent: cross-seeding, counted.
    pub fn cross_seeds(&self) -> usize {
        self.contents
            .values()
            .filter(|torrents| torrents.len() > 1)
            .count()
    }

    /// What it gave back for what it took, or `None` when it took nothing.
    ///
    /// `None` rather than zero or infinity, because "never asked for anything"
    /// and "asked and gave nothing" are different facts and the second is the
    /// only one worth reading.
    pub fn ratio(&self) -> Option<f64> {
        if self.sent <= 0 {
            return None;
        }
        Some(self.received as f64 / self.sent as f64)
    }
}

/// The whole ledger.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub peers: BTreeMap<String, Record>,
}

/// One peer as a sample says it is, at one moment, in one torrent.
#[derive(Debug, Clone)]
pub struct Observation {
    pub address: String,
    pub client: String,
    pub torrent: String,
    /// The fingerprint of what that torrent carries, empty when unknown.
    pub content: String,
    /// This connection's counters: sent to the peer, received from it.
    pub sent: i64,
    pub received: i64,
}

impl Ledger {
    /// Folds one sample into the ledger.
    ///
    /// The difference, never the counter: see this module's note on why.
    pub fn observe(&mut self, seen: &Observation, now: f64) {
        let record = self.peers.entry(seen.address.clone()).or_insert_with(|| {
            let mut fresh = Record {
                first_seen: now,
                ..Record::default()
            };
            fresh.client = seen.client.clone();
            fresh
        });

        record.last_seen = now;
        if !seen.client.is_empty() {
            record.client = seen.client.clone();
        }
        record.torrents.insert(seen.torrent.clone());
        if !seen.content.is_empty() {
            record
                .contents
                .entry(seen.content.clone())
                .or_default()
                .insert(seen.torrent.clone());
        }

        let previous = record
            .last_sample
            .get(&seen.torrent)
            .copied()
            .unwrap_or_default();
        record.sent += advanced(previous.sent, seen.sent);
        record.received += advanced(previous.received, seen.received);
        record.last_sample.insert(
            seen.torrent.clone(),
            Sample {
                sent: seen.sent,
                received: seen.received,
            },
        );
    }

    /// Drops addresses nobody has seen for a while, and the oldest beyond the
    /// cap. Answers how many went.
    pub fn forget_old(&mut self, now: f64, ttl_seconds: f64) -> usize {
        let before = self.peers.len();
        self.peers
            .retain(|_, record| now - record.last_seen < ttl_seconds);

        if self.peers.len() > CAPACITY {
            // Oldest first, which is the same order the time limit would have
            // taken them in had it been shorter.
            let mut by_age: Vec<(String, f64)> = self
                .peers
                .iter()
                .map(|(address, record)| (address.clone(), record.last_seen))
                .collect();
            by_age.sort_by(|a, b| a.1.total_cmp(&b.1));
            for (address, _) in by_age.iter().take(self.peers.len() - CAPACITY) {
                self.peers.remove(address);
            }
        }

        before - self.peers.len()
    }

    /// Forgets what each connection had moved, without forgetting the totals.
    ///
    /// Called when the daemon stops: those counters describe connections that
    /// will not exist next time, and keeping them would make the first sample
    /// after a restart contribute nothing.
    pub fn forget_connections(&mut self) {
        for record in self.peers.values_mut() {
            record.last_sample.clear();
        }
    }

    /// The addresses that have taken the most, most first.
    pub fn takers(&self, limit: usize) -> Vec<(&String, &Record)> {
        let mut rows: Vec<(&String, &Record)> = self.peers.iter().collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.1.sent));
        rows.into_iter().take(limit).collect()
    }
}

impl Ledger {
    /// Where it is kept: beside the torrent list, in the state directory.
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("state").join("peers.json")
    }

    /// Reads it back, or starts empty.
    ///
    /// A file that cannot be read is a warning and an empty ledger, never a
    /// refusal to start: this is a record of what peers did, and no torrent
    /// depends on it.
    pub fn load(config_dir: &Path) -> Self {
        let path = Self::path(config_dir);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err,
                    "could not read the peer ledger");
                return Self::default();
            }
        };
        match serde_json::from_str(&text) {
            Ok(ledger) => ledger,
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err,
                    "the peer ledger is malformed, starting a new one");
                Self::default()
            }
        }
    }

    /// Writes it out, through a temporary file.
    ///
    /// The same care the torrent list gets, for the same reason: a daemon
    /// killed mid-write must not leave half a file behind.
    pub fn save(&self, config_dir: &Path) -> std::io::Result<()> {
        let path = Self::path(config_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec(self)?)?;
        std::fs::rename(&temporary, &path)
    }
}

/// How much a counter advanced since the last sample.
///
/// A counter that went backwards is a new connection, and the whole of it is
/// new. A counter that stood still contributes nothing.
fn advanced(previous: i64, current: i64) -> i64 {
    if current < previous {
        current.max(0)
    } else {
        current - previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(address: &str, torrent: &str, sent: i64, received: i64) -> Observation {
        Observation {
            address: address.to_owned(),
            client: "qBittorrent 4.6".to_owned(),
            torrent: torrent.to_owned(),
            content: String::new(),
            sent,
            received,
        }
    }

    #[test]
    fn a_counter_that_climbs_is_counted_once() {
        // The mistake this is written to avoid: adding the counter rather than
        // the difference, which counts the same bytes once per sample and
        // turns a 1 GiB transfer into 40 GiB over a poll cycle.
        let mut ledger = Ledger::default();
        ledger.observe(&seen("1.2.3.4", "abc", 100, 10), 1_000.0);
        ledger.observe(&seen("1.2.3.4", "abc", 400, 40), 1_015.0);
        ledger.observe(&seen("1.2.3.4", "abc", 400, 40), 1_030.0);

        let record = &ledger.peers["1.2.3.4"];
        assert_eq!(record.sent, 400);
        assert_eq!(record.received, 40);
        assert_eq!(record.first_seen, 1_000.0);
        assert_eq!(record.last_seen, 1_030.0);
    }

    #[test]
    fn a_reconnection_starts_a_new_connection_and_keeps_the_total() {
        // libtorrent's counters reset when a peer comes back. Treating the
        // smaller number as a decrease would subtract, and treating it as a
        // continuation would lose everything before the reconnection.
        let mut ledger = Ledger::default();
        ledger.observe(&seen("1.2.3.4", "abc", 1_000, 0), 10.0);
        ledger.observe(&seen("1.2.3.4", "abc", 50, 0), 20.0);

        assert_eq!(ledger.peers["1.2.3.4"].sent, 1_050);
    }

    #[test]
    fn each_torrent_is_counted_separately_for_the_same_address() {
        // One address in two swarms has two connections, each with its own
        // counters. Sharing one "last sample" between them would make each
        // sample look like a reset of the other.
        let mut ledger = Ledger::default();
        ledger.observe(&seen("1.2.3.4", "abc", 500, 0), 10.0);
        ledger.observe(&seen("1.2.3.4", "def", 700, 0), 10.0);
        ledger.observe(&seen("1.2.3.4", "abc", 600, 0), 20.0);
        ledger.observe(&seen("1.2.3.4", "def", 900, 0), 20.0);

        assert_eq!(ledger.peers["1.2.3.4"].sent, 600 + 900);
        assert_eq!(ledger.peers["1.2.3.4"].torrents.len(), 2);
    }

    #[test]
    fn a_peer_on_two_of_our_torrents_for_one_content_is_cross_seeding() {
        let mut ledger = Ledger::default();
        let mut one = seen("1.2.3.4", "abc", 0, 0);
        one.content = "film".to_owned();
        let mut two = seen("1.2.3.4", "def", 0, 0);
        two.content = "film".to_owned();
        // The same content, on two of this daemon's torrents: that is the
        // whole definition, and it takes two different torrents to meet it.
        ledger.observe(&one, 1.0);
        assert_eq!(ledger.peers["1.2.3.4"].cross_seeds(), 0);
        ledger.observe(&two, 2.0);
        assert_eq!(ledger.peers["1.2.3.4"].cross_seeds(), 1);

        // A second sample of the same torrent changes nothing.
        ledger.observe(&two, 3.0);
        assert_eq!(ledger.peers["1.2.3.4"].cross_seeds(), 1);
    }

    #[test]
    fn what_it_gave_back_is_unanswerable_until_it_took_something() {
        let mut ledger = Ledger::default();
        ledger.observe(&seen("1.2.3.4", "abc", 0, 900), 1.0);
        assert_eq!(ledger.peers["1.2.3.4"].ratio(), None, "it took nothing");

        ledger.observe(&seen("5.6.7.8", "abc", 1_000, 250), 1.0);
        assert_eq!(ledger.peers["5.6.7.8"].ratio(), Some(0.25));

        ledger.observe(&seen("9.9.9.9", "abc", 1_000, 0), 1.0);
        assert_eq!(
            ledger.peers["9.9.9.9"].ratio(),
            Some(0.0),
            "took and gave nothing, which is the whole point of the column"
        );
    }

    #[test]
    fn an_address_nobody_has_seen_for_the_limit_is_forgotten() {
        let mut ledger = Ledger::default();
        ledger.observe(&seen("1.2.3.4", "abc", 10, 10), 0.0);
        ledger.observe(&seen("5.6.7.8", "abc", 10, 10), 29.0 * DAY);

        let dropped = ledger.forget_old(30.0 * DAY, 30.0 * DAY);
        assert_eq!(dropped, 1);
        assert!(!ledger.peers.contains_key("1.2.3.4"));
        assert!(ledger.peers.contains_key("5.6.7.8"));
    }

    #[test]
    fn the_connection_counters_are_dropped_on_the_way_out() {
        // They describe connections that will not exist after a restart.
        // Keeping them would make the first sample of each peer contribute
        // nothing, because it would look smaller than what was stored.
        let mut ledger = Ledger::default();
        ledger.observe(&seen("1.2.3.4", "abc", 500, 0), 1.0);
        ledger.forget_connections();
        assert!(ledger.peers["1.2.3.4"].last_sample.is_empty());
        assert_eq!(ledger.peers["1.2.3.4"].sent, 500, "the total stays");

        ledger.observe(&seen("1.2.3.4", "abc", 500, 0), 2.0);
        assert_eq!(ledger.peers["1.2.3.4"].sent, 1_000);
    }

    #[test]
    fn the_settings_are_off_and_bounded() {
        let settings = Settings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.ttl_days, 30.0);

        let settings = Settings {
            enabled: true,
            ttl_days: -1.0,
        }
        .sane();
        assert!(settings.ttl_days > 0.0, "a ledger that forgets at once");

        let settings = Settings {
            enabled: true,
            ttl_days: f64::NAN,
        }
        .sane();
        assert_eq!(settings.ttl_days, 30.0);
        assert_eq!(settings.ttl_seconds(), 30.0 * DAY);
    }

    #[test]
    fn the_stored_shape_round_trips() {
        let mut ledger = Ledger::default();
        ledger.observe(&seen("1.2.3.4", "abc", 10, 20), 5.0);
        let text = serde_json::to_string(&ledger).expect("serialises");
        let again: Ledger = serde_json::from_str(&text).expect("parses");
        assert_eq!(again, ledger);
    }
}
