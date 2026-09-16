// SPDX-License-Identifier: GPL-3.0-or-later
//! Per-tracker rules: what should happen to the torrents of one tracker.
//!
//! A tracker is not something you create, the way a label is. It is whatever
//! the torrents you added happen to announce to, and the sidebar groups them
//! by it already. This file adds the other half of that grouping: a place to
//! say what the daemon should do with the torrents of one tracker, without
//! having to say it again for every torrent that arrives from it.
//!
//! Three rules, each off until somebody turns it on for a named host:
//!
//! * **Remove** a finished torrent after a delay, with or without its files.
//!   The destructive one, and the one that makes a purge complete rather than
//!   a chore: a tracker that asks for a week of seeding gets a rule of a week
//!   and cleans up after itself.
//! * **Move** the files somewhere after a delay, refusing when the
//!   destination has no room for them.
//! * **Label** the torrents, when they arrive, when they finish, or a set
//!   number of hours after they finish.
//!
//! Every delay is in hours and measured from the moment the download finished.
//! What counts as that moment differs between the destructive rule and the
//! other two, and [`finished_at`] is where that difference is written down.
//!
//! The shape is `label.rs`'s, deliberately: a register keyed by name, holding
//! an options dictionary per entry, under one key of `core.conf`. There is no
//! `tracker.*` RPC namespace, because there was no Deluge plugin of that name
//! to emulate, so `core.get_config` and `core.set_config` are the whole
//! interface and every client already speaks them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Seconds in an hour, which is the unit the rule is written in.
const HOUR: f64 = 3600.0;

/// Ten years, the ceiling on a delay. Anything longer is a typo, and a typo in
/// this direction is harmless, which is the direction to be wrong in.
const MAX_HOURS: f64 = 87_600.0;

/// What a move has to leave free on the disk it is moving to.
///
/// A gibibyte, the same floor the disk-space rule defaults to, and for the
/// same reason: a destination filled to the last byte breaks the next write
/// rather than this one. It is not a setting, because a move that fills a disk
/// is not a preference anybody holds, and the number to tune is the one in
/// Preferences that governs the whole daemon.
pub const MOVE_HEADROOM: i64 = 1024 * 1024 * 1024;

/// What the daemon does to the torrents of one tracker.
///
/// Every field's default is the behaviour of a daemon that has never heard of
/// this feature, with one exception noted on `label_on_add`, so a tracker with
/// an entry and no switch on is a tracker nothing happens to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Options {
    // ------------------------------------------------------------- removing
    /// Remove a finished torrent once its delay is up.
    ///
    /// The switch is separate from the delay for the same reason the label
    /// options have `apply_*` switches: it lets a delay be prepared, looked
    /// at, and left off until somebody means it.
    #[serde(default)]
    pub auto_remove: bool,

    /// How long after it finished downloading, in hours.
    ///
    /// Zero means as soon as the next sweep notices, which is a legitimate
    /// thing to ask for and is why it is not clamped up to something safer.
    #[serde(default)]
    pub remove_after_hours: f64,

    /// Delete the downloaded files along with the torrent.
    ///
    /// Off by default, so the rule as first turned on does what Deluge's
    /// `remove_at_ratio` does: it takes the torrent out of the list and leaves
    /// the download where it is.
    #[serde(default)]
    pub remove_data: bool,

    // ---------------------------------------------------------------- moving
    /// Move the files once the torrent has finished and its delay is up.
    #[serde(default)]
    pub auto_move: bool,

    /// Where to move them. A move to where they already are is not a move.
    #[serde(default)]
    pub move_path: String,

    /// How long after it finished downloading, in hours.
    #[serde(default)]
    pub move_after_hours: f64,

    // --------------------------------------------------------------- labelling
    /// Put these torrents in a label.
    #[serde(default)]
    pub auto_label: bool,

    /// The label to put them in. An empty one is not a rule.
    #[serde(default)]
    pub label: String,

    /// Label a torrent from this tracker when it arrives.
    ///
    /// The one default here that is not "off", and it only matters once
    /// `auto_label` is on: a rule that names a label and says nothing about
    /// when to apply it means the obvious thing rather than nothing at all.
    /// It never overwrites a label a torrent already carries, so a rule set on
    /// a tracker does not take a torrent out of the group somebody filed it
    /// in by hand.
    #[serde(default = "yes")]
    pub label_on_add: bool,

    /// Label it again once it has finished and the delay below is up.
    ///
    /// This one does overwrite, which is the point: it is how a torrent moves
    /// from the label it was downloading under to the one it is kept under.
    #[serde(default)]
    pub label_when_done: bool,

    /// How long after it finished downloading, in hours.
    #[serde(default)]
    pub label_after_hours: f64,
}

