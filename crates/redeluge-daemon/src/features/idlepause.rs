// SPDX-License-Identifier: GPL-3.0-or-later
//! Pausing a download that is not getting anywhere, so the queue can move.
//!
//! A torrent that holds a slot in the active queue and transfers nothing is
//! costing another torrent its turn. libtorrent has half an answer already:
//! `dont_count_slow_torrents` stops such a torrent counting against the active
//! limit, so a queued one starts beside it. What it will not do is stop the
//! idle one, put it away for a while, or tell anybody what it is about to do.
//!
//! That is what this is. The rule is deliberately dull, because a rule that
//! pauses downloads has to be predictable: under a rate for long enough, with
//! something waiting, and never the last one running.
//!
//! Downloads only. A torrent that is seeding and transferring nothing is doing
//! its job by being reachable, and pausing it takes it off the swarm for no
//! gain, which on a private tracker is worse than no gain.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The `idle_pause` key of `core.conf`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,

    /// Bytes per second below which a download counts as idle.
    ///
    /// libtorrent's own default for the same judgement, so a torrent this
    /// calls idle is one its queue would also stop counting.
    #[serde(default = "default_rate")]
    pub inactive_rate: i64,

    /// How long it has to stay under that rate before anything happens.
    ///
    /// Not a formality: a torrent between pieces, or one whose peers have just
    /// gone, is under the rate for a few seconds all the time.
    #[serde(default = "default_grace")]
    pub grace: u64,

    /// How long it is then left paused.
    #[serde(default = "default_pause_for")]
    pub pause_for: u64,

    /// Do nothing unless a torrent is actually waiting for the slot.
    ///
    /// Pausing an idle download when nothing wants its place gains nothing and
    /// costs the chance that a peer turns up.
    #[serde(default = "yes")]
    pub only_when_queued: bool,

    /// Never leave fewer than this many downloads running.
    ///
    /// Without it, a queue of torrents that are all idle pauses every one of
    /// them and nothing ever restarts on its own.
    #[serde(default = "default_min_active")]
    pub min_active: i64,
}

fn default_rate() -> i64 {
    2048
}
fn default_grace() -> u64 {
    300
}
fn default_pause_for() -> u64 {
    3600
}
fn default_min_active() -> i64 {
    1
}
fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            inactive_rate: default_rate(),
            grace: default_grace(),
            pause_for: default_pause_for(),
            only_when_queued: yes(),
            min_active: default_min_active(),
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("idle_pause", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// Bounded, because these come from a client and a zero grace would pause
    /// a torrent the instant it dipped.
    pub fn sane(&self) -> Self {
        Self {
            enabled: self.enabled,
            inactive_rate: self.inactive_rate.clamp(0, 100 * 1024 * 1024),
            grace: self.grace.clamp(10, 86_400),
            pause_for: self.pause_for.clamp(60, 30 * 86_400),
            only_when_queued: self.only_when_queued,
            min_active: self.min_active.max(0),
        }
    }
}

/// What the rule has decided about one torrent, for the interface to show.
///
/// Times are Unix seconds, and zero means "not counting down", which is what
/// every other time field in this API does.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Countdown {
    /// When this torrent first went under the rate. Zero if it is not idle.
    pub idle_since: f64,
    /// When it will be paused, if nothing changes. Zero if it will not be.
    pub pause_at: f64,
    /// When it will be let go again. Zero if it is not paused by this rule.
    pub resume_at: f64,
}

/// Whether a torrent under the rate is due to be paused now.
pub fn due(idle_since: f64, now: f64, grace: u64) -> bool {
    idle_since > 0.0 && now - idle_since >= grace as f64
}

/// When a torrent that went idle at this moment would be paused.
pub fn pause_at(idle_since: f64, grace: u64) -> f64 {
    if idle_since <= 0.0 {
        return 0.0;
    }
    idle_since + grace as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_rule_is_off_and_harmless_until_it_is_turned_on() {
        let settings = Settings::default();
        assert!(!settings.enabled);
        assert!(
            settings.only_when_queued,
            "nothing waiting, nothing to gain"
        );
        assert_eq!(settings.min_active, 1, "never pause the last one running");
    }

    #[test]
    fn a_dip_under_the_rate_is_not_enough_on_its_own() {
        // A torrent between pieces is under the rate for a few seconds all the
        // time. Pausing on that would make the feature unusable.
        let grace = 300;
        assert!(!due(1_000.0, 1_010.0, grace));
        assert!(!due(1_000.0, 1_299.0, grace));
        assert!(due(1_000.0, 1_300.0, grace));
    }

    #[test]
    fn a_torrent_that_is_not_idle_has_no_countdown() {
        assert_eq!(pause_at(0.0, 300), 0.0);
        assert_eq!(pause_at(1_000.0, 300), 1_300.0);
        assert!(!due(0.0, 9_999.0, 1));
    }

    #[test]
    fn values_from_a_client_are_bounded() {
        let settings = Settings {
            grace: 0,
            pause_for: 1,
            inactive_rate: -5,
            min_active: -3,
            ..Settings::default()
        }
        .sane();

        assert_eq!(settings.grace, 10, "a zero grace pauses on the first dip");
        assert_eq!(settings.pause_for, 60);
        assert_eq!(settings.inactive_rate, 0);
        assert_eq!(settings.min_active, 0);
    }

    #[test]
    fn the_stored_shape_round_trips() {
        let stored = json!({"enabled": true, "grace": 60, "pause_for": 900});
        let settings = Settings::from_config(Some(&stored));
        assert!(settings.enabled);
        assert_eq!(settings.grace, 60);
        assert_eq!(settings.pause_for, 900);
        // The rest fall back to the defaults rather than the key being lost.
        assert_eq!(settings.inactive_rate, default_rate());
    }

    #[test]
    fn a_malformed_value_is_the_defaults_rather_than_an_error() {
        let settings = Settings::from_config(Some(&json!({"grace": "soon"})));
        assert_eq!(settings.grace, default_grace());
    }
}
