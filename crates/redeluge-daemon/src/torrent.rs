// SPDX-License-Identifier: GPL-3.0-or-later
//! A torrent, as the daemon sees it.
//!
//! libtorrent knows how to move the bytes. Everything else is here: the options
//! the user chose, the name they gave it, what happens when it finishes, and
//! the status dictionary every client reads.

use std::collections::BTreeMap;

use redeluge_libtorrent::{flags, TorrentStatus as LtStatus};
use redeluge_rencode::Value;
use serde::{Deserialize, Serialize};

use crate::state::{derive, StateContext, TorrentState};

/// The per-torrent options the daemon keeps, saved across restarts.
///
/// These are `deluge/core/torrentmanager.py`'s `TorrentState` fields, which the
/// Python daemon stores as a pickle. Here they are JSON, and the field names
/// match so a converted file lines up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentOptions {
    #[serde(default)]
    pub torrent_id: String,
    /// The `.torrent` file this was added from, relative to the state
    /// directory. Empty for a magnet.
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub magnet: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub save_path: Option<String>,
    #[serde(default = "default_storage_mode")]
    pub storage_mode: String,
    #[serde(default)]
    pub paused: bool,
    #[serde(default = "yes")]
    pub auto_managed: bool,
    #[serde(default = "minus_one")]
    pub max_connections: i64,
    #[serde(default = "minus_one")]
    pub max_upload_slots: i64,
    #[serde(default = "minus_one_float")]
    pub max_upload_speed: f64,
    #[serde(default = "minus_one_float")]
    pub max_download_speed: f64,
    #[serde(default)]
    pub prioritize_first_last: bool,
    #[serde(default)]
    pub sequential_download: bool,
    #[serde(default)]
    pub file_priorities: Vec<u8>,
    #[serde(default)]
    pub is_finished: bool,
    #[serde(default = "two")]
    pub stop_ratio: f64,
    #[serde(default)]
    pub stop_at_ratio: bool,
    #[serde(default)]
    pub remove_at_ratio: bool,
    #[serde(default)]
    pub move_completed: bool,
    #[serde(default)]
    pub move_completed_path: Option<String>,
    /// The label this torrent carries, empty for none.
    ///
    /// Deluge kept this in the Label plugin's own config file, keyed by
    /// torrent id. Here it is an option like any other, so it is set through
    /// `core.set_torrent_options`, reported in the status, and saved and
    /// restored with the torrent rather than in a second file that can drift
    /// out of step with the first.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub shared: bool,
    #[serde(default)]
    pub super_seeding: bool,
    /// When the idle rule should let this torrent go again, in Unix seconds.
    ///
    /// Zero means it is not paused by that rule. Saved with the torrent on
    /// purpose: a daemon that restarted while a torrent was put away would
    /// otherwise leave it paused for good, with nothing to say why.
    #[serde(default)]
    pub idle_resume_at: f64,

    /// Whether a finished notification has already gone out for this torrent.
    ///
    /// Saved with the torrent because the thing it prevents happens at
    /// startup: libtorrent reports a torrent as finished again once it has
    /// re-checked one that was already complete, so without this a restart
    /// would announce the whole library.
    #[serde(default)]
    pub announced_finished: bool,

    /// Whether the disk-space rule is the reason this torrent is paused.
    ///
    /// Saved with the torrent, like the idle rule's clock above and for the
    /// same reason: a daemon that restarted while the disk was full would
    /// otherwise have no way to tell what it had stopped, and would leave a
    /// pile of torrents paused with nothing to say why.
    #[serde(default)]
    pub space_paused: bool,

    /// Whether the queue was managing this torrent before that rule took it.
    ///
    /// The rule has to clear auto-management to make the pause stick, so it
    /// has to remember what it cleared: a torrent somebody was running by hand
    /// must not come back under the queue just because a disk filled up.
    #[serde(default = "yes")]
    pub space_was_managed: bool,

    /// Trackers the user added or reordered, kept because libtorrent forgets
    /// them when a torrent is removed and re-added from resume data.
    #[serde(default)]
    pub trackers: Vec<TrackerOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerOption {
    pub url: String,
    #[serde(default)]
    pub tier: u8,
}

