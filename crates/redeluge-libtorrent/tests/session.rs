// SPDX-License-Identifier: GPL-3.0-or-later
//! Integration tests against a real libtorrent session.
//!
//! Every test runs offline: no DHT, no local discovery, no port mapping, and a
//! magnet with no trackers. Nothing here touches the network, so these run in
//! the build container exactly as they run on a workstation.

use std::time::{Duration, Instant};

use redeluge_libtorrent::{
    libtorrent_version, Alert, AlertKind, Error, IpRange, Session, SessionSettings, TorrentState,
};

/// A magnet whose infohash is fixed, with no trackers so it cannot announce.
const TEST_MAGNET: &str =
    "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=redeluge-spike";
const TEST_HASH: &str = "0123456789abcdef0123456789abcdef01234567";

fn session() -> Session {
    Session::new(&SessionSettings::offline()).expect("offline session should start")
}

fn drain(session: &mut Session, kind: AlertKind, attempts: u32) -> Option<Alert> {
    for _ in 0..attempts {
        session.wait_for_alert(Duration::from_millis(200));
        if let Some(found) = session.pop_alerts().into_iter().find(|a| a.kind == kind) {
            return Some(found);
        }
    }
    None
}

#[test]
fn links_against_libtorrent_2() {
    let version = libtorrent_version();
    assert!(
        version.starts_with("2."),
        "expected libtorrent 2.x, linked against {version}"
    );
}

#[test]
fn session_starts_and_stops_cleanly() {
    let session = session();
    assert!(session.torrent_hashes().is_empty());
    drop(session);
}

#[test]
fn add_magnet_returns_the_infohash_from_the_uri() {
    let mut session = session();
    let hash = session
        .add_magnet(TEST_MAGNET, "/tmp")
        .expect("a well-formed magnet should be accepted");
    assert_eq!(hash, TEST_HASH);
    assert_eq!(session.torrent_hashes(), vec![TEST_HASH.to_owned()]);
}

#[test]
fn a_magnet_without_metadata_reports_as_such() {
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    let status = session.torrent_status(&hash).expect("status should exist");
    assert_eq!(status.info_hash, TEST_HASH);
    assert_eq!(status.save_path, "/tmp");
    assert!(!status.has_metadata, "a bare magnet has no metadata yet");
    assert_eq!(status.state, TorrentState::DownloadingMetadata);
    assert_eq!(status.total_done, 0);
    assert!((0.0..=1.0).contains(&status.progress));
}

