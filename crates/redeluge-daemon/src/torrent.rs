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

        put("label", Value::Str(self.options.label.clone()));
        put("owner", Value::Str(self.options.owner.clone()));
        put("shared", Value::Bool(self.options.shared));
        put("private", Value::Bool(false));
        put("comment", Value::Str(String::new()));
        put("creator", Value::Str(String::new()));

        put("tracker", Value::Str(status.current_tracker.clone()));
        put(
            "tracker_host",
            Value::Str(tracker_host(&status.current_tracker)),
        );
        put("tracker_status", Value::Str(self.tracker_status.clone()));
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

    fn message(&self) -> String {
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
    fn ratio(&self, status: &LtStatus) -> f64 {
        if status.all_time_download <= 0 {
            return -1.0;
        }
        status.all_time_upload as f64 / status.all_time_download as f64
    }
}

/// The host part of a tracker URL, which clients group by.
fn tracker_host(url: &str) -> String {
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = without_scheme
        .split(['/', ':'])
        .next()
        .unwrap_or("")
        .to_owned();

    // Deluge drops the leading label of a three-part name so that
    // tracker.example.org and announce.example.org group together.
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() > 2 {
        let tail = &labels[labels.len() - 2..];
        // Not for two-part public suffixes like co.uk, where the result would
        // be the suffix itself.
        if tail[0].len() > 3 || tail[1].len() > 3 {
            return tail.join(".");
        }
        if labels.len() > 3 {
            return labels[labels.len() - 3..].join(".");
        }
    }
    host
}
