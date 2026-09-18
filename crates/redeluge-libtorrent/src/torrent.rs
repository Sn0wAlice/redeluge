// SPDX-License-Identifier: GPL-3.0-or-later
//! Torrents: their status, flags, files, trackers and peers.

use crate::bridge::ffi;

/// libtorrent's `torrent_status::state_t`, named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentState {
    CheckingFiles,
    DownloadingMetadata,
    Downloading,
    Finished,
    Seeding,
    CheckingResumeData,
    /// A state this build of libtorrent reports and this crate does not name.
    Other(u8),
}

impl From<u8> for TorrentState {
    fn from(raw: u8) -> Self {
        // libtorrent 2.0 dropped state 0 (queued_for_checking) but kept the
        // numbering, so the mapping starts at 1.
        match raw {
            1 => Self::CheckingFiles,
            2 => Self::DownloadingMetadata,
            3 => Self::Downloading,
            4 => Self::Finished,
            5 => Self::Seeding,
            7 => Self::CheckingResumeData,
            other => Self::Other(other),
        }
    }
}

/// The `torrent_flags` Deluge uses.
///
/// These are the bit values from libtorrent's `torrent_flags.hpp`. They are
/// part of the ABI rather than a detail: they appear in resume data, so they
/// cannot be renumbered.
pub mod flags {
    pub const SEED_MODE: u64 = 1 << 0;
    pub const UPLOAD_MODE: u64 = 1 << 1;
    pub const SHARE_MODE: u64 = 1 << 2;
    pub const APPLY_IP_FILTER: u64 = 1 << 3;
    pub const PAUSED: u64 = 1 << 4;
    pub const AUTO_MANAGED: u64 = 1 << 5;
    pub const DUPLICATE_IS_ERROR: u64 = 1 << 6;
    pub const UPDATE_SUBSCRIBE: u64 = 1 << 7;
    pub const SUPER_SEEDING: u64 = 1 << 8;
    pub const SEQUENTIAL_DOWNLOAD: u64 = 1 << 9;
    pub const STOP_WHEN_READY: u64 = 1 << 10;
    pub const OVERRIDE_TRACKERS: u64 = 1 << 11;
    pub const OVERRIDE_WEB_SEEDS: u64 = 1 << 12;
    pub const NEED_SAVE_RESUME: u64 = 1 << 13;
}

/// Which flags to turn on and which to turn off.
///
/// Both in one value, because setting and clearing separately leaves a window
/// where a torrent has neither state. For the paused flag that window means it
/// briefly starts, announces, and then stops again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlagChange {
    pub set: u64,
    pub unset: u64,
}

impl FlagChange {
    pub fn new() -> Self {
        Self::default()
    }

    /// Turns a flag on, cancelling any request to turn it off.
    pub fn on(mut self, flag: u64) -> Self {
        self.set |= flag;
        self.unset &= !flag;
        self
    }

    /// Turns a flag off, cancelling any request to turn it on.
    pub fn off(mut self, flag: u64) -> Self {
        self.unset |= flag;
        self.set &= !flag;
        self
    }

    /// Sets a flag to a value, which is what a configuration change does.
    pub fn set_to(self, flag: u64, value: bool) -> Self {
        if value {
            self.on(flag)
        } else {
            self.off(flag)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.set == 0 && self.unset == 0
    }
}

/// A snapshot of one torrent.
///
/// Every field is one the daemon reports. Fields libtorrent offers and Deluge
/// ignores are deliberately absent: crossing them costs on every status poll,
/// and status is polled constantly.
#[derive(Debug, Clone, PartialEq)]
pub struct TorrentStatus {
    pub info_hash: String,
    pub name: String,
    pub save_path: String,
    pub state: TorrentState,
    /// 0.0 to 1.0.
    pub progress: f32,
    /// Current `torrent_flags`, testable against [`flags`].
    pub flags: u64,

    pub download_rate: i32,
    pub upload_rate: i32,
    pub download_payload_rate: i32,
    pub upload_payload_rate: i32,

    pub num_peers: i32,
    pub num_seeds: i32,
    /// Seeds the tracker reports, which is more than we are connected to.
    pub num_complete: i32,
    pub num_incomplete: i32,
    pub connect_candidates: i32,

    pub total_done: i64,
    pub total_wanted: i64,
    pub total_wanted_done: i64,
    pub total_payload_download: i64,
    pub total_payload_upload: i64,
    pub all_time_download: i64,
    pub all_time_upload: i64,

    pub active_time: i64,
    pub seeding_time: i64,
    pub time_since_download: i64,
    pub time_since_upload: i64,
    /// Unix timestamps; 0 when it has not happened.
    pub added_time: i64,
    pub completed_time: i64,
    /// Seconds spent finished, not a timestamp, which libtorrent's naming hides.
    pub finished_time: i64,
    pub last_seen_complete: i64,
    pub next_announce: i64,