fn yes() -> bool {
    true
}

impl Default for Options {
    fn default() -> Self {
        Self {
            auto_remove: false,
            remove_after_hours: 0.0,
            remove_data: false,
            auto_move: false,
            move_path: String::new(),
            move_after_hours: 0.0,
            auto_label: false,
            label: String::new(),
            label_on_add: true,
            label_when_done: false,
            label_after_hours: 0.0,
        }
    }
}

impl Options {
    /// Bounded, because these come from a client.
    ///
    /// A negative delay would be read as "already due" by the comparison
    /// below, and a `NaN` delay compares false against everything, which would
    /// make a rule that is on look broken rather than say why. Paths and
    /// labels are trimmed, because a trailing space in a directory name is a
    /// different directory and nobody ever means it.
    pub fn sane(&self) -> Self {
        Self {
            auto_remove: self.auto_remove,
            remove_after_hours: hours(self.remove_after_hours),
            remove_data: self.remove_data,
            auto_move: self.auto_move,
            move_path: self.move_path.trim().to_owned(),
            move_after_hours: hours(self.move_after_hours),
            auto_label: self.auto_label,
            label: self.label.trim().to_owned(),
            label_on_add: self.label_on_add,
            label_when_done: self.label_when_done,
            label_after_hours: hours(self.label_after_hours),
        }
    }

    /// Whether this entry asks for anything at all.
    ///
    /// A switch with nothing to act on is not a rule: moving to nowhere and
    /// labelling with no label are both entries somebody started and did not
    /// finish, and neither should cost a sweep of the library.
    pub fn acts(&self) -> bool {
        self.removes() || self.moves() || self.labels()
    }

    pub fn removes(&self) -> bool {
        self.auto_remove
    }

    pub fn moves(&self) -> bool {
        self.auto_move && !self.move_path.trim().is_empty()
    }

    pub fn labels(&self) -> bool {
        self.auto_label && !self.label.trim().is_empty()
    }
}

/// A delay from a client, bounded and never `NaN`.
fn hours(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, MAX_HOURS)
    } else {
        0.0
    }
}

/// The `tracker` key of `core.conf`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The trackers somebody has set a rule on, by host.
    ///
    /// The key is the host as the sidebar groups it, which is
    /// [`crate::torrent::tracker_host`]'s answer and not the announce URL: a
    /// rule is set on the row somebody right-clicked, and that row is a host.
    #[serde(default)]
    pub trackers: BTreeMap<String, Options>,
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("tracker", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    pub fn to_json(&self) -> Json {
        serde_json::to_value(self).expect("the settings serialise")
    }

    /// The same settings with every entry bounded.
    pub fn sane(&self) -> Self {
        Self {
            trackers: self
                .trackers
                .iter()
                .map(|(host, options)| (host.clone(), options.sane()))
                .collect(),
        }
    }

    pub fn options(&self, host: &str) -> Option<&Options> {
        self.trackers.get(host)
    }

    /// Whether any tracker asks for anything.
    ///
    /// The sweep asks this before it looks at a single torrent, because the
    /// answer is no on every daemon that has not been configured and the work
    /// behind it is a status sweep of the whole library.
    pub fn any_rule(&self) -> bool {
        self.trackers.values().any(Options::acts)
    }
}

/// Whether a delay measured from this moment is up.
///
/// A completion time of zero is "not known", and nothing is ever due from a
/// time nobody knows: see [`finished_at`] for where that zero comes from.
pub fn due(completed_at: f64, now: f64, hours: f64) -> bool {
    completed_at > 0.0 && now - completed_at >= hours * HOUR
}

/// When a finished torrent counts as having finished, for a given rule.
///
/// libtorrent records a completion time and keeps it in the resume data, so it
/// survives a restart. It is zero for a torrent it never saw finish, which is
/// what adding a torrent over files that were already on disk looks like, and
/// the two kinds of rule answer that case differently on purpose:
///
/// * The **destructive** rule is strict. Deleting on a delay measured from a
///   time nobody knows is the one way this feature could take something
///   unexpectedly, so a torrent with no completion time is never removed. The
///   answer is zero and nothing is ever due.
/// * The **other** rules fall back to the time the torrent was added. Moving
///   files or setting a label is undoable and expected, and a rule that
///   silently skipped every imported torrent would be a rule that looks
///   broken. A torrent that was already complete when it was added did, in
///   every sense that matters here, finish before that.
pub fn finished_at(completed_time: i64, added_time: i64, destructive: bool) -> f64 {
    if completed_time > 0 {
        return completed_time as f64;
    }
    if destructive || added_time <= 0 {
        return 0.0;
    }
    added_time as f64
}