fn default_storage_mode() -> String {
    "sparse".to_owned()
}
fn yes() -> bool {
    true
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

impl Default for TorrentOptions {
    fn default() -> Self {
        Self {
            torrent_id: String::new(),
            filename: String::new(),
            magnet: None,
            name: None,
            save_path: None,
            storage_mode: default_storage_mode(),
            paused: false,
            auto_managed: true,
            max_connections: -1,
            max_upload_slots: -1,
            max_upload_speed: -1.0,
            max_download_speed: -1.0,
            prioritize_first_last: false,
            sequential_download: false,
            file_priorities: Vec::new(),
            is_finished: false,
            stop_ratio: 2.0,
            stop_at_ratio: false,
            remove_at_ratio: false,
            move_completed: false,
            move_completed_path: None,
            label: String::new(),
            owner: String::new(),
            shared: false,
            super_seeding: false,
            idle_resume_at: 0.0,
            announced_finished: false,
            space_paused: false,
            space_was_managed: true,
            trackers: Vec::new(),
        }
    }
}

/// What the daemon knows about one torrent beyond libtorrent's status.
#[derive(Debug, Clone)]
pub struct Torrent {
    pub id: String,
    pub options: TorrentOptions,
    /// An error the daemon raised itself, such as a failed move. It outranks
    /// anything libtorrent reports.
    pub forced_error: Option<String>,
    /// Free text shown next to the state. "OK" when there is nothing to say.
    pub status_message: String,
    /// The last tracker message, which clients show in their own column.
    pub tracker_status: String,
    /// Where a move is going, while one is in progress.
    pub moving_to: Option<String>,
    /// What this torrent carries, as a fingerprint of its file list.
    ///
    /// The same content published on two trackers has two infohashes — the
    /// tracker adds a `source` field, or the piece size differs — so the
    /// infohash cannot say they are the same download. The file list can:
    /// see [`content_fingerprint`]. Computed once, when the metadata is there,
    /// and kept because it cannot change afterwards.
    pub content_id: Option<String>,
}

impl Torrent {
    pub fn new(id: String, options: TorrentOptions) -> Self {
        Self {
            id,
            options,
            forced_error: None,
            status_message: "OK".to_owned(),
            tracker_status: String::new(),
            moving_to: None,
            content_id: None,
        }
    }

    /// The state this torrent is in.
    pub fn state(&self, status: &LtStatus, session_paused: bool) -> TorrentState {
        derive(
            status,
            StateContext {
                session_paused,
                forced_error: self.forced_error.is_some(),
            },
        )
    }

    /// The full status dictionary, as the RPC returns it.
    ///
    /// Clients ask for a subset by key, so this builds everything and the
    /// caller filters. Building all of it costs one pass over a status that has
    /// already been fetched.
    pub fn status(
        &self,
        status: &LtStatus,
        session_paused: bool,
        trackers: &[redeluge_libtorrent::TrackerEntry],
    ) -> BTreeMap<String, Value> {
        self.status_with_peers(
            status,
            session_paused,
            trackers,
            &[],
            0.0,
            0,
            &crate::features::tracker::Settings::default(),
        )
    }

    /// The status, with the peer list filled in.
    ///
    /// Peers are a separate call into libtorrent and only one tab of the
    /// interface shows them, so the caller decides whether to pay for them.
    #[allow(clippy::too_many_arguments)]
    pub fn status_with_peers(
        &self,
        status: &LtStatus,
        session_paused: bool,
        trackers: &[redeluge_libtorrent::TrackerEntry],
        peers: &[(redeluge_libtorrent::PeerInfo, PeerCountry)],
        idle_since: f64,
        idle_grace: u64,
        tracker_rules: &crate::features::tracker::Settings,
    ) -> BTreeMap<String, Value> {
        let state = self.state(status, session_paused);
        let mut out: BTreeMap<String, Value> = BTreeMap::new();

        let mut put = |key: &str, value: Value| {
            out.insert(key.to_owned(), value);
        };

        put("hash", Value::Str(self.id.clone()));
        put("name", Value::Str(self.display_name(status)));
        put("state", Value::Str(state.to_string()));
        put("message", Value::Str(self.message()));
        put(
            "progress",
            Value::Float64(f64::from(status.progress) * 100.0),
        );

        put("save_path", Value::Str(status.save_path.clone()));
        // Deluge renamed this key and kept the old one working.
        put("download_location", Value::Str(status.save_path.clone()));
        put(
            "storage_mode",
            Value::Str(self.options.storage_mode.clone()),
        );

        put("total_done", Value::Int(status.total_done));
        put("total_wanted", Value::Int(status.total_wanted));
        put(
            "total_remaining",
            Value::Int((status.total_wanted - status.total_wanted_done).max(0)),
        );
        put("total_size", Value::Int(status.total_size));
        put("total_uploaded", Value::Int(status.all_time_upload));
        put("all_time_download", Value::Int(status.all_time_download));
        put(
            "total_payload_download",
            Value::Int(status.total_payload_download),
        );
        put(
            "total_payload_upload",
            Value::Int(status.total_payload_upload),
        );

        put(
            "download_payload_rate",
            Value::Int(i64::from(status.download_payload_rate)),
        );
        put(
            "upload_payload_rate",
            Value::Int(i64::from(status.upload_payload_rate)),
        );

        put(
            "num_peers",
            Value::Int(i64::from(status.num_peers - status.num_seeds)),
        );
        put("num_seeds", Value::Int(i64::from(status.num_seeds)));
        put("total_peers", Value::Int(i64::from(status.num_incomplete)));
        put("total_seeds", Value::Int(i64::from(status.num_complete)));
        put(
            "seeds_peers_ratio",
            Value::Float64(if status.num_incomplete <= 0 {
                // Deluge reports -1 rather than infinity, which its clients
                // render as a dash.
                -1.0
            } else {
                f64::from(status.num_complete) / f64::from(status.num_incomplete)
            }),
        );

        put(
            "distributed_copies",
            Value::Float64(f64::from(status.distributed_copies).max(0.0)),
        );
        put("eta", Value::Int(self.eta(status)));
        put("ratio", Value::Float64(self.ratio(status)));

        put("time_added", Value::Int(status.added_time));
        put("completed_time", Value::Int(status.completed_time));
        put("active_time", Value::Int(status.active_time));
        put("seeding_time", Value::Int(status.seeding_time));
        put("finished_time", Value::Int(status.finished_time));
        put(
            "time_since_transfer",
            Value::Int(status.time_since_download),
        );
        put("last_seen_complete", Value::Int(status.last_seen_complete));
        put("next_announce", Value::Int(status.next_announce));

        put("queue", Value::Int(i64::from(status.queue_position)));
        put("seed_rank", Value::Int(i64::from(status.seed_rank)));
        put("is_finished", Value::Bool(status.is_finished));
        put("is_seed", Value::Bool(status.is_seeding));
        put("paused", Value::Bool(status.is_paused));
        put("moving_completed", Value::Bool(status.moving_storage));
        put("has_metadata", Value::Bool(status.has_metadata));
        put("num_pieces", Value::Int(i64::from(status.num_pieces)));
        put("piece_length", Value::Int(i64::from(status.piece_length)));
        put("num_files", Value::Int(i64::from(status.num_files)));

        put("auto_managed", Value::Bool(status.is_auto_managed()));
        put("is_auto_managed", Value::Bool(status.is_auto_managed()));
        put("sequential_download", Value::Bool(status.is_sequential()));
        put(
            "super_seeding",
            Value::Bool(status.has_flag(flags::SUPER_SEEDING)),
        );
        put(
            "prioritize_first_last",
            Value::Bool(self.options.prioritize_first_last),
        );
        put(
            "prioritize_first_last_pieces",
            Value::Bool(self.options.prioritize_first_last),
        );

        put("max_connections", Value::Int(self.options.max_connections));
        put(
            "max_upload_slots",
            Value::Int(self.options.max_upload_slots),
        );
        put(
            "max_upload_speed",
            Value::Float64(self.options.max_upload_speed),
        );
        put(
            "max_download_speed",
            Value::Float64(self.options.max_download_speed),
        );

        put("stop_ratio", Value::Float64(self.options.stop_ratio));
        put("stop_at_ratio", Value::Bool(self.options.stop_at_ratio));
        put("remove_at_ratio", Value::Bool(self.options.remove_at_ratio));
        put("move_completed", Value::Bool(self.options.move_completed));
        put(
            "move_on_completed",
            Value::Bool(self.options.move_completed),
        );
        let move_path = self
            .options
            .move_completed_path
            .clone()
            .unwrap_or_else(|| status.save_path.clone());
        put("move_completed_path", Value::Str(move_path.clone()));
        put("move_on_completed_path", Value::Str(move_path));

        put(
            "peers",
            Value::List(
                peers
                    .iter()
                    .map(|(peer, country)| {
                        Value::Dict(vec![
                            (
                                Value::Str("ip".into()),
                                Value::Str(format!("{}:{}", peer.ip, peer.port)),
                            ),
                            (Value::Str("client".into()), Value::Str(peer.client.clone())),
                            (
                                Value::Str("country".into()),
                                Value::Str(country.code.clone().unwrap_or_default()),
                            ),
                            // The name beside the code: the code picks the
                            // flag and is unreadable on its own.
                            (
                                Value::Str("country_name".into()),
                                Value::Str(country.name.clone().unwrap_or_default()),
                            ),
                            (
                                Value::Str("progress".into()),
                                Value::Float64(f64::from(peer.progress)),
                            ),
                            (
                                Value::Str("down_speed".into()),
                                Value::Int(i64::from(peer.down_speed)),
                            ),
                            (
                                Value::Str("up_speed".into()),
                                Value::Int(i64::from(peer.up_speed)),
                            ),
                            (Value::Str("seed".into()), Value::Int(i64::from(peer.seed))),
                            // How the connection is made, which libtorrent
                            // knows and nothing was carrying.
                            (Value::Str("utp".into()), Value::Bool(peer.utp)),
                            (Value::Str("encrypted".into()), Value::Bool(peer.encrypted)),
                            // The number that says whether this peer is worth
                            // having: pieces it has and we do not.
                            (
                                Value::Str("useful_pieces".into()),
                                Value::Int(i64::from(peer.useful_pieces)),
                            ),
                        ])
                    })
                    .collect(),
            ),
        );
        put("label", Value::Str(self.options.label.clone()));
        put("owner", Value::Str(self.options.owner.clone()));
        put("shared", Value::Bool(self.options.shared));
        put("private", Value::Bool(false));
        put("comment", Value::Str(String::new()));
        put("creator", Value::Str(String::new()));

        let current = current_tracker(&status.current_tracker, trackers);
        let host = tracker_host(&current);
        put("tracker", Value::Str(current.clone()));
        put("tracker_host", Value::Str(host.clone()));
        put("tracker_status", Value::Str(self.tracker_status.clone()));

        // What this torrent's tracker is about to do to it, as two moments
        // rather than two sentences: the interface counts down between polls,
        // and a sentence computed here would be stale the moment it arrived.
        // Zero means nothing is coming, as it does for the idle rule below.
        //
        // The conditions are the sweep's, and they have to stay the sweep's: a
        // countdown that reaches zero and is followed by nothing is worse than
        // no countdown, because it teaches you not to believe the next one.
        let (move_at, remove_at) = tracker_countdowns(status, &host, tracker_rules);
        put("tracker_move_at", Value::Float64(move_at));
        put("tracker_remove_at", Value::Float64(remove_at));

        // The idle rule's countdown. Three numbers rather than a sentence,
        // because the interface counts down between polls and a sentence
        // computed here would be two seconds stale the moment it arrived.
        // Zero means "not counting", as it does for every other time here.
        put("idle_since", Value::Float64(idle_since));
        put(
            "idle_pause_at",
            Value::Float64(crate::features::idlepause::pause_at(idle_since, idle_grace)),
        );
        put(
            "idle_resume_at",
            Value::Float64(self.options.idle_resume_at),
        );
        // The other reason a torrent can be paused without anybody asking.
        put("space_paused", Value::Bool(self.options.space_paused));
        put(
            "trackers",
            Value::List(
                trackers
                    .iter()
                    .map(|tracker| {
                        Value::Dict(vec![
                            (Value::Str("url".into()), Value::Str(tracker.url.clone())),
                            (
                                Value::Str("tier".into()),
                                Value::Int(i64::from(tracker.tier)),
                            ),
                        ])
                    })
                    .collect(),
            ),
        );

        out
    }

    /// The name shown to the user, which may be one they chose.
    pub fn display_name(&self, status: &LtStatus) -> String {
        if let Some(name) = &self.options.name {
            if !name.is_empty() {
                return name.clone();
            }
        }
        if !status.name.is_empty() {
            return status.name.clone();
        }
        // A magnet with no metadata and no name has only its infohash.
        self.id.clone()
    }

    /// The error or status line shown for this torrent.
    ///
    /// Public because a notification carries it: what went wrong is the whole
    /// content of the message an error sends.
    pub fn message(&self) -> String {
        if let Some(error) = &self.forced_error {
            return error.clone();
        }
        self.status_message.clone()
    }

    /// Seconds until done, or 0 when that cannot be said.
    ///
    /// Deluge reports 0 rather than a large number for "never", and clients
    /// render 0 as an infinity sign.
    fn eta(&self, status: &LtStatus) -> i64 {
        if status.is_finished || status.is_seeding {
            // A seeding torrent's countdown is to the seeding rule, not to
            // completion, and Deluge only computes that when one is set.
            if !self.options.stop_at_ratio {
                return 0;
            }
            let ratio = self.ratio(status);
            if ratio >= self.options.stop_ratio || status.upload_payload_rate <= 0 {
                return 0;
            }
            let wanted = self.options.stop_ratio * status.all_time_download as f64;
            let remaining = wanted - status.all_time_upload as f64;
            return (remaining / f64::from(status.upload_payload_rate)) as i64;
        }

        let left = status.total_wanted - status.total_wanted_done;
        if left <= 0 || status.download_payload_rate <= 0 {
            return 0;
        }
        left / i64::from(status.download_payload_rate)
    }

    /// Upload divided by download.
    ///
    /// Deluge reports -1 when nothing has been downloaded, which its clients
    /// render as a dash. Returning infinity would make every seeding rule fire.
    pub fn ratio(&self, status: &LtStatus) -> f64 {
        if status.all_time_download <= 0 {
            return -1.0;
        }
        status.all_time_upload as f64 / status.all_time_download as f64
    }
}

/// Where a peer is, as far as the country database knows.
///
/// Two fields rather than one because they answer different questions: the
/// code picks the flag, the name is what a person reads. A database may have
/// the code and no name for a country, so the name is optional on its own.
#[derive(Debug, Clone, Default)]
pub struct PeerCountry {
    pub code: Option<String>,
    pub name: Option<String>,
}

/// Which tracker a torrent counts as being on.
///
/// The one it last announced to, or the first it knows about when it has not
/// announced yet. The fallback is Deluge's, and without it a torrent shows no
/// tracker at all until its first announce succeeds, which puts it in the
/// sidebar's group for torrents that have none.
///
/// Shared, because the filter tree and the torrent status both need the
/// A fingerprint of what a torrent carries, from its file list.
///
/// Two torrents of the same content have different infohashes as a matter of
/// course: a private tracker stamps its own `source` into the info dictionary,
/// which changes the hash without changing a byte of the data. So the infohash
/// cannot answer "are these the same download". This can, near enough: the
/// file names and their sizes to the byte, in a fixed order.
///
/// Near enough, and not proof — which is why nothing in this daemon deletes
/// anything on the strength of it. Two unrelated files can share a name and a
/// size; it is unlikely and it is possible. What it is good for is grouping:
/// showing that a peer carries the same content on two of your trackers, or
/// that you are storing the same download twice.
///
/// The order the files come in is not part of it, because it is not part of
/// the content: libtorrent reports them in the torrent's own order and two
/// torrents of the same files can list them differently.
pub fn content_fingerprint(files: &[redeluge_libtorrent::FileEntry]) -> String {
    if files.is_empty() {
        return String::new();
    }

    // Names rather than paths: the same release inside a differently named
    // folder is the same release, and the folder is the part a tracker is most
    // likely to have renamed.
    let mut parts: Vec<(String, i64)> = files
        .iter()
        .map(|file| {
            let name = file
                .path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&file.path)
                .to_lowercase();
            (name, file.size)
        })
        .collect();
    parts.sort();

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut total: i64 = 0;
    for (name, size) in &parts {
        total = total.saturating_add(*size);
        for byte in name.as_bytes().iter().chain(&size.to_le_bytes()) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    }

    // The total as well as the parts, so that two different file lists have to
    // collide in both to collide at all.
    format!("{hash:016x}{total:x}")
}