    pub distributed_copies: f32,
    pub queue_position: i32,
    pub seed_rank: i32,
    pub storage_mode: u8,

    pub is_finished: bool,
    pub is_seeding: bool,
    pub is_paused: bool,
    pub has_metadata: bool,
    pub moving_storage: bool,

    pub current_tracker: String,
    /// None when the torrent is healthy.
    pub error: Option<String>,
    /// The file the error is about, when it names one.
    pub error_file: Option<String>,

    /// One entry per piece, true when we have it. Empty without metadata.
    pub pieces: Vec<bool>,
    pub num_pieces: i32,
    pub piece_length: i32,
    pub total_size: i64,
    pub num_files: i32,
}

impl TorrentStatus {
    /// Whether a flag is currently set.
    pub fn has_flag(&self, flag: u64) -> bool {
        self.flags & flag != 0
    }

    pub fn is_auto_managed(&self) -> bool {
        self.has_flag(flags::AUTO_MANAGED)
    }

    pub fn is_sequential(&self) -> bool {
        self.has_flag(flags::SEQUENTIAL_DOWNLOAD)
    }

    /// Upload divided by download, the number a seeding rule is written against.
    ///
    /// A torrent with nothing downloaded has an undefined ratio rather than an
    /// infinite one, and returning infinity makes every "stop at ratio" rule
    /// fire at once.
    pub fn share_ratio(&self) -> Option<f64> {
        if self.all_time_download <= 0 {
            return None;
        }
        Some(self.all_time_upload as f64 / self.all_time_download as f64)
    }
}

impl From<ffi::TorrentStatus> for TorrentStatus {
    fn from(raw: ffi::TorrentStatus) -> Self {
        Self {
            info_hash: raw.info_hash,
            name: raw.name,
            save_path: raw.save_path,
            state: raw.state.into(),
            progress: raw.progress,
            flags: raw.flags,
            download_rate: raw.download_rate,
            upload_rate: raw.upload_rate,
            download_payload_rate: raw.download_payload_rate,
            upload_payload_rate: raw.upload_payload_rate,
            num_peers: raw.num_peers,
            num_seeds: raw.num_seeds,
            num_complete: raw.num_complete,
            num_incomplete: raw.num_incomplete,
            connect_candidates: raw.connect_candidates,
            total_done: raw.total_done,
            total_wanted: raw.total_wanted,
            total_wanted_done: raw.total_wanted_done,
            total_payload_download: raw.total_payload_download,
            total_payload_upload: raw.total_payload_upload,
            all_time_download: raw.all_time_download,
            all_time_upload: raw.all_time_upload,
            active_time: raw.active_time,
            seeding_time: raw.seeding_time,
            time_since_download: raw.time_since_download,
            time_since_upload: raw.time_since_upload,
            added_time: raw.added_time,
            completed_time: raw.completed_time,
            finished_time: raw.finished_time,
            last_seen_complete: raw.last_seen_complete,
            next_announce: raw.next_announce,
            distributed_copies: raw.distributed_copies,
            queue_position: raw.queue_position,
            seed_rank: raw.seed_rank,
            storage_mode: raw.storage_mode,
            is_finished: raw.is_finished,
            is_seeding: raw.is_seeding,
            is_paused: raw.is_paused,
            has_metadata: raw.has_metadata,
            moving_storage: raw.moving_storage,
            current_tracker: raw.current_tracker,
            error: (!raw.error.is_empty()).then_some(raw.error),
            error_file: (!raw.error_file.is_empty()).then_some(raw.error_file),
            pieces: raw.pieces.into_iter().map(|byte| byte != 0).collect(),
            num_pieces: raw.num_pieces,
            piece_length: raw.piece_length,
            total_size: raw.total_size,
            num_files: raw.num_files,
        }
    }
}

/// One file inside a torrent.
#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub index: i32,
    pub path: String,
    pub size: i64,
    /// Offset of this file within the torrent's single byte stream.
    pub offset: i64,
}

impl From<ffi::FileEntry> for FileEntry {
    fn from(raw: ffi::FileEntry) -> Self {
        Self {
            index: raw.index,
            path: raw.path,
            size: raw.size,
            offset: raw.offset,
        }
    }
}

/// One tracker in a torrent's announce list.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackerEntry {
    pub url: String,
    pub tier: u8,
    /// The last error or warning, when there is one.
    pub message: Option<String>,
    /// True when at least one endpoint has had an announce answered.
    ///
    /// Not the same as "is not failing": a tracker nobody has announced to yet
    /// is neither. libtorrent announces once per listen socket, so a host with
    /// an IPv6 socket and an IPv4-only tracker has one endpoint working and one
    /// failing, for ever, and this is the half that decides whether the tracker
    /// is up.
    pub verified: bool,
    pub updating: bool,
    /// Consecutive failures on the worst endpoint.
    pub fails: i32,
}