/// When a rule measured from this moment comes due.
///
/// Zero when it never does, which is what every other time in this API uses
/// for "not counting down": the interface draws nothing rather than a date in
/// 1970. Reported in the torrent's status so the list can say what is about to
/// happen to a torrent, which for the destructive rule is the difference
/// between a feature you dare leave on and one you do not.
pub fn due_at(finished_at: f64, hours: f64) -> f64 {
    if finished_at <= 0.0 {
        return 0.0;
    }
    finished_at + hours * HOUR
}

/// Whether there is room at the destination to move this torrent.
///
/// `free` is what the destination's filesystem reports, and a negative figure
/// is `core.get_free_space`'s way of saying it could not be measured, which is
/// refused rather than guessed at. `same_filesystem` is the case that matters
/// most in practice and is easiest to get wrong: somebody points the rule at a
/// subdirectory of where the files already are, the move is a rename, and no
/// bytes are written at all. Demanding a spare copy's worth of room there
/// would refuse every move that was never going to cost anything.
pub fn room_to_move(free: i64, size: i64, same_filesystem: bool) -> bool {
    if same_filesystem {
        return true;
    }
    if free < 0 {
        return false;
    }
    free.saturating_sub(size.max(0)) >= MOVE_HEADROOM
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_daemon_nobody_has_configured_does_nothing() {
        let settings = Settings::default();
        assert!(!settings.any_rule());
        assert!(settings.options("example.com").is_none());

        // And an entry that exists but says nothing is still nothing: the
        // interface writes one the moment somebody opens the window.
        let settings = Settings::from_config(Some(&json!({"trackers": {"example.com": {}}})));
        assert!(!settings.any_rule(), "an empty entry is not a rule");
        assert!(!settings.options("example.com").expect("the entry").acts());
    }

    #[test]
    fn a_switch_with_nothing_to_act_on_is_not_a_rule() {
        // Somebody ticks "move them somewhere else", does not say where, and
        // presses OK. Nothing should be moved, and the library should not be
        // swept every minute on account of it.
        let half_done = Options {
            auto_move: true,
            auto_label: true,
            ..Options::default()
        };
        assert!(!half_done.moves());
        assert!(!half_done.labels());
        assert!(!half_done.acts());

        let complete = Options {
            auto_move: true,
            move_path: "/done".into(),
            auto_label: true,
            label: "films".into(),
            ..Options::default()
        };
        assert!(complete.moves());
        assert!(complete.labels());
        assert!(complete.acts());
    }

    #[test]
    fn a_label_rule_applies_on_arrival_unless_it_is_told_otherwise() {
        // The one default here that is not "off": naming a label and saying
        // nothing about when means the obvious thing.
        let options = Options::default();
        assert!(options.label_on_add);
        assert!(!options.label_when_done);

        // And it is still not a rule until the switch above it is on.
        assert!(!options.labels());
    }

    #[test]
    fn what_counts_as_finished_differs_for_the_destructive_rule() {
        // libtorrent knows when it finished: both kinds of rule use that.
        assert_eq!(finished_at(1_000, 500, true), 1_000.0);
        assert_eq!(finished_at(1_000, 500, false), 1_000.0);

        // It does not: only the rules that can be undone fall back to the add.
        assert_eq!(
            finished_at(0, 500, true),
            0.0,
            "nothing is deleted on a delay measured from a time nobody knows"
        );
        assert_eq!(finished_at(0, 500, false), 500.0);

        // Neither time known is nothing to measure from, whatever the rule.
        assert_eq!(finished_at(0, 0, false), 0.0);
    }

    #[test]
    fn a_move_is_refused_when_the_destination_has_no_room() {
        let gib = MOVE_HEADROOM;

        // Room for the files and a gibibyte after them.
        assert!(room_to_move(3 * gib, gib, false));
        assert!(room_to_move(2 * gib, gib, false));
        // A gibibyte short of that.
        assert!(!room_to_move(2 * gib - 1, gib, false));
        assert!(!room_to_move(gib, gib, false));

        // The case the rule exists for: a destination that is a different
        // disk under a directory that looks like the same one.
        assert!(!room_to_move(100, 10 * gib, false));

        // And the case that must not be refused: the same filesystem, where
        // the move is a rename and writes nothing.
        assert!(room_to_move(0, 10 * gib, true));

        // A destination that could not be measured is refused rather than
        // guessed at.
        assert!(!room_to_move(-1, 0, false));
        assert!(room_to_move(-1, 0, true));
    }

    #[test]
    fn the_delay_is_measured_from_the_moment_it_finished() {
        let finished = 1_000_000.0;
        let day = 24.0;

        assert!(!due(finished, finished, day));
        assert!(!due(finished, finished + 23.0 * HOUR, day));
        assert!(due(finished, finished + 24.0 * HOUR, day));
        assert!(due(finished, finished + 400.0 * HOUR, day));

        // And the same moment, as the interface counts down to it.
        assert_eq!(due_at(finished, day), finished + 24.0 * HOUR);
    }

    #[test]
    fn a_torrent_whose_completion_time_is_unknown_is_never_removed() {
        // The one way this could delete something nobody asked it to. A
        // torrent added over files that were already there has no completion
        // time, and "zero seconds since the epoch" is not one.
        assert!(!due(0.0, 9_999_999.0, 0.0));
        assert!(!due(-1.0, 9_999_999.0, 0.0));
        // And nothing to count down to, rather than a date in 1970.
        assert_eq!(due_at(0.0, 24.0), 0.0);
    }

    #[test]
    fn a_delay_of_zero_means_the_next_sweep() {
        // Legitimate, and the reason the delay is not clamped up to something
        // that feels safer: a tracker that wants nothing kept is entitled to
        // say so.
        let finished = 1_000_000.0;
        assert!(due(finished, finished, 0.0));
    }

    #[test]
    fn values_from_a_client_are_bounded() {
        let options = Options {
            auto_remove: true,
            remove_after_hours: -5.0,
            remove_data: true,
            ..Options::default()
        }
        .sane();
        assert_eq!(options.remove_after_hours, 0.0, "a negative delay is now");
        assert!(options.remove_data, "the rest is left as it was");

        let options = Options {
            remove_after_hours: f64::NAN,
            ..Options::default()
        }
        .sane();
        assert_eq!(
            options.remove_after_hours, 0.0,
            "NaN compares false against everything, which looks like a bug"
        );

        let options = Options {
            remove_after_hours: 1_000_000.0,
            move_after_hours: f64::INFINITY,
            label_after_hours: -1.0,
            ..Options::default()
        }
        .sane();
        assert_eq!(options.remove_after_hours, MAX_HOURS);
        assert_eq!(options.move_after_hours, 0.0, "infinity is not a delay");
        assert_eq!(options.label_after_hours, 0.0);

        // A trailing space in a directory name is a different directory, and
        // nobody has ever meant one.
        let options = Options {
            move_path: "  /done  ".into(),
            label: " films ".into(),
            ..Options::default()
        }
        .sane();
        assert_eq!(options.move_path, "/done");
        assert_eq!(options.label, "films");
    }

    #[test]
    fn the_stored_shape_round_trips() {
        let stored = json!({
            "trackers": {
                "example.com": {
                    "auto_remove": true,
                    "remove_after_hours": 48,
                    "remove_data": true,
                    "auto_move": true,
                    "move_path": "/archive",
                    "move_after_hours": 1,
                    "auto_label": true,
                    "label": "films",
                    "label_when_done": true
                },
                "other.example.org": {"auto_remove": false}
            }
        });
        let settings = Settings::from_config(Some(&stored)).sane();
        assert!(settings.any_rule());

        let rule = settings.options("example.com").expect("the entry");
        assert!(rule.auto_remove);
        // Written as an integer by a client that had no reason to write 48.0.
        assert_eq!(rule.remove_after_hours, 48.0);
        assert!(rule.remove_data);
        assert_eq!(rule.move_path, "/archive");
        assert_eq!(rule.move_after_hours, 1.0);
        assert_eq!(rule.label, "films");
        assert!(rule.label_when_done);
        assert!(
            rule.label_on_add,
            "a key the stored entry does not carry is its default, not false"
        );

        let other = settings.options("other.example.org").expect("the entry");
        assert!(!other.acts());
        // The defaults fill the rest in rather than the entry being lost.
        assert_eq!(other.remove_after_hours, 0.0);

        // And it survives a trip through the configuration file's shape.
        let again = Settings::from_config(Some(&settings.to_json()));
        assert_eq!(again.sane(), settings);
    }

    #[test]
    fn a_malformed_value_is_the_defaults_rather_than_an_error() {
        let settings = Settings::from_config(Some(&json!({"trackers": "not a map"})));
        assert!(settings.trackers.is_empty());
        assert!(!settings.any_rule());

        // A null where a number belongs is what a blank number field in a
        // browser used to send; `without_nulls` takes it out and the default
        // stands in, rather than the whole tracker list being dropped.
        let settings = Settings::from_config(Some(&json!({
            "trackers": {"example.com": {"auto_remove": true, "remove_after_hours": null}}
        })));
        assert!(settings.any_rule());
        assert_eq!(
            settings
                .options("example.com")
                .expect("the entry")
                .remove_after_hours,
            0.0
        );
    }
}