/// Whether the tracker has said this torrent no longer exists.
///
/// A private tracker that has pruned a torrent answers every announce with the
/// same handful of phrases, and the torrent then sits in the list for ever:
/// seeding to nobody, counting for nothing, and indistinguishable at a glance
/// from one that simply has no peers today. This is the check that tells them
/// apart, so the sidebar can group them and you can throw them away.
///
/// Deliberately narrow. A tracker that is down, refusing connections or
/// rate-limiting is not a tracker that has forgotten the torrent, and a rule
/// that lumped the two together would offer to delete a library because a
/// tracker was rebooting. Only the phrases that mean "I have no record of
/// this" count, and everything else is an error like any other.
pub fn tracker_says_unregistered(tracker_status: &str) -> bool {
    /// What trackers answer when the torrent is gone. Lower case; the status
    /// is folded before it is compared.
    const GONE: &[&str] = &[
        "unregistered",
        "not registered",
        "torrent not found",
        "unknown torrent",
        "info hash not found",
        "infohash not found",
    ];

    let status = tracker_status.to_lowercase();
    GONE.iter().any(|phrase| status.contains(phrase))
}

/// When each of a tracker's rules is due to act on this torrent.
///
/// Answers `(move, remove)` as Unix seconds, zero for "not counting down".
/// The two differ in what counts as having finished, which
/// [`crate::features::tracker::finished_at`] explains: the destructive rule
/// will not work from a completion time libtorrent never saw, and the others
/// will.
fn tracker_countdowns(
    status: &LtStatus,
    host: &str,
    rules: &crate::features::tracker::Settings,
) -> (f64, f64) {
    use crate::features::tracker;

    let Some(options) = rules.options(host) else {
        return (0.0, 0.0);
    };
    if !status.is_finished {
        return (0.0, 0.0);
    }

    let move_at = if options.moves()
        && status.save_path.trim_end_matches('/') != options.move_path.trim_end_matches('/')
    {
        tracker::due_at(
            tracker::finished_at(status.completed_time, status.added_time, false),
            options.move_after_hours,
        )
    } else {
        0.0
    };

    let remove_at = if options.removes() {
        tracker::due_at(
            tracker::finished_at(status.completed_time, status.added_time, true),
            options.remove_after_hours,
        )
    } else {
        0.0
    };

    (move_at, remove_at)
}