impl From<ffi::TrackerEntry> for TrackerEntry {
    fn from(raw: ffi::TrackerEntry) -> Self {
        Self {
            url: raw.url,
            tier: raw.tier,
            message: (!raw.message.is_empty()).then_some(raw.message),
            verified: raw.verified,
            updating: raw.updating,
            fails: raw.fails,
        }
    }
}

/// One peer connected to a torrent.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerInfo {
    pub ip: String,
    pub port: u16,
    pub client: String,
    pub peer_id: String,
    pub down_speed: i32,
    pub up_speed: i32,
    /// 0.0 to 1.0.
    pub progress: f32,
    pub seed: bool,
    /// Two-letter country code, when a GeoIP database said so.
    pub country: Option<String>,
    /// True when the connection is over uTP rather than TCP.
    pub utp: bool,
    /// True when the connection is encrypted, either scheme.
    pub encrypted: bool,
    /// How this peer was found: `tracker`, `DHT`, `PEX`, `LSD`, `resume` or
    /// `incoming`. Empty when libtorrent recorded no source.
    ///
    /// The one field that says whether a torrent still has a way of finding
    /// anybody: a swarm reachable only through a tracker dies with it, and one
    /// the DHT is still answering for does not.
    pub source: String,
    /// How many pieces this peer has that we do not.
    pub useful_pieces: i32,
    /// Bytes sent to and received from this peer, on this connection only.
    ///
    /// Both reset when the peer reconnects, so a running total has to be kept
    /// by whoever wants one: see the daemon's peer ledger.
    pub total_upload: i64,
    pub total_download: i64,
}

impl From<ffi::PeerInfo> for PeerInfo {
    fn from(raw: ffi::PeerInfo) -> Self {
        Self {
            ip: raw.ip,
            port: raw.port,
            client: raw.client,
            peer_id: raw.peer_id,
            down_speed: raw.down_speed,
            up_speed: raw.up_speed,
            progress: raw.progress,
            seed: raw.seed,
            country: (!raw.country.is_empty()).then_some(raw.country),
            utp: raw.utp,
            encrypted: raw.encrypted,
            source: raw.source,
            useful_pieces: raw.useful_pieces,
            total_upload: raw.total_upload,
            total_download: raw.total_download,
        }
    }
}

/// How to add a torrent.
#[derive(Debug, Clone, Default)]
pub struct AddTorrent {
    /// Bencoded `.torrent` contents.
    pub torrent_file: Vec<u8>,
    /// Magnet URI, when there is no file.
    pub magnet_uri: String,
    /// Resume data from a previous run.
    pub resume_data: Vec<u8>,
    pub save_path: String,
    /// Overrides the name in the metadata.
    pub name: String,
    /// One priority per file, 0 to 7. Empty leaves them at the default.
    pub file_priorities: Vec<u8>,
    /// Replaces the trackers in the metadata.
    pub trackers: Vec<String>,
    pub flags: FlagChange,
    /// Allocate the full size up front rather than writing sparsely.
    pub pre_allocate: bool,
}

impl AddTorrent {
    /// From a `.torrent` file's bytes.
    pub fn from_file(torrent_file: Vec<u8>, save_path: impl Into<String>) -> Self {
        Self {
            torrent_file,
            save_path: save_path.into(),
            ..Self::default()
        }
    }

    /// From a magnet URI.
    pub fn from_magnet(uri: impl Into<String>, save_path: impl Into<String>) -> Self {
        Self {
            magnet_uri: uri.into(),
            save_path: save_path.into(),
            ..Self::default()
        }
    }

    /// From resume data alone, which is the restart path.
    pub fn from_resume(resume_data: Vec<u8>) -> Self {
        Self {
            resume_data,
            ..Self::default()
        }
    }

    pub fn paused(mut self, paused: bool) -> Self {
        self.flags = self.flags.set_to(flags::PAUSED, paused);
        self
    }

    pub fn auto_managed(mut self, managed: bool) -> Self {
        self.flags = self.flags.set_to(flags::AUTO_MANAGED, managed);
        self
    }

    pub fn sequential(mut self, sequential: bool) -> Self {
        self.flags = self.flags.set_to(flags::SEQUENTIAL_DOWNLOAD, sequential);
        self
    }

    pub(crate) fn to_ffi(&self) -> ffi::AddTorrentRequest {
        ffi::AddTorrentRequest {
            torrent_file: self.torrent_file.clone(),
            magnet_uri: self.magnet_uri.clone(),
            resume_data: self.resume_data.clone(),
            save_path: self.save_path.clone(),
            name: self.name.clone(),
            file_priorities: self.file_priorities.clone(),
            trackers: self.trackers.clone(),
            flags_set: self.flags.set,
            flags_unset: self.flags.unset,
            pre_allocate: self.pre_allocate,
        }
    }
}
