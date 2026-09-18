// SPDX-License-Identifier: GPL-3.0-or-later
//! What the trackers themselves are doing.
//!
//! The sidebar groups torrents by tracker domain, so `tracker.example.org` and
//! `backup.example.org` share one row and one set of rules. What they do not
//! share is a state: one can be refusing every announce while the other
//! answers, and until this existed the only trace of that was the tracker
//! column of whichever torrent happened to be on the failing one. A tracker
//! that has been down for a week looked exactly like a tracker nobody is
//! seeding from.
//!
//! Two questions come out of that, and this file answers both from the same
//! arithmetic:
//!
//! * Which of my trackers are down? Asked of every domain at once, by the
//!   sidebar, which colours its rows with the answer.
//! * What is going on with this one? Asked of a single domain, by the window
//!   that takes the row apart again and gives each announce URL a tab.
//!
//! Nothing here announces or scrapes. Every figure is one libtorrent was
//! already holding: opening a window is not a reason to send a tracker several
//! hundred requests, which is also why BEP 48's full scrape is switched off
//! nearly everywhere it was ever offered.

use std::collections::BTreeMap;

use redeluge_rencode::Value;

use crate::state::TorrentState;
use crate::torrent::{tracker_host, tracker_hostname, tracker_says_unregistered};

/// One torrent, reduced to the parts a tracker is judged by.
///
/// Built in `core.rs`, because that is where the session lock is held, and
/// read here so that the arithmetic can be tested without one.
pub struct Row {
    pub state: TorrentState,
    /// The tracker being announced to, which is where the swarm figures came
    /// from. A torrent that has not announced yet falls back to the first in
    /// its list, the same fallback the status and the sidebar use.
    pub current: String,
    pub trackers: Vec<redeluge_libtorrent::TrackerEntry>,
    /// What the tracker last said, as the status reports it.
    pub tracker_status: String,
    pub size: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    pub seeds: i64,
    pub peers: i64,
    /// Seconds until the next announce, 0 when none is due.
    pub next_announce: i64,
}

/// What one announce URL, or a whole domain, adds up to.
#[derive(Default)]
pub struct Totals {
    pub torrents: i64,
    /// Of those, the ones announcing here rather than to a sibling tracker.
    /// The swarm figures are theirs alone: they came back from an announce,
    /// and a tracker nobody announces to has reported no swarm.
    pub announcing: i64,
    /// Torrent-and-tracker pairs, not torrents: one torrent can be working on
    /// one tracker of this domain and failing on another.
    pub working: i64,
    pub failing: i64,
    pub updating: i64,
    /// The worst failure count seen, which is the one worth showing.
    pub fails: i64,
    pub message: String,
    pub unregistered: i64,
    pub size: i64,
    pub downloaded: i64,
    pub uploaded: i64,
    pub seeds: i64,
    pub peers: i64,
    /// The soonest announce due, in seconds. Zero when nothing is due.
    pub next_announce: i64,
    pub states: BTreeMap<&'static str, i64>,
}

impl Totals {
    /// Adds one torrent, to a tracker's totals or a domain's.
    ///
    /// `announcing` says whether this is the tracker the figures came from.
    /// Everything that is the torrent's own — its size, what it has moved —
    /// counts either way; everything the tracker said counts only where it was
    /// said, or a backup tracker that has never been reached would inherit the
    /// swarm of the one that has.
    fn add(&mut self, row: &Row, announcing: bool) {
        self.torrents += 1;
        self.size += row.size;
        self.downloaded += row.downloaded;
        self.uploaded += row.uploaded;
        *self.states.entry(row.state.as_str()).or_insert(0) += 1;
        if !announcing {
            return;
        }
        self.announcing += 1;
        // libtorrent answers -1 for a swarm it has not been told about, and a
        // sum of those counts down: a domain of two unreachable trackers
        // reported minus two seeds. Nothing known is nothing, not less than
        // nothing.
        self.seeds += row.seeds.max(0);
        self.peers += row.peers.max(0);
        if tracker_says_unregistered(&row.tracker_status) {
            self.unregistered += 1;
        }
        if row.next_announce > 0
            && (self.next_announce == 0 || row.next_announce < self.next_announce)
        {
            self.next_announce = row.next_announce;
        }
    }