/// answer, and the last time each worked it out for itself they disagreed.
pub fn current_tracker(announced: &str, trackers: &[redeluge_libtorrent::TrackerEntry]) -> String {
    if !announced.is_empty() {
        return announced.to_owned();
    }
    trackers
        .first()
        .map(|entry| entry.url.clone())
        .unwrap_or_default()
}

/// The host a tracker URL names, subdomain and all.
///
/// [`tracker_host`] drops the subdomain, because the sidebar groups
/// `tracker.example.org` and `announce.example.org` into one row. A window
/// about that row has to take them apart again, and this is the name each one
/// goes by. An IPv6 literal comes back without its brackets, and a port is
/// not part of a host.
pub fn tracker_hostname(url: &str) -> String {
    // udp:// parses like any other scheme once it is one this understands.
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    // Strip any credentials, then take the host up to the port or path.
    let authority = without_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, rest)| rest);

    // An IPv6 literal is bracketed, and its colons are not a port separator.
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_owned();
    }
    authority.split(':').next().unwrap_or("").to_owned()
}

/// What clients group a torrent's tracker by.
///
/// Deluge's rule, label for label, because this string is a filter value as
/// well as something to display: a client sends back exactly what it was shown
/// and expects it to match. Subdomains are dropped so that
/// `tracker.example.org` and `announce.example.org` are one group.
///
/// The rule was a length heuristic here once, which got `x.abc.com` wrong
/// (three-letter second level) and turned the IP address `192.168.1.1` into
/// `168.1.1`. This is the list Deluge actually uses.
pub fn tracker_host(url: &str) -> String {
    let host = tracker_hostname(url);
    if host.is_empty() {
        return String::new();
    }
    // An address is not a name and has no subdomain to drop.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_owned();
    }

    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() <= 2 {
        return host.to_owned();
    }

    // Two-part public suffixes: keep three labels so the answer is a domain
    // rather than the suffix itself.
    let second_level = labels[labels.len() - 2];
    let top_level = labels[labels.len() - 1];
    let keep = if matches!(second_level, "co" | "com" | "net" | "org")
        || matches!(top_level, "uk" | "au")
    {
        3
    } else {
        2
    };
    labels[labels.len().saturating_sub(keep)..].join(".")
}

