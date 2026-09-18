// SPDX-License-Identifier: GPL-3.0-or-later
//! Labels: the register of them, and the options each one carries.
//!
//! A torrent's label is a torrent option, kept with the torrent in
//! `torrent.rs`, and that is the whole of what a label *is*. This file is the
//! other half: the list of labels that exist even when nothing carries them,
//! and the per-label options the Label plugin let you set.
//!
//! Both halves are needed for a reason that has nothing to do with the
//! interface. Radarr, Sonarr and everything else built on Deluge's API ask the
//! daemon whether the Label plugin is enabled, then call `label.get_labels`,
//! `label.add` and `label.set_torrent`. Deriving the list from the torrents
//! that happen to exist cannot answer that: a label with nothing in it yet is
//! exactly the one those programs are about to use. So the list is stored.
//!
//! The shape is the plugin's own, under the `label` key of `core.conf`, so a
//! client that reads it back recognises what it gets.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// What a label does to the torrents that carry it.
///
/// The plugin's own option set, name for name. The three `apply_*` switches
/// are what make the rest mean anything: a label with `apply_max` off carries
/// speed limits that are remembered and not imposed, which is how the plugin
/// let you prepare a label before using it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Options {
    #[serde(default)]
    pub apply_max: bool,
    #[serde(default = "minus_one_float")]
    pub max_download_speed: f64,
    #[serde(default = "minus_one_float")]
    pub max_upload_speed: f64,
    #[serde(default = "minus_one")]
    pub max_connections: i64,
    #[serde(default = "minus_one")]
    pub max_upload_slots: i64,
    #[serde(default)]
    pub prioritize_first_last: bool,

    #[serde(default)]
    pub apply_queue: bool,
    #[serde(default)]
    pub is_auto_managed: bool,
    #[serde(default)]
    pub stop_at_ratio: bool,
    #[serde(default = "two")]
    pub stop_ratio: f64,
    #[serde(default)]
    pub remove_at_ratio: bool,

    #[serde(default)]
    pub apply_move_completed: bool,
    #[serde(default)]
    pub move_completed: bool,
    #[serde(default)]
    pub move_completed_path: String,

    /// Throw away a download that never starts.
    ///
    /// The one rule here that deletes something nobody pressed a button for,
    /// so it is off until somebody turns it on for a label by name, and the
    /// state it acts on is deliberately narrow: nothing downloaded at all,
    /// while the torrent was actually trying. See [`Options::removes_stuck`].
    #[serde(default)]
    pub apply_stuck: bool,

    /// How long it has to have been trying, in hours.
    ///
    /// Counted in time the torrent spent active, not on the wall clock: a
    /// torrent that sat in the queue for a day, or that somebody paused over
    /// the weekend, has not been failing for a day.
    #[serde(default)]
    pub stuck_hours: f64,

    /// How far along a torrent of this label may be and still be taken.
    ///
    /// Percent. Zero, the default, is the rule this started as: only torrents
    /// that never downloaded a byte. A hundred takes anything that has stalled
    /// for the delay, whatever it got to first.
    #[serde(default)]
    pub stuck_max_progress: f64,

    /// Delete its files along with it.
    ///
    /// On by default, which the tracker rule's equivalent is not, and the
    /// difference is the point: that one removes torrents that finished, where
    /// the files are the whole reason they exist. This one removes torrents
    /// that downloaded nothing, where "its files" is an empty directory and
    /// whatever libtorrent preallocated. Leaving those behind is how a
    /// download directory fills with the skeletons of torrents that never ran.
    #[serde(default = "yes")]
    pub stuck_remove_data: bool,

    /// Keep these torrents out of the list until somebody asks for them.
    ///
    /// Not something the label does to its torrents: nothing is paused, moved
    /// or limited by it, and every other client still sees everything. It is a
    /// view rule, stored here because it belongs to the label rather than to
    /// one browser, and because a person with three thousand torrents wants
    /// the noisy label out of the way on every machine they open.
    #[serde(default)]
    pub hide_by_default: bool,

    /// Deluge's rule for labelling a torrent by its tracker. Stored and
    /// reported so a client that sets it does not lose it; nothing acts on it
    /// yet, and [`Settings::unapplied`] says so out loud.
    #[serde(default)]
    pub auto_add: bool,
    #[serde(default)]
    pub auto_add_trackers: Vec<String>,
}

fn minus_one() -> i64 {
    -1
}
fn minus_one_float() -> f64 {
    -1.0
}
fn two() -> f64 {
    2.0
}
fn yes() -> bool {
    true
}

impl Default for Options {
    fn default() -> Self {
        Self {
            apply_max: false,
            max_download_speed: -1.0,
            max_upload_speed: -1.0,
            max_connections: -1,
            max_upload_slots: -1,
            prioritize_first_last: false,
            apply_queue: false,
            is_auto_managed: false,
            stop_at_ratio: false,
            stop_ratio: 2.0,
            remove_at_ratio: false,
            apply_move_completed: false,
            move_completed: false,
            move_completed_path: String::new(),
            apply_stuck: false,
            stuck_hours: 0.0,
            stuck_max_progress: 0.0,
            stuck_remove_data: true,
            hide_by_default: false,
            auto_add: false,
            auto_add_trackers: Vec::new(),
        }
    }
}

