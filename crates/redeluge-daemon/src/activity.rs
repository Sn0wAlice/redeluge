// SPDX-License-Identifier: GPL-3.0-or-later
//! What the daemon did without being asked.
//!
//! Seven things in this daemon act on their own: the queue's share-ratio rule,
//! the idle rule, the disk-space rule, the schedule, a tracker's rules for
//! labelling, moving and removing, and a label's rule for downloads that never
//! start. Three of them move or delete files. Until
//! this existed, the only trace any of them left was a line in the log, which
//! on a container install means `docker logs` and knowing to look.
//!
//! So they write here as well. A short history, in memory, newest first, read
//! back over `redeluge.get_recent_actions`. Not a second log and not an audit
//! trail: it answers "why is that torrent paused" and "what took my files
//! away", which is what somebody asks the moment automatic rules are turned on
//! and is exactly what a log file is bad at answering.
//!
//! In memory on purpose. It is worth nothing after a restart — the state it
//! explains is the state the daemon is in now — and a file would need rotating,
//! locking and a format, for something a person reads twice a month.

use std::collections::VecDeque;
use std::sync::Mutex;

/// How many actions are kept.
///
/// Enough to cover a night of a busy library and small enough that nobody has
/// to think about the memory: an entry is a few hundred bytes.
pub const CAPACITY: usize = 200;

/// Which rule acted. Kept short because the interface groups by it.
pub mod rule {
    pub const TRACKER: &str = "tracker";
    pub const LABEL: &str = "label";
    pub const IDLE: &str = "idle";
    pub const DISK: &str = "disk";
    pub const SCHEDULE: &str = "schedule";
    pub const RATIO: &str = "ratio";
    pub const FEED: &str = "feed";
}

/// What it did. Also short, and also read by the interface.
pub mod did {
    pub const LABELLED: &str = "labelled";
    pub const MOVED: &str = "moved";
    pub const REMOVED: &str = "removed";
    pub const PAUSED: &str = "paused";
    pub const RESUMED: &str = "resumed";
    pub const CHANGED: &str = "changed";
    pub const LIMITED: &str = "limited";
    pub const ADDED: &str = "added";
}

/// One thing the daemon did on its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    /// Unix seconds.
    pub at: f64,
    /// Which rule: one of [`rule`].
    pub rule: &'static str,
    /// What it did: one of [`did`].
    pub did: &'static str,
    /// The torrent, empty for something that was not about one.
    pub torrent_id: String,
    /// Its name, kept here because the torrent may be gone by the time anybody
    /// reads this — which for a removal is always.
    pub name: String,
    /// The rest of the sentence: where it moved to, what label it got, why it
    /// was paused.
    pub detail: String,
}

impl Action {
    pub fn new(rule: &'static str, did: &'static str, at: f64) -> Self {
        Self {
            at,
            rule,
            did,
            torrent_id: String::new(),
            name: String::new(),
            detail: String::new(),
        }
    }

    pub fn torrent(mut self, id: &str, name: &str) -> Self {
        self.torrent_id = id.to_owned();
        self.name = name.to_owned();
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

/// The history itself.
///
/// Its own lock rather than a field of the session state: the rules that write
/// here run on the async side and the session thread both, and neither should
/// have to wait for the other to record a line about something already done.
#[derive(Debug, Default)]
pub struct Log {
    entries: Mutex<VecDeque<Action>>,
}

impl Log {
    /// Records one action, dropping the oldest when the ring is full.
    ///
    /// Never fails and never blocks anything meaningful: a poisoned lock is
    /// dropped on the floor, because losing a line of history must not stop
    /// the rule that was writing it.
    pub fn record(&self, action: Action) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        entries.push_back(action);
    }

    /// The most recent actions, newest first.
    pub fn recent(&self, limit: usize) -> Vec<Action> {
        let Ok(entries) = self.entries.lock() else {
            return Vec::new();
        };
        entries.iter().rev().take(limit).cloned().collect()
    }

    /// How many are held.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(at: f64, name: &str) -> Action {
        Action::new(rule::TRACKER, did::REMOVED, at).torrent("abc", name)
    }

    #[test]
    fn the_newest_action_is_the_first_one_read_back() {
        let log = Log::default();
        log.record(action(1.0, "first"));
        log.record(action(2.0, "second"));

        let recent = log.recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].name, "second");
        assert_eq!(recent[1].name, "first");
    }

    #[test]
    fn a_limit_takes_the_newest_rather_than_the_first_it_finds() {
        let log = Log::default();
        for index in 0..10 {
            log.record(action(index as f64, &format!("torrent {index}")));
        }
        let recent = log.recent(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].name, "torrent 9");
        assert_eq!(recent[2].name, "torrent 7");
    }

    #[test]
    fn the_oldest_falls_off_the_end_rather_than_the_daemon_growing() {
        let log = Log::default();
        for index in 0..(CAPACITY + 50) {
            log.record(action(index as f64, &format!("torrent {index}")));
        }
        assert_eq!(log.len(), CAPACITY);

        let recent = log.recent(CAPACITY);
        assert_eq!(recent[0].name, format!("torrent {}", CAPACITY + 49));
        assert_eq!(recent[CAPACITY - 1].name, "torrent 50");
    }

    #[test]
    fn a_removal_still_says_what_it_removed() {
        // The reason the name is copied in rather than looked up: by the time
        // anybody reads this line, the torrent it names does not exist.
        let log = Log::default();
        log.record(
            Action::new(rule::TRACKER, did::REMOVED, 10.0)
                .torrent("deadbeef", "Some.Torrent.Name")
                .detail("example.org, with its files"),
        );
        let entry = &log.recent(1)[0];
        assert_eq!(entry.name, "Some.Torrent.Name");
        assert_eq!(entry.detail, "example.org, with its files");
        assert_eq!(entry.did, did::REMOVED);
    }

    #[test]
    fn an_empty_log_answers_nothing_rather_than_failing() {
        let log = Log::default();
        assert!(log.is_empty());
        assert!(log.recent(10).is_empty());
    }
}