/// Adds the three file keys to a status dictionary.
///
/// Kept out of `status_with_peers` because each of these is a separate call
/// into libtorrent and only the Files tab wants them, the same reason the peer
/// list is optional. Until this existed the daemon answered
/// `core.get_torrent_status` without the keys at all, so `web.get_torrent_files`
/// built an empty tree and the Files tab of the interface was blank for every
/// torrent.
///
/// The shape is Deluge's: `files` is a list of dictionaries with the index,
/// path, size and offset; `file_progress` is the fraction of each file that is
/// done, not a byte count; `file_priorities` is one number per file.
pub fn put_files(
    out: &mut BTreeMap<String, Value>,
    entries: &[redeluge_libtorrent::FileEntry],
    progress: &[i64],
    priorities: &[u8],
) {
    let files = entries
        .iter()
        .map(|file| {
            Value::Dict(vec![
                (
                    Value::Str("index".into()),
                    Value::Int(i64::from(file.index)),
                ),
                (Value::Str("path".into()), Value::Str(file.path.clone())),
                (Value::Str("size".into()), Value::Int(file.size)),
                (Value::Str("offset".into()), Value::Int(file.offset)),
            ])
        })
        .collect();
    out.insert("files".to_owned(), Value::List(files));

    let done = entries
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let bytes = progress.get(index).copied().unwrap_or(0);
            let fraction = if file.size > 0 {
                (bytes as f64 / file.size as f64).clamp(0.0, 1.0)
            } else {
                // A zero-length file is complete the moment it exists, and
                // dividing by its size would be a NaN in the interface.
                1.0
            };
            Value::Float64(fraction)
        })
        .collect();
    out.insert("file_progress".to_owned(), Value::List(done));

    let wanted = entries
        .iter()
        .enumerate()
        .map(|(index, _)| Value::Int(i64::from(priorities.get(index).copied().unwrap_or(4))))
        .collect();
    out.insert("file_priorities".to_owned(), Value::List(wanted));
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use redeluge_libtorrent::FileEntry;

    fn entry(index: i32, path: &str, size: i64, offset: i64) -> FileEntry {
        FileEntry {
            index,
            path: path.to_owned(),
            size,
            offset,
        }
    }

    #[test]
    fn the_files_of_a_torrent_reach_the_status() {
        // Without these three keys `web.get_torrent_files` builds an empty
        // tree and the Files tab is blank for every torrent, which is what it
        // did.
        let mut out = BTreeMap::new();
        put_files(
            &mut out,
            &[
                entry(0, "video/episode.mkv", 1000, 0),
                entry(1, "video/subtitles.srt", 200, 1000),
            ],
            &[500, 200],
            &[4, 7],
        );

        let Some(Value::List(files)) = out.get("files") else {
            panic!("no files key");
        };
        assert_eq!(files.len(), 2);

        let Some(Value::List(progress)) = out.get("file_progress") else {
            panic!("no file_progress key");
        };
        // A fraction, not a byte count: half of the first, all of the second.
        assert_eq!(progress[0], Value::Float64(0.5));
        assert_eq!(progress[1], Value::Float64(1.0));

        let Some(Value::List(priorities)) = out.get("file_priorities") else {
            panic!("no file_priorities key");
        };
        assert_eq!(priorities[1], Value::Int(7));
    }

    #[test]
    fn a_file_of_no_length_is_complete_rather_than_a_division_by_zero() {
        let mut out = BTreeMap::new();
        put_files(&mut out, &[entry(0, "marker", 0, 0)], &[0], &[4]);

        let Some(Value::List(progress)) = out.get("file_progress") else {
            panic!("no file_progress key");
        };
        assert_eq!(progress[0], Value::Float64(1.0));
    }

    #[test]
    fn a_missing_priority_or_progress_is_filled_in_rather_than_dropped() {
        // libtorrent answers these as separate calls, so the three lists can
        // disagree for an instant after a torrent changes.
        let mut out = BTreeMap::new();
        put_files(
            &mut out,
            &[entry(0, "a", 100, 0), entry(1, "b", 100, 100)],
            &[50],
            &[],
        );

        let Some(Value::List(progress)) = out.get("file_progress") else {
            panic!("no file_progress key");
        };
        let Some(Value::List(priorities)) = out.get("file_priorities") else {
            panic!("no file_priorities key");
        };
        assert_eq!(progress.len(), 2);
        assert_eq!(progress[1], Value::Float64(0.0));
        assert_eq!(priorities.len(), 2);
        assert_eq!(priorities[0], Value::Int(4));
    }
}

