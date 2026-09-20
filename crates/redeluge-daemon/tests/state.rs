// SPDX-License-Identifier: GPL-3.0-or-later
//! Deriving the state Deluge reports from libtorrent's.
//!
//! The order of the tests in `derive` is the logic, and getting it wrong is not
//! visible from the code: a paused torrent would show as queued, or a torrent
//! being moved would show as paused. So each rule gets its own case.

use redeluge_daemon::state::{derive, StateContext, TorrentState};
use redeluge_libtorrent::{flags, TorrentState as LtState, TorrentStatus as LtStatus};

fn status() -> LtStatus {
    LtStatus {
        info_hash: "a".repeat(40),
        name: "test".to_owned(),
        save_path: "/tmp".to_owned(),
        state: LtState::Downloading,
        progress: 0.0,
        flags: 0,
        download_rate: 0,
        upload_rate: 0,
        download_payload_rate: 0,
        upload_payload_rate: 0,
        num_peers: 0,
        num_seeds: 0,
        num_complete: 0,
        num_incomplete: 0,
        connect_candidates: 0,
        total_done: 0,
        total_wanted: 0,
        total_wanted_done: 0,
        total_payload_download: 0,
        total_payload_upload: 0,
        all_time_download: 0,
        all_time_upload: 0,
        active_time: 0,
        seeding_time: 0,
        time_since_download: 0,
        time_since_upload: 0,
        added_time: 0,
        completed_time: 0,
        finished_time: 0,
        last_seen_complete: 0,
        next_announce: 0,
        distributed_copies: 0.0,
        queue_position: 0,
        seed_rank: 0,
        storage_mode: 0,
        is_finished: false,
        is_seeding: false,
        is_paused: false,
        has_metadata: true,
        moving_storage: false,
        current_tracker: String::new(),
        error: None,
        error_file: None,
        num_pieces: 0,
        piece_length: 0,
        total_size: 0,
        num_files: 0,
    }
}

fn plain() -> StateContext {
    StateContext::default()
}

#[test]
fn an_error_outranks_everything_else() {
    let mut status = status();
    status.error = Some("disk full".to_owned());
    status.moving_storage = true;
    status.is_paused = true;
    assert_eq!(derive(&status, plain()), TorrentState::Error);
}

#[test]
fn an_error_the_daemon_raised_counts_too() {
    // A failed move is the daemon's own error; libtorrent reports nothing.
    let status = status();
    let context = StateContext {
        forced_error: true,
        ..plain()
    };
    assert_eq!(derive(&status, context), TorrentState::Error);
}

#[test]
fn a_move_outranks_a_pause() {
    let mut status = status();
    status.moving_storage = true;
    status.is_paused = true;
    assert_eq!(derive(&status, plain()), TorrentState::Moving);
}

#[test]
fn a_paused_auto_managed_torrent_is_queued_not_paused() {
    // This is the rule worth having a test for: the queue paused it, not the
    // user, and a client that showed "Paused" would invite someone to resume
    // something the queue is about to start anyway.
    let mut status = status();
    status.is_paused = true;
    status.flags = flags::AUTO_MANAGED;
    assert_eq!(derive(&status, plain()), TorrentState::Queued);
}

#[test]
fn a_paused_torrent_the_user_paused_is_paused() {
    let mut status = status();
    status.is_paused = true;
    status.flags = 0; // Pausing by hand clears auto-management.
    assert_eq!(derive(&status, plain()), TorrentState::Paused);
}

#[test]
fn a_paused_session_makes_everything_paused() {
    // Even a queued torrent: with the session paused, nothing is waiting its
    // turn, it is simply stopped.
    let mut status = status();
    status.is_paused = true;
    status.flags = flags::AUTO_MANAGED;
    let context = StateContext {
        session_paused: true,
        ..plain()
    };
    assert_eq!(derive(&status, context), TorrentState::Paused);

    // And a running torrent too.
    let running = status_running();
    assert_eq!(derive(&running, context), TorrentState::Paused);
}

fn status_running() -> LtStatus {
    let mut status = status();
    status.state = LtState::Downloading;
    status
}

#[test]
fn libtorrent_states_map_onto_deluge_states() {
    let cases = [
        (LtState::CheckingFiles, TorrentState::Checking),
        (LtState::CheckingResumeData, TorrentState::Checking),
        (LtState::DownloadingMetadata, TorrentState::Downloading),
        (LtState::Downloading, TorrentState::Downloading),
        (LtState::Finished, TorrentState::Seeding),
        (LtState::Seeding, TorrentState::Seeding),
    ];

    for (from, expected) in cases {
        let mut status = status();
        status.state = from;
        assert_eq!(derive(&status, plain()), expected, "{from:?}");
    }
}

#[test]
fn a_state_libtorrent_adds_later_does_not_panic() {
    let mut status = status();
    status.state = LtState::Other(99);
    assert_eq!(derive(&status, plain()), TorrentState::Downloading);
}

#[test]
fn the_state_names_are_the_ones_clients_filter_on() {
    // Clients build their sidebar from these strings and filter on them. A
    // rename here is a client that shows an empty list.
    let names: Vec<&str> = TorrentState::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Allocating",
            "Checking",
            "Downloading",
            "Seeding",
            "Paused",
            "Error",
            "Queued",
            "Moving",
        ]
    );

    for state in TorrentState::ALL {
        assert_eq!(TorrentState::from_name(state.as_str()), Some(state));
    }
    assert_eq!(TorrentState::from_name("Nonsense"), None);
}
