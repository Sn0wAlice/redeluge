// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed view over the flattened alerts.

use crate::bridge::ffi::FlatAlert;

/// Every libtorrent alert the daemon handles.
///
/// The discriminants are the order `contract/alerts.json` lists them in, which
/// the C++ shim mirrors. `tests/contract.rs` fails if the two ever drift, so an
/// alert added on one side cannot silently go unhandled on the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum AlertKind {
    /// An alert libtorrent raised that the daemon has no handler for. Carried
    /// across rather than dropped so it shows up in logs.
    Unknown = 0,
    AddTorrent = 1,
    ExternalIp = 2,
    FastresumeRejected = 3,
    FileCompleted = 4,
    FileError = 5,
    FileRenamed = 6,
    MetadataReceived = 7,
    Performance = 8,
    SaveResumeData = 9,
    SaveResumeDataFailed = 10,
    SessionStats = 11,
    StateChanged = 12,
    StateUpdate = 13,
    StorageMoved = 14,
    StorageMovedFailed = 15,
    TorrentChecked = 16,
    TorrentFinished = 17,
    TorrentNeedCert = 18,
    TorrentPaused = 19,
    TorrentResumed = 20,
    TrackerAnnounce = 21,
    TrackerError = 22,
    TrackerReply = 23,
    TrackerWarning = 24,
}

impl AlertKind {
    /// Highest discriminant in use, excluding [`AlertKind::Unknown`].
    pub const COUNT: u16 = 24;

    pub fn from_raw(raw: u16) -> Self {
        match raw {
            1 => Self::AddTorrent,
            2 => Self::ExternalIp,
            3 => Self::FastresumeRejected,
            4 => Self::FileCompleted,
            5 => Self::FileError,
            6 => Self::FileRenamed,
            7 => Self::MetadataReceived,
            8 => Self::Performance,
            9 => Self::SaveResumeData,
            10 => Self::SaveResumeDataFailed,
            11 => Self::SessionStats,
            12 => Self::StateChanged,
            13 => Self::StateUpdate,
            14 => Self::StorageMoved,
            15 => Self::StorageMovedFailed,
            16 => Self::TorrentChecked,
            17 => Self::TorrentFinished,
            18 => Self::TorrentNeedCert,
            19 => Self::TorrentPaused,
            20 => Self::TorrentResumed,
            21 => Self::TrackerAnnounce,
            22 => Self::TrackerError,
            23 => Self::TrackerReply,
            24 => Self::TrackerWarning,
            _ => Self::Unknown,
        }
    }

    /// The name used in `contract/alerts.json`, without the `_alert` suffix.
    pub fn handler_key(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::AddTorrent => "add_torrent",
            Self::ExternalIp => "external_ip",
            Self::FastresumeRejected => "fastresume_rejected",
            Self::FileCompleted => "file_completed",
            Self::FileError => "file_error",
            Self::FileRenamed => "file_renamed",
            Self::MetadataReceived => "metadata_received",
            Self::Performance => "performance",
            Self::SaveResumeData => "save_resume_data",
            Self::SaveResumeDataFailed => "save_resume_data_failed",
            Self::SessionStats => "session_stats",
            Self::StateChanged => "state_changed",
            Self::StateUpdate => "state_update",
            Self::StorageMoved => "storage_moved",
            Self::StorageMovedFailed => "storage_moved_failed",
            Self::TorrentChecked => "torrent_checked",
            Self::TorrentFinished => "torrent_finished",
            Self::TorrentNeedCert => "torrent_need_cert",
            Self::TorrentPaused => "torrent_paused",
            Self::TorrentResumed => "torrent_resumed",
            Self::TrackerAnnounce => "tracker_announce",
            Self::TrackerError => "tracker_error",
            Self::TrackerReply => "tracker_reply",
            Self::TrackerWarning => "tracker_warning",
        }
    }

    /// Every handled alert, in contract order. Excludes [`AlertKind::Unknown`].
    pub fn all() -> impl Iterator<Item = Self> {
        (1..=Self::COUNT).map(Self::from_raw)
    }
}