#[cfg(test)]
mod fingerprint_tests {
    use super::content_fingerprint;
    use redeluge_libtorrent::FileEntry;

    fn file(path: &str, size: i64) -> FileEntry {
        FileEntry {
            index: 0,
            path: path.to_owned(),
            size,
            offset: 0,
        }
    }

    #[test]
    fn the_same_files_in_a_differently_named_folder_are_the_same_content() {
        // What a cross-seed looks like: one tracker's copy inside its own
        // folder, another's inside a folder named for the tracker.
        let one = vec![
            file("Some.Release/video.mkv", 1_500_000_000),
            file("Some.Release/readme.nfo", 4_096),
        ];
        let other = vec![
            file("Some.Release-TRACKER/readme.nfo", 4_096),
            file("Some.Release-TRACKER/video.mkv", 1_500_000_000),
        ];
        assert_eq!(content_fingerprint(&one), content_fingerprint(&other));
    }

    #[test]
    fn one_byte_of_difference_is_a_different_content() {
        let one = vec![file("video.mkv", 1_500_000_000)];
        let other = vec![file("video.mkv", 1_500_000_001)];
        assert_ne!(content_fingerprint(&one), content_fingerprint(&other));

        // And so is a different name at the same size.
        let renamed = vec![file("other.mkv", 1_500_000_000)];
        assert_ne!(content_fingerprint(&one), content_fingerprint(&renamed));
    }

