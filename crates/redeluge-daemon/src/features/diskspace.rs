// SPDX-License-Identifier: GPL-3.0-or-later
//! Stopping downloads before the disk runs out, and starting them again after.
//!
//! A full disk is the one failure that costs more than the download. libtorrent
//! reports the write error, the torrent goes to Error, and the next one does
//! the same a minute later: thirty torrents in Error, none of them resumable
//! until somebody frees space and restarts every one of them by hand. Deluge
//! had the number that would have prevented it — `core.get_free_space` feeds
//! the status bar — and did nothing with it.
//!
//! So this does. Under the floor, every torrent that is still writing is
//! paused; over the ceiling, the ones it paused are let go again. Seeding
//! torrents are never touched, because a seed writes nothing and taking it off
//! the swarm would cost ratio for no gain.
//!
//! Two thresholds rather than one, on purpose. A single one flaps: the rule
//! pauses at 1 GiB, a piece is discarded, free space crosses back over by a
//! megabyte, everything resumes, and it pauses again a second later. The gap
//! between them is what stops that.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// A gibibyte, which is the unit both thresholds are really thought about in.
const GIB: i64 = 1024 * 1024 * 1024;

/// The `disk_space` key of `core.conf`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// On by default, which nothing else here is.
    ///
    /// Every other feature waits to be asked because it does something new:
    /// reaches the network, watches a directory, changes what runs when. This
    /// one only ever declines to write to a disk that has no room, which is
    /// what the operator wanted in every case where it fires.
    #[serde(default = "yes")]
    pub enabled: bool,

    /// Pause everything still writing when free space falls under this.
    #[serde(default = "one_gib")]
    pub min_free: i64,

    /// Let them go again once free space reaches this.
    ///
    /// Above `min_free` by default, so recovering a little room does not start
    /// everything again only to have it stop a moment later.
    #[serde(default = "two_gib")]
    pub resume_free: i64,
}

fn yes() -> bool {
    true
}
fn one_gib() -> i64 {
    GIB
}
fn two_gib() -> i64 {
    2 * GIB
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: yes(),
            min_free: one_gib(),
            resume_free: two_gib(),
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("disk_space", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// Bounded, because these come from a client.
    ///
    /// A ceiling under the floor is the one combination that cannot work: it
    /// would pause and release on the same reading, for ever. It is raised to
    /// the floor rather than refused, so a client that writes only one of the
    /// two numbers still gets a rule that behaves.
    pub fn sane(&self) -> Self {
        let min_free = self.min_free.clamp(0, 1024 * GIB);
        Self {
            enabled: self.enabled,
            min_free,
            resume_free: self.resume_free.clamp(min_free, 1024 * GIB),
        }
    }
}

/// What the rule has to say about one filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Under the floor: pause what is writing here.
    Low,
    /// Over the ceiling: let go of what this rule is holding here.
    Recovered,
    /// Between the two, or not something we could measure. Change nothing.
    Hold,
}

/// Reads one free-space figure against the thresholds.
///
/// A negative figure is `core.get_free_space`'s way of saying it could not
/// tell, which happens when the path has gone: an unplugged disk, an unmounted
/// share. That answers neither question, so it answers neither — pausing on it
/// would stop everything whenever a mount blinked, and releasing on it would
/// start writing again with no idea whether there is room.
pub fn judge(free: i64, settings: &Settings) -> Verdict {
    if free < 0 {
        Verdict::Hold
    } else if free < settings.min_free {
        Verdict::Low
    } else if free >= settings.resume_free {
        Verdict::Recovered
    } else {
        Verdict::Hold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_floor_is_a_gibibyte_and_the_rule_is_on() {
        let settings = Settings::default();
        assert!(settings.enabled);
        assert_eq!(settings.min_free, 1024 * 1024 * 1024);
        assert!(
            settings.resume_free > settings.min_free,
            "one threshold flaps"
        );
    }

    #[test]
    fn it_pauses_under_the_floor_and_releases_over_the_ceiling() {
        let settings = Settings::default();
        assert_eq!(judge(500 * 1024 * 1024, &settings), Verdict::Low);
        assert_eq!(judge(4 * GIB, &settings), Verdict::Recovered);
    }

    #[test]
    fn between_the_two_it_does_nothing() {
        // The whole point of the gap: a torrent paused at 1 GiB is not started
        // again by the 40 MiB that a discarded piece gives back.
        let settings = Settings::default();
        assert_eq!(judge(GIB + 40 * 1024 * 1024, &settings), Verdict::Hold);
    }

    #[test]
    fn a_reading_it_could_not_take_decides_nothing() {
        // -1 is "the path is gone", not "the disk is full".
        assert_eq!(judge(-1, &Settings::default()), Verdict::Hold);
    }

    #[test]
    fn a_ceiling_under_the_floor_is_raised_to_it() {
        let settings = Settings {
            min_free: 4 * GIB,
            resume_free: GIB,
            ..Settings::default()
        }
        .sane();
        assert_eq!(settings.resume_free, 4 * GIB);
        // And then it still decides, rather than pausing and releasing at once.
        assert_eq!(judge(4 * GIB, &settings), Verdict::Recovered);
        assert_eq!(judge(4 * GIB - 1, &settings), Verdict::Low);
    }

    #[test]
    fn a_negative_threshold_becomes_no_threshold() {
        let settings = Settings {
            min_free: -5,
            ..Settings::default()
        }
        .sane();
        assert_eq!(settings.min_free, 0);
        assert_eq!(judge(0, &settings), Verdict::Hold, "nothing is ever low");
    }

    #[test]
    fn the_stored_shape_round_trips() {
        let settings = Settings::from_config(Some(&json!({
            "enabled": false,
            "min_free": 5 * GIB
        })));
        assert!(!settings.enabled);
        assert_eq!(settings.min_free, 5 * GIB);
        assert_eq!(
            settings.resume_free,
            two_gib(),
            "the rest keeps its default"
        );
    }

    #[test]
    fn a_malformed_value_is_the_defaults_rather_than_an_error() {
        let settings = Settings::from_config(Some(&json!({"min_free": "lots"})));
        assert_eq!(settings.min_free, one_gib());
    }
}