impl Options {
    /// The torrent options this label imposes, as `core.set_torrent_options`
    /// would take them.
    ///
    /// Only the groups whose `apply_*` switch is on. An empty result means the
    /// label names a group and changes nothing about the torrents in it, which
    /// is the common case and is not a mistake.
    pub fn to_torrent_options(&self) -> Vec<(String, Json)> {
        let mut out: Vec<(String, Json)> = Vec::new();

        if self.apply_max {
            out.push((
                "max_download_speed".into(),
                json_number(self.max_download_speed),
            ));
            out.push((
                "max_upload_speed".into(),
                json_number(self.max_upload_speed),
            ));
            out.push(("max_connections".into(), Json::from(self.max_connections)));
            out.push(("max_upload_slots".into(), Json::from(self.max_upload_slots)));
            out.push((
                "prioritize_first_last_pieces".into(),
                Json::Bool(self.prioritize_first_last),
            ));
        }

        if self.apply_queue {
            out.push(("auto_managed".into(), Json::Bool(self.is_auto_managed)));
            out.push(("stop_at_ratio".into(), Json::Bool(self.stop_at_ratio)));
            out.push(("stop_ratio".into(), json_number(self.stop_ratio)));
            out.push(("remove_at_ratio".into(), Json::Bool(self.remove_at_ratio)));
        }

        if self.apply_move_completed {
            out.push(("move_completed".into(), Json::Bool(self.move_completed)));
            out.push((
                "move_completed_path".into(),
                Json::String(self.move_completed_path.clone()),
            ));
        }

        out
    }

    /// Whether this label throws away downloads that never start.
    ///
    /// The switch alone, because zero hours is a legitimate thing to ask for:
    /// it means the next sweep takes anything that has done nothing since it
    /// was started, which is what somebody watching a dead tracker's torrents
    /// pile up actually wants.
    pub fn removes_stuck(&self) -> bool {
        self.apply_stuck
    }

    /// This label's answer to the stuck rule: its own, or none.
    ///
    /// `None` means the torrents of this label are exempt — including from the
    /// daemon's own rule. A label is how somebody says "these are different",
    /// and a rule that overrode them saying it would be a poor rule.
    pub fn stuck_rule(&self) -> Option<super::stuck::Rule> {
        self.apply_stuck.then_some(super::stuck::Rule {
            hours: self.stuck_hours,
            max_progress: self.stuck_max_progress,
            remove_data: self.stuck_remove_data,
        })
    }
}

fn json_number(value: f64) -> Json {
    serde_json::Number::from_f64(value)
        .map(Json::Number)
        .unwrap_or_else(|| Json::from(-1))
}