    #[test]
    fn a_torrent_with_no_metadata_yet_has_no_fingerprint() {
        // A magnet before its metadata arrives has no file list, and an empty
        // fingerprint must not match another empty one.
        assert!(content_fingerprint(&[]).is_empty());
    }

    #[test]
    fn the_same_list_always_answers_the_same_thing() {
        let files = vec![file("a/one.bin", 10), file("a/two.bin", 20)];
        assert_eq!(content_fingerprint(&files), content_fingerprint(&files));
    }
}

#[cfg(test)]
mod unregistered_tests {
    use super::tracker_says_unregistered;

    #[test]
    fn the_phrases_trackers_use_for_a_torrent_they_have_dropped() {
        // As they arrive, which is with the daemon's own "Error: " in front.
        assert!(tracker_says_unregistered("Error: Unregistered torrent"));
        assert!(tracker_says_unregistered(
            "Error: torrent not registered with this tracker"
        ));
        assert!(tracker_says_unregistered("Error: Torrent not found"));
        assert!(tracker_says_unregistered("Error: unknown torrent"));
        assert!(tracker_says_unregistered("Error: info hash not found"));
        // Whatever case the tracker felt like.
        assert!(tracker_says_unregistered("ERROR: UNREGISTERED TORRENT"));
    }

