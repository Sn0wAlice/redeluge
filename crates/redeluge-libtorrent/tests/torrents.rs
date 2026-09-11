// SPDX-License-Identifier: GPL-3.0-or-later
//! Torrent operations against a real libtorrent, with a real torrent file.
//!
//! Everything runs offline: no DHT, no discovery, no port mapping, and the
//! trackers in the test torrent are never reached because announcing is not
//! part of any of these paths.

use std::time::Duration;

use redeluge_libtorrent::{
    flags, AddTorrent, Error, FlagChange, Session, SessionSettings, TorrentState,
};

/// A single-file torrent from the Python test data. 307 949 bytes in 10 pieces.
const TEST_TORRENT: &[u8] = include_bytes!("data/test.torrent");
const TEST_HASH: &str = "ab570cdd5a17ea1b61e970bb72047de141bce173";
const TEST_NAME: &str = "azcvsupdater_2.6.2.jar";

/// A multi-file torrent, for the file-level operations.
///
/// Its name says six files and it holds seven. That is the file it ships as in
/// the Python test data, and the count matters more than the name.
const DIR_TORRENT: &[u8] = include_bytes!("data/dir_with_6_files.torrent");
const DIR_FILE_COUNT: usize = 7;

fn session() -> Session {
    Session::new(&SessionSettings::offline()).expect("offline session should start")
}

fn with_test_torrent() -> (Session, String) {
    let mut session = session();
    let hash = session
        .add_torrent(&AddTorrent::from_file(
            TEST_TORRENT.to_vec(),
            "/tmp/redeluge-test",
        ))
        .expect("the test torrent should be accepted");
    (session, hash)
}

fn with_dir_torrent() -> (Session, String) {
    let mut session = session();
    let hash = session
        .add_torrent(&AddTorrent::from_file(
            DIR_TORRENT.to_vec(),
            "/tmp/redeluge-test",
        ))
        .expect("the directory torrent should be accepted");
    (session, hash)
}

// ---------------------------------------------------------------- adding

#[test]
fn a_torrent_file_is_added_with_its_metadata() {
    let (session, hash) = with_test_torrent();
    assert_eq!(hash, TEST_HASH);

    let status = session.torrent_status(&hash).unwrap();
    assert_eq!(status.name, TEST_NAME);
    assert!(status.has_metadata, "a torrent file carries its metadata");
    assert_eq!(status.total_size, 307_949);
    assert_eq!(status.num_pieces, 10);
    assert_eq!(status.piece_length, 32_768);
    assert_eq!(status.num_files, 1);
    assert_eq!(status.pieces.len(), 10, "one entry per piece");
    assert_eq!(status.save_path, "/tmp/redeluge-test");
}

#[test]
fn an_infohash_can_be_read_without_adding_the_torrent() {
    // The daemon needs this to reject a duplicate before it has a handle.
    assert_eq!(
        Session::torrent_file_info_hash(TEST_TORRENT).unwrap(),
        TEST_HASH
    );
    assert!(Session::torrent_file_info_hash(b"not a torrent").is_err());
    assert!(Session::torrent_file_info_hash(&[]).is_err());
}

#[test]
fn adding_the_same_torrent_twice_is_an_error() {
    // Quietly returning the existing handle would lose the caller's options,
    // and the daemon would think it had added a torrent it had not.
    let (mut session, _) = with_test_torrent();
    let again = session.add_torrent(&AddTorrent::from_file(
        TEST_TORRENT.to_vec(),
        "/tmp/elsewhere",
    ));
    assert!(again.is_err(), "a duplicate must be refused");
    assert_eq!(session.torrent_hashes().len(), 1);
}

#[test]
fn a_torrent_can_be_added_paused_and_with_priorities() {
    let mut session = session();
    let request = AddTorrent::from_file(DIR_TORRENT.to_vec(), "/tmp/redeluge-test")
        .paused(true)
        .auto_managed(false);
    let hash = session.add_torrent(&request).unwrap();

    let status = session.torrent_status(&hash).unwrap();
    assert!(status.is_paused, "it was added paused");
    assert!(!status.is_auto_managed());
}

