// SPDX-License-Identifier: GPL-3.0-or-later
//! Downloads that stop getting anywhere.
//!
//! A torrent that has been trying for hours and is not receiving bytes is not
//! slow, it is dead: a magnet nobody seeds, a `.torrent` for content that has
//! left the swarm, a tracker that has stopped answering for it. In the list it
//! looks exactly like one that is between peers, and the only way to tell is
//! to remember how long it has been that way.
//!
//! The rule started on labels and is not a label's business. It lives here so
//! that the same arithmetic serves every scope that wants it — a label today,
//! the whole daemon today, a tracker tomorrow — and so that the thing that
//! deletes files is written once.
//!
//! ## What "stuck" means
//!
//! Not "at zero per cent". That was the first version, and it missed the case
//! people actually complain about: a torrent that got 3% in the first minute
//! and has not moved in a week. What the rule measures is **bytes arriving**,
//! and a torrent that has never had any is the special case where the count
//! has always been zero.
//!
//! Two numbers say when to act:
//!
//! * `hours` — how long without a byte, counted in **time spent trying**.
//! * `max_progress` — how far along a torrent may be and still be taken. Zero,
//!   the default, means only ones that never started; a hundred means any. It
//!   exists because deleting a torrent that is 90% done and stalled is a
//!   different decision from deleting one that never began, and the difference
//!   should be somebody's to make rather than implied.
//!
//! ## Why the clock is not the wall clock
//!
//! libtorrent's `time_since_download` is wall clock: a torrent paused over a
//! weekend comes back three days stale and would be deleted on the first sweep
//! after it resumes, having had no chance at all. `active_time` is the seconds
//! a torrent spent active, so the sweep records the `active_time` at which each
//! torrent's byte count last changed — a [`Mark`] — and measures from there.
//! A torrent sitting in the queue or paused does not age towards deletion.
//!
//! The marks are in memory. A restart clears them, so every torrent gets its
//! clock set again at the first sweep after a start. For a rule that deletes
//! files, erring towards "waits longer than asked" is the direction to err in.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Ten years, the ceiling on the delay. The same one the tracker rules use:
/// anything longer is a typo, and a typo in this direction is harmless.
const MAX_HOURS: f64 = 87_600.0;
const HOUR: f64 = 3600.0;

/// When a torrent last had bytes arrive, in its own active seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    /// What `total_done` was when this mark was taken.
    pub done: i64,
    /// The `active_time` at that moment.
    pub active_time: i64,
}

/// What one scope asks for. Resolved from a label's options or the global
/// settings, so the sweep does not care which said so.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rule {
    pub hours: f64,
    /// Percent, 0 to 100.
    pub max_progress: f64,
    pub remove_data: bool,
}

impl Rule {
    /// The delay in seconds, bounded because it comes from a client.
    ///
    /// A negative delay would read as "already due" and a `NaN` one compares
    /// false against everything, which makes a rule that is on look broken
    /// rather than say why.
    pub fn seconds(&self) -> f64 {
        if self.hours.is_finite() {
            self.hours.clamp(0.0, MAX_HOURS) * HOUR
        } else {
            0.0
        }
    }

    /// How far along a torrent may be and still be taken, as a fraction.
    pub fn ceiling(&self) -> f32 {
        if self.max_progress.is_finite() {
            (self.max_progress.clamp(0.0, 100.0) / 100.0) as f32
        } else {
            0.0
        }
    }
}

/// The `stuck` key of `core.conf`: the rule for torrents no label covers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub hours: f64,
    /// Percent. Zero means only torrents that never started.
    #[serde(default)]
    pub max_progress: f64,
    #[serde(default = "yes")]
    pub remove_data: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            hours: 0.0,
            max_progress: 0.0,
            remove_data: true,
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("stuck", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// The rule, when it is on at all.
    pub fn rule(&self) -> Option<Rule> {
        self.enabled.then_some(Rule {
            hours: self.hours,
            max_progress: self.max_progress,
            remove_data: self.remove_data,
        })
    }
}

/// Whether this torrent has gone as long as the rule allows without a byte.
///
/// `mark` is where its byte count last changed, in its own active seconds.
/// Answering `false` for a torrent with no mark yet is deliberate: the sweep
/// takes the mark on the pass that first sees it, so the first answer after a
/// start is always "wait", and the clock begins there.
pub fn is_stuck(progress: f32, active_time: i64, mark: Option<&Mark>, rule: &Rule) -> bool {
    if progress > rule.ceiling() {
        return false;
    }
    let Some(mark) = mark else {
        return false;
    };
    (active_time - mark.active_time) as f64 >= rule.seconds()
}

