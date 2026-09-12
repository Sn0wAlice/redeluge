// SPDX-License-Identifier: GPL-3.0-or-later
//! The weekly schedule: full speed, reduced speed, or stopped, by the hour.
//!
//! Deluge's Scheduler plugin, as a daemon feature. The grid is kept in the
//! shape the plugin stored it in, 24 rows of 7 with `grid[hour][weekday]` and
//! Monday as weekday 0, so the `button_state` out of an existing
//! `scheduler.conf` can be pasted into `core.conf` and mean the same thing.
//!
//! Deciding what the schedule says is pure and tested here. Acting on it lives
//! in the driver, because that needs a session.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

pub const HOURS: usize = 24;
pub const DAYS: usize = 7;

/// What the schedule says about right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The configuration's own limits apply. Green in the plugin.
    Full,
    /// The reduced limits apply. Yellow.
    Slow,
    /// The session is paused. Red.
    Stopped,
}

impl State {
    /// The plugin's name for this state, which is what its config stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "Green",
            Self::Slow => "Yellow",
            Self::Stopped => "Red",
        }
    }

    fn from_level(level: u8) -> Self {
        match level {
            1 => Self::Slow,
            2 => Self::Stopped,
            _ => Self::Full,
        }
    }
}

/// The schedule, as stored under the `scheduler` key of `core.conf`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    /// 24 rows of 7, `[hour][weekday]`, 0 full, 1 slow, 2 stopped.
    #[serde(default = "empty_grid")]
    pub button_state: Vec<Vec<u8>>,
    /// Reduced limits, in KiB/s. -1 is no limit, as everywhere else.
    #[serde(default = "minus_one_float")]
    pub low_down: f64,
    #[serde(default = "minus_one_float")]
    pub low_up: f64,
    #[serde(default = "minus_one")]
    pub low_active: i64,
    #[serde(default = "minus_one")]
    pub low_active_down: i64,
    #[serde(default = "minus_one")]
    pub low_active_up: i64,
}

fn empty_grid() -> Vec<Vec<u8>> {
    vec![vec![0; DAYS]; HOURS]
}
fn minus_one() -> i64 {
    -1
}
fn minus_one_float() -> f64 {
    -1.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            button_state: empty_grid(),
            low_down: -1.0,
            low_up: -1.0,
            low_active: -1,
            low_active_down: -1,
            low_active_up: -1,
        }
    }
}

impl Settings {
    /// Reads the settings out of the configuration value.
    ///
    /// A malformed value is the defaults rather than an error: this is reached
    /// from a client that can write any dictionary it likes into the key, and
    /// a daemon that refuses to start because of it would be worse than one
    /// that schedules nothing.
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            // The nulls come out first: `serde` fills in a key that is
            // absent, not one that is present and null, so one null used to
            // cost the whole dictionary.
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("scheduler", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    /// The default, as JSON, for `core.conf`.
    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// What the grid says for one hour of one day.
    ///
    /// Anything the grid does not cover is full speed. A short or ragged grid
    /// is a config someone edited by hand, and the safe reading of a missing
    /// cell is "no restriction" rather than "stop everything".
    pub fn state_at(&self, weekday: usize, hour: usize) -> State {
        if !self.enabled {
            return State::Full;
        }
        self.button_state
            .get(hour)
            .and_then(|row| row.get(weekday))
            .copied()
            .map(State::from_level)
            .unwrap_or(State::Full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_missing_configuration_schedules_nothing() {
        let settings = Settings::from_config(None);
        assert!(!settings.enabled);
        assert_eq!(settings.state_at(0, 0), State::Full);
    }

    #[test]
    fn a_disabled_schedule_is_full_speed_whatever_the_grid_says() {
        let mut grid = vec![vec![2u8; DAYS]; HOURS];
        grid[3][4] = 2;
        let settings = Settings {
            enabled: false,
            button_state: grid,
            ..Settings::default()
        };
        assert_eq!(settings.state_at(4, 3), State::Full);
    }

    #[test]
    fn the_grid_is_indexed_hour_then_weekday() {
        // Getting this the wrong way round would silently apply Tuesday's
        // schedule on Wednesday, and would still pass a test that only ever
        // filled the whole grid.
        let mut grid = vec![vec![0u8; DAYS]; HOURS];
        grid[9][2] = 1; // 09:00 on Wednesday
        let settings = Settings {
            enabled: true,
            button_state: grid,
            ..Settings::default()
        };

        assert_eq!(settings.state_at(2, 9), State::Slow);
        assert_eq!(settings.state_at(9, 2), State::Full);
    }

    #[test]
    fn the_three_levels_map_to_the_three_states() {
        let mut grid = vec![vec![0u8; DAYS]; HOURS];
        grid[0][0] = 0;
        grid[1][0] = 1;
        grid[2][0] = 2;
        grid[3][0] = 7; // Not a level the plugin ever wrote.
        let settings = Settings {
            enabled: true,
            button_state: grid,
            ..Settings::default()
        };

        assert_eq!(settings.state_at(0, 0), State::Full);
        assert_eq!(settings.state_at(0, 1), State::Slow);
        assert_eq!(settings.state_at(0, 2), State::Stopped);
        assert_eq!(settings.state_at(0, 3), State::Full);
    }

    #[test]
    fn a_cell_the_grid_does_not_have_is_full_speed() {
        let settings = Settings {
            enabled: true,
            button_state: vec![vec![2; 3]; 2],
            ..Settings::default()
        };
        assert_eq!(settings.state_at(0, 0), State::Stopped);
        assert_eq!(settings.state_at(5, 0), State::Full, "past the row's end");
        assert_eq!(settings.state_at(0, 9), State::Full, "past the last row");
    }

    #[test]
    fn a_partial_dictionary_keeps_the_defaults_for_what_it_omits() {
        let settings = Settings::from_config(Some(&json!({"enabled": true, "low_down": 50.0})));
        assert!(settings.enabled);
        assert_eq!(settings.low_down, 50.0);
        assert_eq!(settings.low_up, -1.0, "not given, so no limit");
        assert_eq!(settings.button_state.len(), HOURS);
    }

    #[test]
    fn a_malformed_value_is_ignored_rather_than_fatal() {
        // A client can write anything into this key through core.set_config.
        let settings = Settings::from_config(Some(&json!("every other tuesday")));
        assert!(!settings.enabled);
    }

    #[test]
    fn the_default_round_trips_through_json() {
        let json = Settings::default_json();
        let settings = Settings::from_config(Some(&json));
        assert!(!settings.enabled);
        assert_eq!(settings.button_state.len(), HOURS);
        assert_eq!(settings.button_state[0].len(), DAYS);
    }

    #[test]
    fn a_scheduler_conf_from_the_plugin_is_understood_as_it_is() {
        // Exactly the keys the plugin wrote, with nothing renamed.
        let stored = json!({
            "low_down": 25.0,
            "low_up": 10.0,
            "low_active": 4,
            "low_active_down": 2,
            "low_active_up": 2,
            "button_state": vec![vec![0u8; DAYS]; HOURS],
            "enabled": true,
        });
        let settings = Settings::from_config(Some(&stored));
        assert_eq!(settings.low_down, 25.0);
        assert_eq!(settings.low_active, 4);
        assert_eq!(settings.state_at(0, 0), State::Full);
    }
}