#[test]
fn a_name_in_the_options_does_not_override_the_metadata() {
    // libtorrent uses add_torrent_params::name only when it has no metadata to
    // take a name from, so it applies to a bare magnet and not to a torrent
    // file. The daemon therefore cannot rename a torrent this way; renaming is
    // its own state, kept beside libtorrent rather than inside it.
    let mut session = session();
    let mut request = AddTorrent::from_file(TEST_TORRENT.to_vec(), "/tmp/redeluge-test");
    request.name = "renamed by the caller".to_owned();

    let hash = session.add_torrent(&request).unwrap();
    assert_eq!(
        session.torrent_status(&hash).unwrap().name,
        TEST_NAME,
        "the metadata name wins"
    );
}

#[test]
fn a_name_in_the_options_does_apply_to_a_bare_magnet() {
    let mut session = session();
    let mut request = AddTorrent::from_magnet(
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        "/tmp/redeluge-test",
    );
    request.name = "named by the caller".to_owned();

    let hash = session.add_torrent(&request).unwrap();
    assert_eq!(
        session.torrent_status(&hash).unwrap().name,
        "named by the caller"
    );
}

#[test]
fn a_save_path_is_required() {
    let mut session = session();
    let request = AddTorrent::from_file(TEST_TORRENT.to_vec(), "");
    assert!(session.add_torrent(&request).is_err());
    assert!(session.torrent_hashes().is_empty());
}

#[test]
fn a_corrupt_torrent_file_is_refused() {
    let mut session = session();
    for junk in [
        b"".as_slice(),
        b"d".as_slice(),
        b"not bencode".as_slice(),
        &TEST_TORRENT[..TEST_TORRENT.len() / 2],
    ] {
        assert!(session
            .add_torrent(&AddTorrent::from_file(junk.to_vec(), "/tmp"))
            .is_err());
    }
    assert!(session.torrent_hashes().is_empty());
}

// ----------------------------------------------------------------- status

#[test]
fn a_fresh_torrent_reports_a_plausible_status() {
    let (session, hash) = with_test_torrent();
    let status = session.torrent_status(&hash).unwrap();

    assert_eq!(status.info_hash, TEST_HASH);
    assert!(matches!(
        status.state,
        TorrentState::CheckingResumeData | TorrentState::CheckingFiles | TorrentState::Downloading
    ));
    assert_eq!(status.total_done, 0);
    assert_eq!(status.all_time_download, 0);
    assert_eq!(status.all_time_upload, 0);
    assert_eq!(status.num_peers, 0);
    assert!(status.error.is_none(), "a fresh torrent has no error");
    assert!(status.added_time > 0, "it was added just now");
    assert!(!status.moving_storage);
    assert!((0.0..=1.0).contains(&status.progress));
}

#[test]
fn a_torrent_with_no_download_has_no_share_ratio() {
    // Returning infinity here makes every "stop at ratio" rule fire at once.
    let (session, hash) = with_test_torrent();
    assert_eq!(session.torrent_status(&hash).unwrap().share_ratio(), None);
}

#[test]
fn all_torrent_status_returns_every_torrent_in_one_call() {
    let mut session = session();
    let first = session
        .add_torrent(&AddTorrent::from_file(TEST_TORRENT.to_vec(), "/tmp"))
        .unwrap();
    let second = session
        .add_torrent(&AddTorrent::from_file(DIR_TORRENT.to_vec(), "/tmp"))
        .unwrap();

    let all = session.all_torrent_status();
    assert_eq!(all.len(), 2);

    let hashes: Vec<&str> = all.iter().map(|s| s.info_hash.as_str()).collect();
    assert!(hashes.contains(&first.as_str()));
    assert!(hashes.contains(&second.as_str()));
}

// ------------------------------------------------------------------ flags

#[test]
fn flags_are_set_and_cleared() {
    let (mut session, hash) = with_test_torrent();

    session
        .set_flags(&hash, FlagChange::new().on(flags::SEQUENTIAL_DOWNLOAD))
        .unwrap();
    assert!(session.torrent_status(&hash).unwrap().is_sequential());

    session
        .set_flags(&hash, FlagChange::new().off(flags::SEQUENTIAL_DOWNLOAD))
        .unwrap();
    assert!(!session.torrent_status(&hash).unwrap().is_sequential());
}