#[test]
fn adding_a_torrent_raises_an_add_torrent_alert() {
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    let alert = drain(&mut session, AlertKind::AddTorrent, 25)
        .expect("adding a torrent must raise add_torrent_alert");

    assert_eq!(alert.info_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(alert.what, "add_torrent");
    assert!(
        !alert.message.is_empty(),
        "libtorrent always renders a message"
    );
    assert_eq!(alert.error(), None, "this add should not have failed");
}

#[test]
fn pause_and_resume_are_reflected_in_the_status() {
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    session.pause_torrent(&hash).unwrap();
    assert!(session.torrent_status(&hash).unwrap().is_paused);

    session.resume_torrent(&hash).unwrap();
    assert!(!session.torrent_status(&hash).unwrap().is_paused);
}

#[test]
fn pausing_a_started_torrent_raises_a_torrent_paused_alert() {
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    // libtorrent posts this alert on a state transition, not on the call. A
    // torrent paused before it has started never transitions and so never
    // reports, which the daemon has to account for when it restores a paused
    // session at boot.
    drain(&mut session, AlertKind::TorrentResumed, 25)
        .expect("a freshly added torrent should start");

    session.pause_torrent(&hash).unwrap();
    let alert = drain(&mut session, AlertKind::TorrentPaused, 25)
        .expect("pausing a started torrent must raise torrent_paused_alert");
    assert_eq!(alert.info_hash.as_deref(), Some(hash.as_str()));
}

#[test]
fn alerts_this_crate_does_not_handle_still_cross_intact() {
    // libtorrent raises more than the daemon subscribes to, listen_succeeded
    // among them. Those must arrive as Unknown with their name and message
    // rather than being dropped, otherwise a new alert is invisible in logs.
    let mut session = session();
    session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    let mut unknown = Vec::new();
    for _ in 0..25 {
        session.wait_for_alert(Duration::from_millis(100));
        for alert in session.pop_alerts() {
            if alert.kind == AlertKind::Unknown {
                unknown.push(alert);
            }
        }
    }

    assert!(
        !unknown.is_empty(),
        "a starting session always raises at least one unhandled alert"
    );
    for alert in unknown {
        assert!(!alert.what.is_empty(), "an unhandled alert lost its name");
        assert!(
            !alert.message.is_empty(),
            "an unhandled alert lost its message"
        );
        assert_ne!(alert.what, "unknown", "the name should be libtorrent's own");
    }
}

#[test]
fn removing_a_torrent_forgets_it() {
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    session.remove_torrent(&hash, false).unwrap();

    assert!(session.torrent_hashes().is_empty());
    match session.torrent_status(&hash) {
        Err(Error::UnknownTorrent(reported)) => assert_eq!(reported, hash),
        other => panic!("expected UnknownTorrent, got {other:?}"),
    }
}

#[test]
fn an_unknown_infohash_is_an_error_not_a_panic() {
    let session = session();
    match session.torrent_status("deadbeef") {
        Err(Error::UnknownTorrent(hash)) => assert_eq!(hash, "deadbeef"),
        other => panic!("expected UnknownTorrent, got {other:?}"),
    }
}

#[test]
fn a_malformed_magnet_is_rejected_as_an_error() {
    let mut session = session();
    match session.add_magnet("not-a-magnet", "/tmp") {
        Err(Error::Libtorrent(message)) => {
            assert!(
                message.contains("invalid magnet uri"),
                "unexpected error text: {message}"
            );
        }
        other => panic!("expected a Libtorrent error, got {other:?}"),
    }
    assert!(
        session.torrent_hashes().is_empty(),
        "a rejected magnet must not leave a handle behind"
    );
}

#[test]
fn a_cpp_exception_does_not_take_the_process_down() {
    // The whole point of routing errors through Result: an exception thrown in
    // the shim has to arrive as a value. Provoke several in a row.
    let mut session = session();
    for _ in 0..50 {
        assert!(session.torrent_status("nope").is_err());
        assert!(session.pause_torrent("nope").is_err());
        assert!(session.remove_torrent("nope", true).is_err());
    }
    assert!(session.add_magnet(TEST_MAGNET, "/tmp").is_ok());
}

#[test]
fn two_sessions_can_run_at_once() {
    // The daemon will eventually run one session, but a leaked global in the
    // shim would show up here first.
    let mut first = session();
    let second = session();

    let hash = first.add_magnet(TEST_MAGNET, "/tmp").unwrap();
    assert_eq!(first.torrent_hashes().len(), 1);
    assert!(
        second.torrent_hashes().is_empty(),
        "sessions must not share a handle table"
    );
    assert!(second.torrent_status(&hash).is_err());
}

#[test]
fn a_session_can_be_moved_to_another_thread() {
    // Session is Send so the daemon can park the alert loop on its own thread,
    // the same shape the Python AlertManager uses.
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    let joined = std::thread::spawn(move || {
        let status = session.torrent_status(&hash).unwrap();
        (session, status.info_hash)
    })
    .join()
    .expect("the worker thread should not panic");

    assert_eq!(joined.1, TEST_HASH);
}

#[test]
fn every_alert_that_arrives_carries_a_message() {
    let mut session = session();
    session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    let mut seen = 0;
    for _ in 0..25 {
        session.wait_for_alert(Duration::from_millis(100));
        for alert in session.pop_alerts() {
            assert!(
                !alert.message.is_empty(),
                "{} arrived with an empty message",
                alert.what
            );
            assert!(!alert.what.is_empty(), "an alert arrived with no name");
            seen += 1;
        }
    }
    assert!(
        seen > 0,
        "a session that added a torrent should raise alerts"
    );
}

#[test]
fn wait_for_alert_is_bounded_by_its_timeout() {
    // The daemon parks its alert thread on this call, so what matters is that
    // it always comes back, not what it reports. Asserting on the return value
    // instead would be timing-dependent: a session raises listen_succeeded
    // while it is still starting, so an "idle" session is not reliably idle.
    let mut session = session();
    for _ in 0..10 {
        session.wait_for_alert(Duration::from_millis(50));
        session.pop_alerts();
    }

    for timeout in [Duration::from_millis(0), Duration::from_millis(250)] {
        let started = Instant::now();
        session.wait_for_alert(timeout);
        let elapsed = started.elapsed();
        assert!(
            elapsed < timeout + Duration::from_secs(2),
            "wait_for_alert({timeout:?}) took {elapsed:?}"
        );
    }
}

// ------------------------------------------------------------- resume data

#[test]
fn resume_data_is_requested_asynchronously_and_arrives_as_bencode() {
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();
    drain(&mut session, AlertKind::TorrentResumed, 25)
        .expect("a freshly added torrent should start");

    session.save_resume_data(&hash, false).unwrap();

    let alert = drain(&mut session, AlertKind::SaveResumeData, 25)
        .expect("save_resume_data must answer with an alert");
    let blob = alert
        .resume_data()
        .expect("the save_resume_data alert must carry the bytes");

    assert_eq!(alert.info_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(
        blob.first(),
        Some(&b'd'),
        "libtorrent writes resume data as a bencoded dictionary"
    );
    assert!(blob.len() > 20, "a resume blob this small holds nothing");
}

#[test]
fn resume_data_round_trips_through_a_removal() {
    // The restart path in miniature: save, forget the torrent, bring it back
    // from the bytes alone. No magnet URI involved the second time.
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/var/tmp").unwrap();
    drain(&mut session, AlertKind::TorrentResumed, 25).expect("torrent should start");

    session.save_resume_data(&hash, false).unwrap();
    let blob = drain(&mut session, AlertKind::SaveResumeData, 25)
        .and_then(|alert| alert.resume_data().map(<[u8]>::to_vec))
        .expect("resume data should arrive");

    session.remove_torrent(&hash, false).unwrap();
    assert!(session.torrent_hashes().is_empty());

    let restored = session
        .add_torrent_from_resume(&blob, "")
        .expect("resume data alone should be enough to re-add the torrent");

    assert_eq!(restored, hash);
    let status = session.torrent_status(&restored).unwrap();
    assert_eq!(
        status.save_path, "/var/tmp",
        "an empty save_path must keep the one in the resume data"
    );
}

#[test]
fn resume_data_can_relocate_a_torrent() {
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/var/tmp").unwrap();
    drain(&mut session, AlertKind::TorrentResumed, 25).expect("torrent should start");

    session.save_resume_data(&hash, false).unwrap();
    let blob = drain(&mut session, AlertKind::SaveResumeData, 25)
        .and_then(|alert| alert.resume_data().map(<[u8]>::to_vec))
        .expect("resume data should arrive");

    session.remove_torrent(&hash, false).unwrap();
    session.add_torrent_from_resume(&blob, "/tmp").unwrap();

    assert_eq!(session.torrent_status(&hash).unwrap().save_path, "/tmp");
}

#[test]
fn unreadable_resume_data_is_an_error_not_a_crash() {
    let mut session = session();

    for junk in [
        b"".as_slice(),
        b"not bencode".as_slice(),
        b"d".as_slice(),
        &[0xff; 64],
    ] {
        assert!(
            session.add_torrent_from_resume(junk, "/tmp").is_err(),
            "{junk:?} should have been rejected"
        );
    }
    assert!(session.torrent_hashes().is_empty());
}

#[test]
fn needs_resume_save_answers_for_a_known_torrent() {
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();

    // The value itself is libtorrent's business; what matters is that asking
    // works, because the daemon's periodic save gates on it.
    let _ = session
        .needs_resume_save(&hash)
        .expect("a known torrent answers");

    assert!(matches!(
        session.needs_resume_save("deadbeef"),
        Err(Error::UnknownTorrent(_))
    ));
}

#[test]
fn only_the_resume_alert_carries_a_blob() {
    // The payload slot is shared across alert kinds, so a stray blob on another
    // alert would mean the shim is leaking state between them.
    let mut session = session();
    let hash = session.add_magnet(TEST_MAGNET, "/tmp").unwrap();
    drain(&mut session, AlertKind::TorrentResumed, 25).expect("torrent should start");
    session.save_resume_data(&hash, false).unwrap();

    for _ in 0..25 {
        session.wait_for_alert(Duration::from_millis(100));
        for alert in session.pop_alerts() {
            if alert.kind != AlertKind::SaveResumeData {
                assert_eq!(
                    alert.resume_data(),
                    None,
                    "{} should not carry resume data",
                    alert.what
                );
            }
        }
    }
}

#[test]
fn saving_resume_data_for_an_unknown_torrent_is_an_error() {
    let mut session = session();
    assert!(matches!(
        session.save_resume_data("deadbeef", false),
        Err(Error::UnknownTorrent(_))
    ));
}

// ----------------------------------------------------------------- ip filter

fn blocked(first: &str, last: &str) -> IpRange {
    IpRange {
        first: first.to_owned(),
        last: last.to_owned(),
        blocked: true,
    }
}

#[test]
fn an_empty_filter_holds_one_range_per_address_family() {
    // The floor the other tests are measured against: libtorrent always has a
    // filter, and an empty one covers everything and allows it.
    let session = session();
    assert_eq!(session.ip_filter_ranges(), 2);
}

#[test]
fn blocking_a_range_splits_the_address_space() {
    let mut session = session();
    session
        .set_ip_filter(&[blocked("1.2.3.0", "1.2.3.255")])
        .unwrap();

    // Below the range, the range itself, above it, and the whole of v6.
    assert_eq!(session.ip_filter_ranges(), 4);
}

#[test]
fn clearing_the_filter_puts_the_address_space_back() {
    let mut session = session();
    session
        .set_ip_filter(&[blocked("10.0.0.0", "10.255.255.255")])
        .unwrap();
    assert!(session.ip_filter_ranges() > 2);

    session.clear_ip_filter().unwrap();
    assert_eq!(session.ip_filter_ranges(), 2);
}

#[test]
fn a_later_rule_wins_where_it_overlaps_an_earlier_one() {
    // This is what a whitelist is: a blocked range with an allowed hole in it,
    // and the hole has to be applied second to survive.
    let mut session = session();
    session
        .set_ip_filter(&[
            blocked("1.0.0.0", "1.255.255.255"),
            IpRange {
                first: "1.2.3.4".to_owned(),
                last: "1.2.3.4".to_owned(),
                blocked: false,
            },
        ])
        .unwrap();

    // Blocked below the hole, the hole, blocked above it, everything under and
    // over the blocked range, and v6.
    assert_eq!(session.ip_filter_ranges(), 6);
}

#[test]
fn ipv6_ranges_are_accepted() {
    let mut session = session();
    session
        .set_ip_filter(&[blocked("2001:db8::", "2001:db8::ffff")])
        .unwrap();
    assert_eq!(session.ip_filter_ranges(), 4);
}

#[test]
fn a_range_that_is_not_an_address_is_refused() {
    let mut session = session();
    assert!(session
        .set_ip_filter(&[blocked("not an address", "1.2.3.4")])
        .is_err());
}

#[test]
fn a_range_that_mixes_address_families_is_refused() {
    // libtorrent asserts on this rather than reporting it, and an assert is
    // compiled out of a release build, so the shim checks it first.
    let mut session = session();
    assert!(session
        .set_ip_filter(&[blocked("1.2.3.4", "2001:db8::1")])
        .is_err());
}

#[test]
fn a_range_that_ends_before_it_starts_is_refused() {
    let mut session = session();
    assert!(session
        .set_ip_filter(&[blocked("1.2.3.9", "1.2.3.1")])
        .is_err());
}

/// Every session statistic is named at the index its value sits at.
///
/// `session_stats_alert` carries one flat array and each metric says where in
/// it to look. Reading the metric list in order and counting along it assumes
/// the two agree; they do not, because the counters and the gauges are
/// numbered in separate ranges. Everything past the point where they diverge
/// then reported somebody else's value: the count of connected peers came out
/// as six figures, and the DHT node count in the tens of thousands.
#[test]
fn a_statistic_is_named_where_its_value_lives() {
    let names = redeluge_libtorrent::Session::stat_names();
    assert!(!names.is_empty(), "libtorrent should publish some metrics");

    // The gauges the interface shows, and two counters, checked against
    // libtorrent's own answer for where each one is.
    for name in [
        "peer.num_peers_connected",
        "dht.dht_nodes",
        "net.recv_bytes",
        "net.sent_bytes",
        "peer.incoming_connections",
        "disk.num_blocks_read",
    ] {
        let index = redeluge_libtorrent::Session::stat_index(name);
        assert!(index >= 0, "libtorrent does not know {name}");
        assert_eq!(
            names.get(index as usize).map(String::as_str),
            Some(name),
            "{name} should be named at index {index}"
        );
    }

    // And every name that is there is where libtorrent says it is, so this
    // cannot drift for one metric while the six above stay right.
    for (index, name) in names.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        assert_eq!(
            redeluge_libtorrent::Session::stat_index(name),
            index as i32,
            "{name} is listed at {index} and libtorrent puts it elsewhere"
        );
    }
}
