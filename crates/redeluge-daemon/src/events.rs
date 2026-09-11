// SPDX-License-Identifier: GPL-3.0-or-later
//! The events the daemon broadcasts.
//!
//! Every client subscribes to the ones it cares about and the daemon pushes
//! them unsolicited. The names and argument order are the wire contract:
//! `contract/events.json` has them, extracted from `deluge/event.py`, and a
//! test checks this list against it.

use redeluge_rencode::Value;

/// One event, ready to send.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    ClientDisconnected {
        session_id: i64,
    },
    ConfigValueChanged {
        key: String,
        value: Value,
    },
    CreateTorrentProgress {
        piece_count: i64,
        num_pieces: i64,
    },
    ExternalIp {
        external_ip: String,
    },
    NewVersionAvailable {
        new_release: String,
    },
    PreTorrentRemoved {
        torrent_id: String,
    },
    SessionPaused,
    SessionResumed,
    SessionStarted,
    TorrentAdded {
        torrent_id: String,
        from_state: bool,
    },
    TorrentFileCompleted {
        torrent_id: String,
        index: i64,
    },
    TorrentFileRenamed {
        torrent_id: String,
        index: i64,
        name: String,
    },
    TorrentFinished {
        torrent_id: String,
    },
    TorrentFolderRenamed {
        torrent_id: String,
        old: String,
        new: String,
    },
    TorrentQueueChanged,
    TorrentRemoved {
        torrent_id: String,
    },
    TorrentResumed {
        torrent_id: String,
    },
    TorrentStateChanged {
        torrent_id: String,
        state: String,
    },
    TorrentStorageMoved {
        torrent_id: String,
        path: String,
    },
    TorrentTrackerStatus {
        torrent_id: String,
        status: String,
    },
}

impl Event {
    /// The class name the wire uses, which is what a client subscribes to.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ClientDisconnected { .. } => "ClientDisconnectedEvent",
            Self::ConfigValueChanged { .. } => "ConfigValueChangedEvent",
            Self::CreateTorrentProgress { .. } => "CreateTorrentProgressEvent",
            Self::ExternalIp { .. } => "ExternalIPEvent",
            Self::NewVersionAvailable { .. } => "NewVersionAvailableEvent",
            Self::PreTorrentRemoved { .. } => "PreTorrentRemovedEvent",
            Self::SessionPaused => "SessionPausedEvent",
            Self::SessionResumed => "SessionResumedEvent",
            Self::SessionStarted => "SessionStartedEvent",
            Self::TorrentAdded { .. } => "TorrentAddedEvent",
            Self::TorrentFileCompleted { .. } => "TorrentFileCompletedEvent",
            Self::TorrentFileRenamed { .. } => "TorrentFileRenamedEvent",
            Self::TorrentFinished { .. } => "TorrentFinishedEvent",
            Self::TorrentFolderRenamed { .. } => "TorrentFolderRenamedEvent",
            Self::TorrentQueueChanged => "TorrentQueueChangedEvent",
            Self::TorrentRemoved { .. } => "TorrentRemovedEvent",
            Self::TorrentResumed { .. } => "TorrentResumedEvent",
            Self::TorrentStateChanged { .. } => "TorrentStateChangedEvent",
            Self::TorrentStorageMoved { .. } => "TorrentStorageMovedEvent",
            Self::TorrentTrackerStatus { .. } => "TorrentTrackerStatusEvent",
        }
    }

    /// The arguments, in the order the Python event's `__init__` takes them.
    ///
    /// Clients unpack these positionally, so the order is not cosmetic.
    pub fn args(&self) -> Vec<Value> {
        match self {
            Self::ClientDisconnected { session_id } => vec![Value::Int(*session_id)],
            Self::ConfigValueChanged { key, value } => {
                vec![Value::Str(key.clone()), value.clone()]
            }
            Self::CreateTorrentProgress {
                piece_count,
                num_pieces,
            } => vec![Value::Int(*piece_count), Value::Int(*num_pieces)],
            Self::ExternalIp { external_ip } => vec![Value::Str(external_ip.clone())],
            Self::NewVersionAvailable { new_release } => {
                vec![Value::Str(new_release.clone())]
            }
            Self::PreTorrentRemoved { torrent_id }
            | Self::TorrentFinished { torrent_id }
            | Self::TorrentRemoved { torrent_id }
            | Self::TorrentResumed { torrent_id } => vec![Value::Str(torrent_id.clone())],
            Self::SessionPaused | Self::SessionResumed | Self::SessionStarted => Vec::new(),
            Self::TorrentQueueChanged => Vec::new(),
            Self::TorrentAdded {
                torrent_id,
                from_state,
            } => vec![Value::Str(torrent_id.clone()), Value::Bool(*from_state)],
            Self::TorrentFileCompleted { torrent_id, index } => {
                vec![Value::Str(torrent_id.clone()), Value::Int(*index)]
            }
            Self::TorrentFileRenamed {
                torrent_id,
                index,
                name,
            } => vec![
                Value::Str(torrent_id.clone()),
                Value::Int(*index),
                Value::Str(name.clone()),
            ],
            Self::TorrentFolderRenamed {
                torrent_id,
                old,
                new,
            } => vec![
                Value::Str(torrent_id.clone()),
                Value::Str(old.clone()),
                Value::Str(new.clone()),
            ],
            Self::TorrentStateChanged { torrent_id, state } => {
                vec![Value::Str(torrent_id.clone()), Value::Str(state.clone())]
            }
            Self::TorrentStorageMoved { torrent_id, path } => {
                vec![Value::Str(torrent_id.clone()), Value::Str(path.clone())]
            }
            Self::TorrentTrackerStatus { torrent_id, status } => {
                vec![Value::Str(torrent_id.clone()), Value::Str(status.clone())]
            }
        }
    }

    /// Every event name the daemon can send.
    ///
    /// Two of the twenty-two in the contract are not here: the plugin events,
    /// which went with the plugin system.
    pub const NAMES: &'static [&'static str] = &[
        "ClientDisconnectedEvent",
        "ConfigValueChangedEvent",
        "CreateTorrentProgressEvent",
        "ExternalIPEvent",
        "NewVersionAvailableEvent",
        "PreTorrentRemovedEvent",
        "SessionPausedEvent",
        "SessionResumedEvent",
        "SessionStartedEvent",
        "TorrentAddedEvent",
        "TorrentFileCompletedEvent",
        "TorrentFileRenamedEvent",
        "TorrentFinishedEvent",
        "TorrentFolderRenamedEvent",
        "TorrentQueueChangedEvent",
        "TorrentRemovedEvent",
        "TorrentResumedEvent",
        "TorrentStateChangedEvent",
        "TorrentStorageMovedEvent",
        "TorrentTrackerStatusEvent",
    ];
}