    /// Adds what libtorrent recorded against one tracker of one torrent:
    /// whether it answered, and what it said when it did not.
    ///
    /// `verified` decides it, not `fails`, and the difference is not academic.
    /// libtorrent announces once per listen socket, and a machine with an IPv6
    /// socket talking to an IPv4-only tracker fails on one endpoint and
    /// succeeds on the other every single time. `fails` is the worst endpoint's
    /// count, so reading it first called every such tracker down while it was
    /// answering and reporting a swarm. If anything got through, the tracker is
    /// up, and the endpoint that did not is not this window's business.
    fn add_entry(&mut self, entry: &redeluge_libtorrent::TrackerEntry) {
        if entry.updating {
            self.updating += 1;
        }
        if entry.verified {
            self.working += 1;
            return;
        }
        if entry.fails == 0 {
            // Listed, never tried. Not a failure and not a success.
            return;
        }

        self.failing += 1;
        if i64::from(entry.fails) > self.fails {
            self.fails = i64::from(entry.fails);
        }
        // The first message is kept rather than the last: when a tracker is
        // down they are all the same sentence, and replacing it once per
        // torrent is work for an answer that does not change. Only a tracker
        // that reached nothing has one, so the sentence shown is always the
        // reason it is not working, never one endpoint's complaint about a
        // tracker that is.
        if self.message.is_empty() {
            if let Some(message) = &entry.message {
                self.message = message.clone();
            }
        }
    }

    /// One word for the state of this tracker, or of a whole domain.
    ///
    /// This is what the sidebar colours a row with, so the four answers are
    /// the four colours and there is no fifth. `down` is reserved for a
    /// tracker that is failing everywhere and working nowhere, because a
    /// colour that cries wolf over one torrent's stale announce is a colour
    /// people learn to ignore.
    pub fn health(&self) -> &'static str {
        if self.failing > 0 && self.working == 0 {
            "down"
        } else if self.failing > 0 || self.unregistered > 0 {
            "warning"
        } else if self.working > 0 {
            "ok"
        } else {
            // Added and not yet announced, or a domain whose torrents are all
            // paused. Nothing is wrong and nothing has been confirmed either.
            "unknown"
        }
    }

    /// The keys every entry carries.
    ///
    /// `url`, `host` and `tier` are the caller's to add, because the domain
    /// summary has none of the three.
    fn into_pairs(self) -> Vec<(Value, Value)> {
        let health = self.health();
        vec![
            (Value::Str("health".into()), Value::Str(health.to_owned())),
            (Value::Str("torrents".into()), Value::Int(self.torrents)),
            (Value::Str("announcing".into()), Value::Int(self.announcing)),
            (Value::Str("working".into()), Value::Int(self.working)),
            (Value::Str("failing".into()), Value::Int(self.failing)),
            (Value::Str("updating".into()), Value::Int(self.updating)),
            (Value::Str("fails".into()), Value::Int(self.fails)),
            (Value::Str("message".into()), Value::Str(self.message)),
            (
                Value::Str("unregistered".into()),
                Value::Int(self.unregistered),
            ),
            (Value::Str("size".into()), Value::Int(self.size)),
            (Value::Str("downloaded".into()), Value::Int(self.downloaded)),
            (Value::Str("uploaded".into()), Value::Int(self.uploaded)),
            (Value::Str("seeds".into()), Value::Int(self.seeds)),
            (Value::Str("peers".into()), Value::Int(self.peers)),
            (
                Value::Str("next_announce".into()),
                Value::Int(self.next_announce),
            ),
            (
                Value::Str("states".into()),
                Value::Dict(
                    self.states
                        .into_iter()
                        .map(|(state, count)| (Value::Str(state.to_owned()), Value::Int(count)))
                        .collect(),
                ),
            ),
        ]
    }
}