/// The mark this torrent should carry after a pass.
///
/// A byte count that moved resets the clock; one that did not keeps whatever
/// it had, which is what makes the measurement cumulative across sweeps rather
/// than a rolling window.
pub fn mark_for(done: i64, active_time: i64, previous: Option<&Mark>) -> Mark {
    match previous {
        Some(mark) if mark.done == done => *mark,
        _ => Mark { done, active_time },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOUR_HOURS: Rule = Rule {
        hours: 4.0,
        max_progress: 0.0,
        remove_data: true,
    };

    #[test]
    fn a_torrent_with_no_mark_yet_is_never_stuck() {
        // The first pass after a start takes the mark. Acting on that pass
        // would delete on the strength of a clock that had not been set.
        assert!(!is_stuck(0.0, 99_999, None, &FOUR_HOURS));
    }

    #[test]
    fn the_clock_runs_in_active_seconds_not_wall_time() {
        let mark = Mark {
            done: 0,
            active_time: 1_000,
        };
        // Three hours of trying: not yet.
        assert!(!is_stuck(0.0, 1_000 + 3 * 3600, Some(&mark), &FOUR_HOURS));
        // Four, exactly.
        assert!(is_stuck(0.0, 1_000 + 4 * 3600, Some(&mark), &FOUR_HOURS));
    }

    #[test]
    fn a_byte_arriving_puts_the_clock_back() {
        let old = Mark {
            done: 0,
            active_time: 1_000,
        };
        // Same byte count: the mark stands, so the clock keeps running.
        let same = mark_for(0, 20_000, Some(&old));
        assert_eq!(same, old);

        // One byte more, and the wait starts again from here.
        let moved = mark_for(1, 20_000, Some(&old));
        assert_eq!(
            moved,
            Mark {
                done: 1,
                active_time: 20_000
            }
        );
        assert!(!is_stuck(0.01, 20_000, Some(&moved), &FOUR_HOURS));
    }

    #[test]
    fn how_far_along_it_got_decides_whether_it_can_be_taken() {
        // The default takes only what never started. A torrent at three per
        // cent has found the swarm once, which is a different thing from never
        // having found it.
        let mark = Mark {
            done: 1,
            active_time: 0,
        };
        assert!(!is_stuck(0.03, 99_999, Some(&mark), &FOUR_HOURS));

        // Raise the ceiling and it is in scope.
        let anything = Rule {
            max_progress: 100.0,
            ..FOUR_HOURS
        };
        assert!(is_stuck(0.03, 99_999, Some(&mark), &anything));
        assert!(is_stuck(0.99, 99_999, Some(&mark), &anything));

        // And a ceiling in between means what it says.
        let barely = Rule {
            max_progress: 5.0,
            ..FOUR_HOURS
        };
        assert!(is_stuck(0.04, 99_999, Some(&mark), &barely));
        assert!(!is_stuck(0.06, 99_999, Some(&mark), &barely));
    }

    #[test]
    fn a_delay_from_a_client_is_bounded() {
        let mark = Mark {
            done: 0,
            active_time: 0,
        };
        let negative = Rule {
            hours: -5.0,
            ..FOUR_HOURS
        };
        assert_eq!(negative.seconds(), 0.0);
        assert!(is_stuck(0.0, 0, Some(&mark), &negative), "zero means now");

        let nonsense = Rule {
            hours: f64::NAN,
            max_progress: f64::NAN,
            remove_data: true,
        };
        assert_eq!(nonsense.seconds(), 0.0);
        assert_eq!(nonsense.ceiling(), 0.0);

        let silly = Rule {
            hours: 1_000_000.0,
            max_progress: 900.0,
            remove_data: true,
        };
        assert_eq!(silly.seconds(), MAX_HOURS * HOUR);
        assert_eq!(silly.ceiling(), 1.0);
    }

    #[test]
    fn the_global_rule_is_off_until_somebody_turns_it_on() {
        assert!(Settings::default().rule().is_none());
        let on = Settings {
            enabled: true,
            hours: 6.0,
            ..Settings::default()
        };
        let rule = on.rule().expect("a rule");
        assert_eq!(rule.seconds(), 6.0 * HOUR);
        // The same default as the label rule's, and for the same reason: a
        // torrent that downloaded nothing has no files worth keeping.
        assert!(rule.remove_data);
    }
}