/// The `label` key of `core.conf`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Every label that exists, by its id, whether or not a torrent carries it.
    #[serde(default)]
    pub labels: BTreeMap<String, Options>,
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("label", &err.to_string());
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

    /// The labels, sorted, which is the order `label.get_labels` answers in.
    pub fn names(&self) -> Vec<String> {
        self.labels.keys().cloned().collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.labels.contains_key(id)
    }

    /// Adds a label. Answers whether it was new.
    pub fn add(&mut self, id: &str) -> bool {
        if self.labels.contains_key(id) {
            return false;
        }
        self.labels.insert(id.to_owned(), Options::default());
        true
    }

    /// Removes a label. Answers whether there was one.
    pub fn remove(&mut self, id: &str) -> bool {
        self.labels.remove(id).is_some()
    }

    pub fn options(&self, id: &str) -> Option<&Options> {
        self.labels.get(id)
    }

    /// Whether any label at all throws away downloads that never start.
    ///
    /// Checked before the sweep walks the library, because nothing configured
    /// is the normal case and a status pass over three thousand torrents once
    /// a minute is not free.
    pub fn any_stuck_rule(&self) -> bool {
        self.labels.values().any(Options::removes_stuck)
    }

    /// The options this build stores and does not yet act on.
    ///
    /// Kept as a function rather than a comment so the answer is checkable: a
    /// setting that is read back exactly as it was written, and does nothing,
    /// is the thing this project has spent its time removing.
    pub fn unapplied() -> &'static [&'static str] {
        &["auto_add", "auto_add_trackers"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_label_with_no_switches_on_imposes_nothing() {
        // The common case: a label names a group and leaves the torrents in it
        // alone. It must not quietly reset their limits to the defaults.
        assert!(Options::default().to_torrent_options().is_empty());
    }

    #[test]
    fn each_group_is_applied_only_when_its_switch_is_on() {
        let mut options = Options {
            apply_max: true,
            max_download_speed: 500.0,
            ..Options::default()
        };
        let applied: Vec<String> = options
            .to_torrent_options()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert!(applied.contains(&"max_download_speed".to_owned()));
        assert!(!applied.contains(&"move_completed".to_owned()));

        options.apply_move_completed = true;
        options.move_completed = true;
        options.move_completed_path = "/done".to_owned();
        let applied: BTreeMap<String, Json> = options.to_torrent_options().into_iter().collect();
        assert_eq!(
            applied.get("move_completed_path"),
            Some(&Json::String("/done".to_owned()))
        );
    }

    #[test]
    fn the_register_holds_a_label_nothing_carries() {
        // The whole reason it exists: Radarr adds a label and only then starts
        // putting torrents in it.
        let mut settings = Settings::default();
        assert!(settings.add("radarr"));
        assert!(!settings.add("radarr"), "adding twice should say so");
        assert_eq!(settings.names(), vec!["radarr".to_owned()]);
        assert!(settings.remove("radarr"));
        assert!(!settings.remove("radarr"));
    }

    #[test]
    fn a_label_hands_its_own_rule_to_the_sweep() {
        // The arithmetic is `stuck.rs`'s and tested there. What a label owes
        // is the three numbers, unchanged.
        let options = Options {
            apply_stuck: true,
            stuck_hours: 4.0,
            stuck_max_progress: 5.0,
            stuck_remove_data: false,
            ..Options::default()
        };
        let rule = options.stuck_rule().expect("a rule");
        assert_eq!(rule.hours, 4.0);
        assert_eq!(rule.max_progress, 5.0);
        assert!(!rule.remove_data);
        assert_eq!(rule.seconds(), 4.0 * 3600.0);
    }

    #[test]
    fn a_label_with_the_rule_off_is_an_exemption_not_a_gap() {
        // `None` is read by the sweep as "these torrents are not the daemon's
        // rule's business either". A label is how somebody says these are
        // different, and this is that sentence in one value.
        assert!(Options::default().stuck_rule().is_none());
    }

    #[test]
    fn the_stuck_rule_takes_the_files_unless_told_not_to() {
        // The opposite default to the tracker rule's, and deliberately: that
        // one removes torrents that finished, where the files are the point.
        // This one removes torrents that downloaded nothing.
        assert!(Options::default().stuck_remove_data);

        // An entry written by an older build has no key for it, and must not
        // read as "keep the files" by accident of serde's bool default.
        let stored = json!({"labels": {"dead": {"apply_stuck": true}}});
        let settings = Settings::from_config(Some(&stored));
        let options = settings.options("dead").expect("the label");
        assert!(options.stuck_remove_data);
    }

    #[test]
    fn the_sweep_is_skipped_when_no_label_asks_for_it() {
        // Behind this check is a status pass over the whole library, once a
        // minute, for a feature almost nobody turns on.
        let mut settings = Settings::default();
        settings.add("films");
        assert!(!settings.any_stuck_rule());

        settings
            .labels
            .get_mut("films")
            .expect("the label")
            .apply_stuck = true;
        assert!(settings.any_stuck_rule());
    }

    #[test]
    fn the_stuck_rule_is_not_a_torrent_option() {
        // It is something the daemon does to the torrent, not a setting the
        // torrent carries. Sending it to `core.set_torrent_options` would be
        // an unknown key at best.
        let options = Options {
            apply_stuck: true,
            stuck_hours: 4.0,
            ..Options::default()
        };
        assert!(options.to_torrent_options().is_empty());
    }

    #[test]
    fn the_plugins_own_config_shape_round_trips() {
        let stored = json!({
            "labels": {
                "films": {
                    "apply_move_completed": true,
                    "move_completed": true,
                    "move_completed_path": "/films"
                }
            }
        });
        let settings = Settings::from_config(Some(&stored));
        let options = settings.options("films").expect("the label");
        assert!(options.apply_move_completed);
        assert_eq!(options.move_completed_path, "/films");
        // Defaults fill in the rest rather than the label being discarded.
        assert_eq!(options.max_download_speed, -1.0);
    }

    #[test]
    fn hiding_a_label_changes_nothing_about_its_torrents() {
        // The point of the option: it decides what a list shows, and must not
        // touch a single torrent. A hidden label that also paused things would
        // be a very unpleasant surprise.
        let options = Options {
            hide_by_default: true,
            ..Options::default()
        };
        assert!(options.to_torrent_options().is_empty());

        let settings = Settings::from_config(Some(&json!({
            "labels": {"noise": {"hide_by_default": true}, "films": {}}
        })));
        assert!(
            settings
                .options("noise")
                .expect("the label")
                .hide_by_default
        );
        assert!(
            !settings
                .options("films")
                .expect("the label")
                .hide_by_default,
            "a label that says nothing is visible"
        );
    }

    #[test]
    fn a_malformed_value_is_the_defaults_rather_than_an_error() {
        let settings = Settings::from_config(Some(&json!({"labels": "not a map"})));
        assert!(settings.names().is_empty());
    }
}