#[test]
fn setting_and_clearing_in_one_call_leaves_no_window() {
    // Two calls would leave a moment where the torrent is neither paused nor
    // auto-managed, and it would start, announce and stop again.
    let (mut session, hash) = with_test_torrent();

    session
        .set_flags(
            &hash,
            FlagChange::new().on(flags::PAUSED).off(flags::AUTO_MANAGED),
        )
        .unwrap();

    let status = session.torrent_status(&hash).unwrap();
    assert!(status.is_paused);
    assert!(!status.is_auto_managed());
}

#[test]
fn a_flag_change_resolves_contradictions_in_favour_of_the_last_word() {
    let change = FlagChange::new().on(flags::PAUSED).off(flags::PAUSED);
    assert_eq!(change.set & flags::PAUSED, 0);
    assert_eq!(change.unset & flags::PAUSED, flags::PAUSED);

    let change = FlagChange::new().off(flags::PAUSED).on(flags::PAUSED);
    assert_eq!(change.set & flags::PAUSED, flags::PAUSED);
    assert_eq!(change.unset & flags::PAUSED, 0);

    assert!(FlagChange::new().is_empty());
}

// ------------------------------------------------------------------ files

#[test]
fn the_files_in_a_torrent_are_listed_in_order() {
    let (session, hash) = with_dir_torrent();
    let files = session.files(&hash).unwrap();

    assert_eq!(files.len(), DIR_FILE_COUNT);
    for (position, file) in files.iter().enumerate() {
        assert_eq!(file.index as usize, position, "indexes are positional");
        assert!(!file.path.is_empty());
        assert!(file.size >= 0);
    }

    // Offsets are the running total, which is what a piece-to-file map needs.
    let mut expected = 0;
    for file in &files {
        assert_eq!(file.offset, expected);
        expected += file.size;
    }
}

