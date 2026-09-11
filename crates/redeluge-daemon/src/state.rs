// SPDX-License-Identifier: GPL-3.0-or-later
//! The torrent states Deluge reports, and how they are derived.
//!
//! libtorrent has its own states and Deluge has eight of its own. They are not
//! the same set: libtorrent has no notion of "queued", and Deluge folds several
//! libtorrent states into one. The derivation here is
//! `deluge/core/torrent.py`'s `update_state`, in the same order, because the
//! order is the logic: an error outranks a move, a move outranks a pause, and
//! a paused torrent that is auto-managed is queued rather than paused.

use redeluge_libtorrent::{TorrentState as LtState, TorrentStatus as LtStatus};

/// What the daemon reports. These strings are part of the RPC contract: every
/// client filters and sorts on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TorrentState {
    Allocating,
    Checking,
    Downloading,
    Seeding,
    Paused,
    Error,
    Queued,
    Moving,
}

impl TorrentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allocating => "Allocating",
            Self::Checking => "Checking",
            Self::Downloading => "Downloading",
            Self::Seeding => "Seeding",
            Self::Paused => "Paused",
            Self::Error => "Error",
            Self::Queued => "Queued",
            Self::Moving => "Moving",
        }
    }

    /// Every state, in the order `deluge.common.TORRENT_STATE` lists them.
    ///
    /// Clients build their filter sidebar from this order.
    pub const ALL: [Self; 8] = [
        Self::Allocating,
        Self::Checking,
        Self::Downloading,
        Self::Seeding,
        Self::Paused,
        Self::Error,
        Self::Queued,
        Self::Moving,
    ];

    /// Parses the name the wire uses. Not `FromStr`: a bad name here is a
    /// missing case rather than a failure worth an error type.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == name)
    }
}

impl std::fmt::Display for TorrentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the state derivation needs beyond libtorrent's own status.
#[derive(Debug, Clone, Copy, Default)]
pub struct StateContext {
    /// Whether the whole session is paused, which makes every torrent paused
    /// regardless of its own flag.
    pub session_paused: bool,
    /// An error the daemon raised itself, such as a failed move. It outranks
    /// anything libtorrent reports.
    pub forced_error: bool,
}

/// Derives the state Deluge reports from libtorrent's status.
///
/// The order of the tests is the logic and must not be rearranged.
pub fn derive(status: &LtStatus, context: StateContext) -> TorrentState {
    if context.forced_error || status.error.is_some() {
        return TorrentState::Error;
    }
    if status.moving_storage {
        return TorrentState::Moving;
    }
    // A paused torrent that is still auto-managed has not been paused by the
    // user: the queue paused it, and the queue will start it again.
    if !context.session_paused && status.is_paused && status.is_auto_managed() {
        return TorrentState::Queued;
    }
    if context.session_paused || status.is_paused {
        return TorrentState::Paused;
    }

    match status.state {
        LtState::CheckingFiles | LtState::CheckingResumeData => TorrentState::Checking,
        LtState::DownloadingMetadata | LtState::Downloading => TorrentState::Downloading,
        LtState::Finished | LtState::Seeding => TorrentState::Seeding,
        // libtorrent gained or renamed a state. Reporting Downloading is wrong
        // in a visible way, which beats reporting something no client knows.
        LtState::Other(_) => TorrentState::Downloading,
    }
}