/// Every domain, summarised.
///
/// A torrent counts under the domain of the tracker it is announcing to, which
/// is the rule the sidebar counts by: the number here and the number on the
/// row have to agree or the colour is describing a different set of torrents
/// than the count beside it.
pub fn by_domain(rows: &[Row]) -> BTreeMap<String, Totals> {
    let mut out: BTreeMap<String, Totals> = BTreeMap::new();
    for row in rows {
        let host = tracker_host(&row.current);
        if host.is_empty() {
            continue;
        }
        let totals = out.entry(host.clone()).or_default();
        totals.add(row, true);
        for entry in &row.trackers {
            if tracker_host(&entry.url) == host {
                totals.add_entry(entry);
            }
        }
    }
    out
}

/// The sidebar's colours: one word per domain.
pub fn health_by_domain(rows: &[Row]) -> Value {
    Value::Dict(
        by_domain(rows)
            .into_iter()
            .map(|(host, totals)| (Value::Str(host), Value::Str(totals.health().to_owned())))
            .collect(),
    )
}

/// One domain, taken apart tracker by tracker.
///
/// The torrents counted are the ones the sidebar counts. Within them, a
/// torrent counts towards every tracker of the domain it lists, not only the
/// one it is announcing to: a listed backup tracker that has been failing for
/// a month is exactly what this is for.
pub fn detail(rows: &[Row], host: &str) -> Value {
    let mut domain = Totals::default();
    // Keyed by URL rather than by host: two entries on the same host that
    // differ by scheme or port are two trackers, and one of them can be the
    // one that is down.
    let mut trackers: BTreeMap<String, (u8, Totals)> = BTreeMap::new();

    for row in rows {
        if tracker_host(&row.current) != host {
            continue;
        }
        domain.add(row, true);

        for entry in &row.trackers {
            if tracker_host(&entry.url) != host {
                continue;
            }
            domain.add_entry(entry);

            let announcing = entry.url == row.current;
            let totals = trackers
                .entry(entry.url.clone())
                .or_insert_with(|| (entry.tier, Totals::default()));
            // The tier is the torrent's, not the tracker's, and two torrents
            // can disagree about it. The lowest wins, because that is the one
            // deciding when this tracker is tried at all.
            totals.0 = totals.0.min(entry.tier);
            totals.1.add(row, announcing);
            totals.1.add_entry(entry);
        }
    }

    // Tier first, because that is the order they are tried in and the window
    // draws its tabs in the order it is handed them.
    let mut ordered: Vec<(String, u8, Totals)> = trackers
        .into_iter()
        .map(|(url, (tier, totals))| (url, tier, totals))
        .collect();
    ordered.sort_by(|left, right| (left.1, &left.0).cmp(&(right.1, &right.0)));

    let entries: Vec<Value> = ordered
        .into_iter()
        .map(|(url, tier, totals)| {
            let mut pairs = vec![
                (Value::Str("url".into()), Value::Str(url.clone())),
                (
                    Value::Str("host".into()),
                    Value::Str(tracker_hostname(&url)),
                ),
                (Value::Str("tier".into()), Value::Int(i64::from(tier))),
            ];
            pairs.extend(totals.into_pairs());
            Value::Dict(pairs)
        })
        .collect();

    let mut pairs = vec![(Value::Str("host".into()), Value::Str(host.to_owned()))];
    pairs.extend(domain.into_pairs());
    pairs.push((Value::Str("trackers".into()), Value::List(entries)));
    Value::Dict(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    use redeluge_libtorrent::TrackerEntry;

    fn entry(url: &str, fails: i32, verified: bool) -> TrackerEntry {
        TrackerEntry {
            url: url.to_owned(),
            tier: 0,
            message: (fails > 0).then(|| "connection refused".to_owned()),
            verified,
            updating: false,
            fails,
        }
    }

    fn row(current: &str, trackers: Vec<TrackerEntry>) -> Row {
        Row {
            state: TorrentState::Seeding,
            current: current.to_owned(),
            trackers,
            tracker_status: String::new(),
            size: 1000,
            downloaded: 1000,
            uploaded: 2000,
            seeds: 7,
            peers: 3,
            next_announce: 600,
        }
    }

    /// Reads one key out of an answer, so a test can say what it is checking
    /// rather than how the dictionary is built.
    fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
        let Value::Dict(pairs) = value else {
            panic!("not a dictionary");
        };
        pairs
            .iter()
            .find(|(name, _)| matches!(name, Value::Str(name) if name == key))
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("no key {key}"))
    }

    fn int(value: &Value, key: &str) -> i64 {
        get(value, key).as_i64().expect("an integer")
    }

    fn text(value: &Value, key: &str) -> String {
        get(value, key).as_str().expect("a string").to_owned()
    }

    fn entries(value: &Value) -> Vec<Value> {
        let Value::List(entries) = get(value, "trackers") else {
            panic!("trackers is not a list");
        };
        entries.clone()
    }

    #[test]
    fn a_tracker_that_answers_is_working() {
        let rows = vec![row(
            "http://tracker.example.com/announce",
            vec![entry("http://tracker.example.com/announce", 0, true)],
        )];
        let health = by_domain(&rows);
        assert_eq!(health["example.com"].health(), "ok");
    }

    #[test]
    fn a_tracker_that_answers_nowhere_is_down() {
        // Down is the strong word, so it is kept for the case where nothing
        // has got through: one failing announce among working ones is trouble,
        // not a tracker that has gone away.
        let rows = vec![
            row(
                "http://tracker.example.com/announce",
                vec![entry("http://tracker.example.com/announce", 3, false)],
            ),
            row(
                "http://tracker.example.com/announce",
                vec![entry("http://tracker.example.com/announce", 1, false)],
            ),
        ];
        let health = by_domain(&rows);
        assert_eq!(health["example.com"].health(), "down");
        assert_eq!(health["example.com"].fails, 3);

        let mut mixed = rows;
        mixed.push(row(
            "http://tracker.example.com/announce",
            vec![entry("http://tracker.example.com/announce", 0, true)],
        ));
        assert_eq!(by_domain(&mixed)["example.com"].health(), "warning");
    }

    #[test]
    fn a_torrent_the_tracker_has_dropped_is_a_warning() {
        // The tracker is answering, so it is not down. Something is still
        // wrong, and this is the sidebar's only chance to say so.
        let mut only = row(
            "http://tracker.example.com/announce",
            vec![entry("http://tracker.example.com/announce", 0, true)],
        );
        only.tracker_status = "Error: unregistered torrent".to_owned();

        let health = by_domain(&[only]);
        assert_eq!(health["example.com"].health(), "warning");
        assert_eq!(health["example.com"].unregistered, 1);
    }

    #[test]
    fn one_endpoint_failing_is_not_a_tracker_that_is_down() {
        // The case the preview found. libtorrent announces once per listen
        // socket, so a host with an IPv6 socket and an IPv4-only tracker
        // records a failure on one endpoint and a success on the other, every
        // time, for ever. Reading `fails` first called that tracker down while
        // it was answering and reporting a swarm of thirty-six.
        let url = "http://tracker.example.com/announce";
        let mut half = entry(url, 2, true);
        half.message = Some("skipping tracker announce (unreachable)".to_owned());

        let rows = vec![row(url, vec![half])];
        let health = by_domain(&rows);
        assert_eq!(health["example.com"].health(), "ok");
        assert_eq!(health["example.com"].failing, 0);
        // And nothing is reported as the reason it is not working, because it
        // is working.
        assert_eq!(health["example.com"].fails, 0);
        assert_eq!(health["example.com"].message, "");
    }

    #[test]
    fn a_swarm_nobody_has_reported_is_none_rather_than_less_than_none() {
        // libtorrent answers -1 for a swarm it has not been told about. Summed
        // as they came, two unreachable trackers reported minus two seeds.
        let url = "http://tracker.example.com/announce";
        let mut unknown = row(url, vec![entry(url, 1, false)]);
        unknown.seeds = -1;
        unknown.peers = -1;

        let health = by_domain(&[unknown]);
        assert_eq!(health["example.com"].seeds, 0);
        assert_eq!(health["example.com"].peers, 0);
    }

    #[test]
    fn a_tracker_nothing_has_reached_yet_is_unknown() {
        // Added and not yet announced. Nothing is wrong, and saying `ok` would
        // be claiming something nobody has checked.
        let rows = vec![row(
            "http://tracker.example.com/announce",
            vec![entry("http://tracker.example.com/announce", 0, false)],
        )];
        assert_eq!(by_domain(&rows)["example.com"].health(), "unknown");
    }

    #[test]
    fn one_domain_is_taken_apart_tracker_by_tracker() {
        // The case the window exists for: two trackers under one sidebar row,
        // one answering and one not.
        let working = "http://tracker.example.com/announce";
        let broken = "udp://backup.example.com:6969";
        let rows = vec![
            row(
                working,
                vec![entry(working, 0, true), entry(broken, 4, false)],
            ),
            row(
                working,
                vec![entry(working, 0, true), entry(broken, 4, false)],
            ),
        ];

        let info = detail(&rows, "example.com");
        assert_eq!(text(&info, "host"), "example.com");
        assert_eq!(int(&info, "torrents"), 2);
        // Working somewhere, failing somewhere else.
        assert_eq!(text(&info, "health"), "warning");

        let entries = entries(&info);
        assert_eq!(entries.len(), 2);

        let first = &entries[0];
        assert_eq!(text(first, "url"), working);
        assert_eq!(text(first, "host"), "tracker.example.com");
        assert_eq!(text(first, "health"), "ok");
        assert_eq!(int(first, "announcing"), 2);
        // Two torrents, seven seeds each, and the swarm belongs to the tracker
        // that reported it.
        assert_eq!(int(first, "seeds"), 14);

        let second = &entries[1];
        assert_eq!(text(second, "url"), broken);
        assert_eq!(text(second, "host"), "backup.example.com");
        assert_eq!(text(second, "health"), "down");
        assert_eq!(text(second, "message"), "connection refused");
        // Listed by both torrents, announcing for neither, so it has reported
        // no swarm and is not credited with the other tracker's.
        assert_eq!(int(second, "torrents"), 2);
        assert_eq!(int(second, "announcing"), 0);
        assert_eq!(int(second, "seeds"), 0);
    }

    #[test]
    fn another_domains_trackers_are_not_this_domains_business() {
        // A torrent commonly lists a public tracker beside the private one it
        // is grouped under. It is not part of this row and its state is not
        // this row's state.
        let mine = "http://tracker.example.com/announce";
        let public = "udp://tracker.opentrackr.org:1337/announce";
        let rows = vec![row(
            mine,
            vec![entry(mine, 0, true), entry(public, 9, false)],
        )];

        let info = detail(&rows, "example.com");
        assert_eq!(text(&info, "health"), "ok");
        assert_eq!(entries(&info).len(), 1);

        // And the torrent counts under the tracker it announces to, which is
        // the rule the sidebar counts by.
        let health = by_domain(&rows);
        assert_eq!(health.len(), 1);
        assert_eq!(health["example.com"].health(), "ok");
    }

    #[test]
    fn the_lowest_tier_a_tracker_is_given_wins() {
        // The tier is the torrent's, not the tracker's, and two torrents can
        // disagree. The lowest is the one that decides when it is tried.
        let url = "http://tracker.example.com/announce";
        let mut high = entry(url, 0, true);
        high.tier = 3;
        let rows = vec![row(url, vec![entry(url, 0, true)]), row(url, vec![high])];

        let entries = entries(&detail(&rows, "example.com"));
        assert_eq!(entries.len(), 1);
        assert_eq!(int(&entries[0], "tier"), 0);
    }
}