/// One alert, with its payload named rather than positional.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub kind: AlertKind,
    /// libtorrent's own name for the alert, useful when `kind` is `Unknown`.
    pub what: String,
    /// libtorrent's rendered description. Always populated.
    pub message: String,
    /// Hex v1 infohash, or `None` when the alert is not about one torrent.
    pub info_hash: Option<String>,
    num_a: i64,
    num_b: i64,
    str_a: String,
    str_b: String,
    blob: Vec<u8>,
    counters: Vec<i64>,
}

impl Alert {
    pub(crate) fn from_flat(flat: FlatAlert) -> Self {
        Self {
            kind: AlertKind::from_raw(flat.kind),
            what: flat.what,
            message: flat.message,
            info_hash: (!flat.info_hash.is_empty()).then_some(flat.info_hash),
            num_a: flat.num_a,
            num_b: flat.num_b,
            str_a: flat.str_a,
            str_b: flat.str_b,
            blob: flat.blob,
            counters: flat.counters,
        }
    }

    /// Session counters, for [`AlertKind::SessionStats`].
    ///
    /// Their names are [`crate::Session::stat_names`], in the same order. This
    /// alert is the only way libtorrent hands them over.
    pub fn counters(&self) -> Option<&[i64]> {
        (self.kind == AlertKind::SessionStats).then_some(self.counters.as_slice())
    }

    /// The bencoded resume data, for [`AlertKind::SaveResumeData`].
    ///
    /// Opaque on purpose: it comes out of libtorrent and goes back into it. The
    /// daemon stores the bytes and never parses them.
    pub fn resume_data(&self) -> Option<&[u8]> {
        (self.kind == AlertKind::SaveResumeData && !self.blob.is_empty())
            .then_some(self.blob.as_slice())
    }

    /// Error text, for the alert kinds that carry one.
    pub fn error(&self) -> Option<&str> {
        match self.kind {
            AlertKind::AddTorrent
            | AlertKind::SaveResumeDataFailed
            | AlertKind::StorageMovedFailed => non_empty(&self.str_a),
            AlertKind::FileError | AlertKind::TrackerError => non_empty(&self.str_b),
            _ => None,
        }
    }

    /// Tracker URL, for the four tracker alerts.
    pub fn tracker_url(&self) -> Option<&str> {
        matches!(
            self.kind,
            AlertKind::TrackerAnnounce
                | AlertKind::TrackerError
                | AlertKind::TrackerReply
                | AlertKind::TrackerWarning
        )
        .then(|| non_empty(&self.str_a))
        .flatten()
    }

    /// Peers returned by a tracker, for [`AlertKind::TrackerReply`].
    pub fn num_peers(&self) -> Option<i64> {
        (self.kind == AlertKind::TrackerReply).then_some(self.num_a)
    }

    /// New and previous state, for [`AlertKind::StateChanged`].
    pub fn state_transition(&self) -> Option<(u8, u8)> {
        (self.kind == AlertKind::StateChanged).then_some((self.num_a as u8, self.num_b as u8))
    }

    /// File index, for the alerts scoped to one file in a torrent.
    pub fn file_index(&self) -> Option<i64> {
        matches!(self.kind, AlertKind::FileCompleted | AlertKind::FileRenamed).then_some(self.num_a)
    }

    /// New path, for [`AlertKind::FileRenamed`] and [`AlertKind::StorageMoved`].
    pub fn path(&self) -> Option<&str> {
        matches!(self.kind, AlertKind::FileRenamed | AlertKind::StorageMoved)
            .then(|| non_empty(&self.str_a))
            .flatten()
    }

    /// The external address libtorrent observed, for [`AlertKind::ExternalIp`].
    pub fn external_ip(&self) -> Option<&str> {
        (self.kind == AlertKind::ExternalIp)
            .then(|| non_empty(&self.str_a))
            .flatten()
    }
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}