    #[test]
    fn a_tracker_having_a_bad_day_is_not_a_torrent_that_is_gone() {
        // The distinction the whole thing rests on. Offering to delete a
        // library because a tracker was rebooting would be unforgivable.
        assert!(!tracker_says_unregistered("Error: Connection timed out"));
        assert!(!tracker_says_unregistered("Error: connection refused"));
        assert!(!tracker_says_unregistered(
            "Error: too many requests, slow down"
        ));
        assert!(!tracker_says_unregistered("Error: (403) Forbidden"));
        assert!(!tracker_says_unregistered("Announce OK (12 peers)"));
        assert!(!tracker_says_unregistered("Announce Sent"));
        assert!(!tracker_says_unregistered(""));
    }
}

#[cfg(test)]
mod tracker_host_tests {
    use super::tracker_host;

    #[test]
    fn a_subdomain_is_dropped_so_one_tracker_is_one_group() {
        // announce and tracker on the same site are the same tracker.
        assert_eq!(
            tracker_host("http://tracker.example.com/announce"),
            "example.com"
        );
        assert_eq!(
            tracker_host("udp://announce.example.com:6969"),
            "example.com"
        );
        assert_eq!(tracker_host("https://example.com/announce"), "example.com");
    }

    #[test]
    fn a_three_letter_second_level_is_still_dropped() {
        // The rule used to be a length heuristic, and this is the case it got
        // wrong: it kept the whole host, so the sidebar listed a name no
        // torrent was recorded under.
        assert_eq!(tracker_host("http://x.abc.com/announce"), "abc.com");
        assert_eq!(tracker_host("http://a.b.cd.info/announce"), "cd.info");
    }

    #[test]
    fn a_two_part_suffix_keeps_three_labels() {
        // Otherwise the answer is the suffix itself, and every British tracker
        // lands in one group called co.uk.
        assert_eq!(
            tracker_host("http://tracker.example.co.uk/announce"),
            "example.co.uk"
        );
        assert_eq!(tracker_host("http://a.b.net.au/announce"), "b.net.au");
        assert_eq!(
            tracker_host("http://tracker.example.org.uk/a"),
            "example.org.uk"
        );
    }

    #[test]
    fn an_address_is_not_a_name_and_keeps_every_part() {
        // The heuristic turned this into 168.1.1, which is not a host at all.
        assert_eq!(
            tracker_host("http://192.168.1.1:8080/announce"),
            "192.168.1.1"
        );
        assert_eq!(tracker_host("udp://10.0.0.1:6969"), "10.0.0.1");
        assert_eq!(
            tracker_host("http://[2001:db8::1]:8080/announce"),
            "2001:db8::1"
        );
    }

    #[test]
    fn credentials_and_ports_are_not_part_of_the_host() {
        assert_eq!(
            tracker_host("http://user:pass@tracker.example.com/a"),
            "example.com"
        );
        assert_eq!(
            tracker_host("http://example.com:1337/announce"),
            "example.com"
        );
    }

    #[test]
    fn no_tracker_is_an_empty_host_rather_than_a_word() {
        // It has to be empty, because this same string is the filter value the
        // sidebar sends back: a word here would filter to nothing.
        assert_eq!(tracker_host(""), "");
        assert_eq!(tracker_host("not a url"), "not a url");
    }

    #[test]
    fn the_hostname_keeps_what_the_group_drops() {
        // The tracker info window takes one sidebar row apart again, and the
        // subdomain is the only thing telling two of its trackers apart.
        use super::tracker_hostname;

        assert_eq!(
            tracker_hostname("http://tracker.example.com/announce"),
            "tracker.example.com"
        );
        assert_eq!(
            tracker_hostname("udp://announce.example.com:6969"),
            "announce.example.com"
        );
        // Everything that is not the host is still not the host.
        assert_eq!(
            tracker_hostname("http://user:pass@tracker.example.com/a"),
            "tracker.example.com"
        );
        assert_eq!(
            tracker_hostname("http://[2001:db8::1]:8080/announce"),
            "2001:db8::1"
        );
        assert_eq!(tracker_hostname(""), "");
    }
}