#[test]
fn file_priorities_are_read_and_written() {
    let (mut session, hash) = with_dir_torrent();

    let original = session.file_priorities(&hash).unwrap();
    assert_eq!(original.len(), DIR_FILE_COUNT);
    assert!(original.iter().all(|p| *p == 4), "the default is 4");

    // Priority changes are applied on libtorrent's own thread, not on the
    // calling one, so reading back immediately returns the old values. The
    // daemon must not confirm a change by reading it back straight away.
    let wanted = vec![0, 1, 4, 7, 4, 4, 2];
    session.prioritize_files(&hash, &wanted).unwrap();

    let mut applied = session.file_priorities(&hash).unwrap();
    for _ in 0..50 {
        if applied == wanted {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        applied = session.file_priorities(&hash).unwrap();
    }
    assert_eq!(applied, wanted, "the change never took effect");
}

#[test]
fn a_priority_above_seven_is_refused() {
    // Clamping would hide the mistake until someone wondered why a file never
    // downloaded.
    let (mut session, hash) = with_dir_torrent();
    assert!(session
        .prioritize_files(&hash, &[8, 4, 4, 4, 4, 4, 4])
        .is_err());
    assert!(session
        .prioritize_files(&hash, &[255; DIR_FILE_COUNT])
        .is_err());
}

#[test]
fn file_progress_has_one_entry_per_file() {
    let (session, hash) = with_dir_torrent();
    let progress = session.file_progress(&hash).unwrap();
    assert_eq!(progress.len(), DIR_FILE_COUNT);
    assert!(progress.iter().all(|bytes| *bytes == 0));
}

#[test]
fn renaming_a_file_raises_an_alert() {
    let (mut session, hash) = with_dir_torrent();
    session.rename_file(&hash, 0, "renamed.bin").unwrap();

    let mut renamed = None;
    for _ in 0..25 {
        session.wait_for_alert(Duration::from_millis(200));
        for alert in session.pop_alerts() {
            if alert.kind == redeluge_libtorrent::AlertKind::FileRenamed {
                renamed = Some(alert);
            }
        }
        if renamed.is_some() {
            break;
        }
    }

    let alert = renamed.expect("renaming must raise file_renamed_alert");
    assert_eq!(alert.file_index(), Some(0));
    assert_eq!(alert.path(), Some("renamed.bin"));
    assert_eq!(alert.info_hash.as_deref(), Some(hash.as_str()));
}

#[test]
fn renaming_needs_a_name_and_a_real_index() {
    let (mut session, hash) = with_dir_torrent();
    assert!(session.rename_file(&hash, 0, "").is_err());
    assert!(session.rename_file(&hash, -1, "name").is_err());
}

#[test]
fn piece_priorities_and_availability_have_one_entry_per_piece() {
    let (mut session, hash) = with_test_torrent();

    let priorities = session.piece_priorities(&hash).unwrap();
    assert_eq!(priorities.len(), 10);

    // Asynchronous, like file priorities.
    session.prioritize_pieces(&hash, &[7; 10]).unwrap();
    let mut applied = session.piece_priorities(&hash).unwrap();
    for _ in 0..50 {
        if applied == vec![7u8; 10] {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        applied = session.piece_priorities(&hash).unwrap();
    }
    assert_eq!(applied, vec![7u8; 10]);

    let availability = session.piece_availability(&hash).unwrap();
    assert_eq!(availability.len(), 10);
    assert!(
        availability.iter().all(|count| *count == 0),
        "no peers, so nobody has any piece"
    );
}

// --------------------------------------------------------------- trackers

#[test]
fn the_trackers_from_the_metadata_are_reported() {
    let (session, hash) = with_test_torrent();
    let trackers = session.trackers(&hash).unwrap();

    assert_eq!(trackers.len(), 2);
    assert!(trackers[0].url.starts_with("http://tracker.aelitis.com"));
    assert_eq!(trackers[0].fails, 0, "nothing has been tried yet");
    assert!(trackers[0].message.is_none());
}

#[test]
fn a_tracker_is_added_and_the_list_is_replaced() {
    let (mut session, hash) = with_test_torrent();

    session
        .add_tracker(&hash, "http://added.example/announce", 3)
        .unwrap();
    let trackers = session.trackers(&hash).unwrap();
    assert_eq!(trackers.len(), 3);
    let added = trackers
        .iter()
        .find(|t| t.url == "http://added.example/announce")
        .expect("the tracker should be there");
    assert_eq!(added.tier, 3);

    let replacement = vec![
        "http://one.example/announce".to_owned(),
        "http://two.example/announce".to_owned(),
    ];
    session
        .replace_trackers(&hash, &replacement, &[0, 1])
        .unwrap();

    let trackers = session.trackers(&hash).unwrap();
    assert_eq!(trackers.len(), 2);
    assert_eq!(trackers[0].url, "http://one.example/announce");
    assert_eq!(trackers[1].tier, 1);
}

#[test]
fn replacing_trackers_with_mismatched_tiers_is_refused() {
    let (mut session, hash) = with_test_torrent();
    let urls = vec!["http://one.example/announce".to_owned()];
    assert!(session.replace_trackers(&hash, &urls, &[0, 1]).is_err());

    // No tiers at all means tier 0 for everything, which is allowed.
    session.replace_trackers(&hash, &urls, &[]).unwrap();
    assert_eq!(session.trackers(&hash).unwrap()[0].tier, 0);
}

#[test]
fn an_empty_tracker_url_is_refused() {
    let (mut session, hash) = with_test_torrent();
    assert!(session.add_tracker(&hash, "", 0).is_err());
}

// ------------------------------------------------------------------ peers

#[test]
fn an_offline_torrent_has_no_peers() {
    let (session, hash) = with_test_torrent();
    assert!(session.peers(&hash).unwrap().is_empty());
}

#[test]
fn connecting_to_something_that_is_not_an_address_is_refused() {
    let (mut session, hash) = with_test_torrent();
    assert!(session.connect_peer(&hash, "not an ip", 6881).is_err());
    assert!(session.connect_peer(&hash, "", 6881).is_err());

    // A syntactically valid address is accepted; nothing is there to answer,
    // which is libtorrent's problem and not an error here.
    session.connect_peer(&hash, "127.0.0.1", 1).unwrap();
}

// ------------------------------------------------------------------ queue

#[test]
fn queue_positions_move() {
    let mut session = session();
    let first = session
        .add_torrent(&AddTorrent::from_file(TEST_TORRENT.to_vec(), "/tmp"))
        .unwrap();
    let second = session
        .add_torrent(&AddTorrent::from_file(DIR_TORRENT.to_vec(), "/tmp"))
        .unwrap();

    assert_eq!(session.queue_position(&first).unwrap(), 0);
    assert_eq!(session.queue_position(&second).unwrap(), 1);

    session.queue_top(&second).unwrap();
    assert_eq!(session.queue_position(&second).unwrap(), 0);
    assert_eq!(session.queue_position(&first).unwrap(), 1);

    session.queue_down(&second).unwrap();
    assert_eq!(session.queue_position(&second).unwrap(), 1);

    session.queue_up(&second).unwrap();
    assert_eq!(session.queue_position(&second).unwrap(), 0);

    session.queue_bottom(&second).unwrap();
    assert_eq!(session.queue_position(&second).unwrap(), 1);
}

// -------------------------------------------------------------- the rest

#[test]
fn limits_are_accepted_including_the_no_limit_value() {
    let (mut session, hash) = with_test_torrent();
    for limit in [1024, 0, -1, i32::MAX] {
        session.set_download_limit(&hash, limit).unwrap();
        session.set_upload_limit(&hash, limit).unwrap();
        session.set_max_connections(&hash, limit).unwrap();
        session.set_max_uploads(&hash, limit).unwrap();
    }
}

#[test]
fn the_torrent_file_can_be_read_back() {
    let (session, hash) = with_test_torrent();
    let rebuilt = session.torrent_file(&hash).unwrap();

    // Rebuilt from the parsed metadata, so equivalent rather than identical.
    // What must hold is that it parses back to the same torrent.
    assert!(!rebuilt.is_empty());
    assert_eq!(
        Session::torrent_file_info_hash(&rebuilt).unwrap(),
        TEST_HASH
    );
}

#[test]
fn moving_storage_needs_a_destination() {
    let (mut session, hash) = with_test_torrent();
    assert!(session.move_storage(&hash, "").is_err());
}

#[test]
fn control_operations_are_accepted() {
    let (mut session, hash) = with_test_torrent();
    session.force_recheck(&hash).unwrap();
    session.force_reannounce(&hash, 0).unwrap();
    session.scrape_tracker(&hash).unwrap();
    session.clear_error(&hash).unwrap();
    assert!(session.is_valid(&hash));
}

#[test]
fn every_operation_refuses_an_unknown_infohash() {
    // One missing guard is a crash on a torrent the user removed a moment ago,
    // so this walks the whole surface rather than a sample of it.
    let mut session = session();
    let missing = "0000000000000000000000000000000000000000";

    assert!(!session.is_valid(missing));
    assert!(matches!(
        session.torrent_status(missing),
        Err(Error::UnknownTorrent(_))
    ));

    assert!(session.files(missing).is_err());
    assert!(session.file_progress(missing).is_err());
    assert!(session.file_priorities(missing).is_err());
    assert!(session.piece_priorities(missing).is_err());
    assert!(session.piece_availability(missing).is_err());
    assert!(session.trackers(missing).is_err());
    assert!(session.peers(missing).is_err());
    assert!(session.queue_position(missing).is_err());
    assert!(session.torrent_file(missing).is_err());
    assert!(session.needs_resume_save(missing).is_err());

    assert!(session.pause_torrent(missing).is_err());
    assert!(session.resume_torrent(missing).is_err());
    assert!(session.remove_torrent(missing, false).is_err());
    assert!(session.force_recheck(missing).is_err());
    assert!(session.force_reannounce(missing, 0).is_err());
    assert!(session.scrape_tracker(missing).is_err());
    assert!(session.clear_error(missing).is_err());
    assert!(session.move_storage(missing, "/tmp").is_err());
    assert!(session
        .set_flags(missing, FlagChange::new().on(flags::PAUSED))
        .is_err());
    assert!(session.set_max_connections(missing, 1).is_err());
    assert!(session.set_max_uploads(missing, 1).is_err());
    assert!(session.set_download_limit(missing, 1).is_err());
    assert!(session.set_upload_limit(missing, 1).is_err());
    assert!(session.queue_top(missing).is_err());
    assert!(session.queue_up(missing).is_err());
    assert!(session.queue_down(missing).is_err());
    assert!(session.queue_bottom(missing).is_err());
    assert!(session.prioritize_files(missing, &[4]).is_err());
    assert!(session.prioritize_pieces(missing, &[4]).is_err());
    assert!(session.rename_file(missing, 0, "x").is_err());
    assert!(session
        .add_tracker(missing, "http://x.example/a", 0)
        .is_err());
    assert!(session.replace_trackers(missing, &[], &[]).is_err());
    assert!(session.connect_peer(missing, "127.0.0.1", 1).is_err());
    assert!(session.save_resume_data(missing, false).is_err());
    assert!(session
        .set_ssl_certificate(missing, b"", b"", b"", "")
        .is_err());
}
