// SPDX-License-Identifier: GPL-3.0-or-later
//! Session settings.
//!
//! A setting that silently does nothing is the failure mode that matters here:
//! a rate limit that stopped applying looks exactly like a fast connection
//! until someone checks the bill. So an unknown name and a wrong type are both
//! errors, and both are tested.

use redeluge_libtorrent::settings::names;
use redeluge_libtorrent::{Session, SessionSettings, Setting};

fn session() -> Session {
    Session::new(&SessionSettings::offline()).expect("offline session should start")
}

#[test]
fn an_integer_setting_applies_and_reads_back() {
    let mut session = session();
    session
        .apply_setting(Setting::int("download_rate_limit", 1_500_000))
        .unwrap();
    assert_eq!(
        session.setting_int("download_rate_limit").unwrap(),
        1_500_000
    );
}

#[test]
fn a_rate_limit_of_minus_one_reads_back_as_zero() {
    // Deluge writes -1 for "no limit" and libtorrent normalises it to 0, which
    // is how libtorrent spells the same thing. So a setting does not always
    // read back as written, and the daemon must not compare the two to decide
    // whether a change took effect.
    let mut session = session();
    session
        .apply_setting(Setting::int("upload_rate_limit", -1))
        .unwrap();
    assert_eq!(session.setting_int("upload_rate_limit").unwrap(), 0);

    session
        .apply_setting(Setting::int("download_rate_limit", -1))
        .unwrap();
    assert_eq!(session.setting_int("download_rate_limit").unwrap(), 0);
}

#[test]
fn a_boolean_setting_applies_and_reads_back() {
    let mut session = session();
    for value in [true, false, true] {
        session
            .apply_setting(Setting::boolean("announce_to_all_tiers", value))
            .unwrap();
        assert_eq!(
            session.setting_bool("announce_to_all_tiers").unwrap(),
            value
        );
    }
}

#[test]
fn a_string_setting_applies_and_reads_back() {
    let mut session = session();
    session
        .apply_setting(Setting::string("user_agent", "redeluge/test"))
        .unwrap();
    assert_eq!(session.setting_str("user_agent").unwrap(), "redeluge/test");
}

#[test]
fn a_whole_pack_applies_at_once() {
    // libtorrent applies a pack atomically. Applying one at a time would let
    // the session run briefly in a state nobody asked for.
    let mut session = session();
    session
        .apply_settings(&[
            Setting::int("connections_limit", 321),
            Setting::boolean("enable_dht", false),
            Setting::string("user_agent", "redeluge/pack"),
            Setting::int("active_downloads", 7),
        ])
        .unwrap();

    assert_eq!(session.setting_int("connections_limit").unwrap(), 321);
    assert!(!session.setting_bool("enable_dht").unwrap());
    assert_eq!(session.setting_str("user_agent").unwrap(), "redeluge/pack");
    assert_eq!(session.setting_int("active_downloads").unwrap(), 7);
}

#[test]
fn every_setting_deluge_uses_exists_with_the_type_we_think() {
    // This is the test that earns its place: it checks the names and types the
    // daemon will apply against the libtorrent it is actually linked to. A
    // renamed setting shows up here rather than as a limit that stopped working.
    let mut session = session();

    for name in names::INT {
        session
            .setting_int(name)
            .unwrap_or_else(|err| panic!("int setting `{name}`: {err}"));
    }
    for name in names::BOOL {
        session
            .setting_bool(name)
            .unwrap_or_else(|err| panic!("bool setting `{name}`: {err}"));
    }
    for name in names::STR {
        session
            .setting_str(name)
            .unwrap_or_else(|err| panic!("string setting `{name}`: {err}"));
    }

    // And they all apply, not just read.
    let pack: Vec<Setting> = names::INT
        .iter()
        .map(|name| Setting::int(*name, 1))
        .chain(
            names::BOOL
                .iter()
                .map(|name| Setting::boolean(*name, false)),
        )
        .chain(names::STR.iter().map(|name| Setting::string(*name, "")))
        .collect();
    session
        .apply_settings(&pack)
        .expect("the whole pack applies");
}

#[test]
fn an_unknown_setting_name_is_refused() {
    let mut session = session();
    let err = session
        .apply_setting(Setting::int("no_such_setting", 1))
        .expect_err("an unknown name must not be ignored");
    assert!(
        err.to_string().contains("no_such_setting"),
        "the error should name the setting: {err}"
    );

    assert!(session.setting_int("no_such_setting").is_err());
    assert!(session.setting_bool("no_such_setting").is_err());
    assert!(session.setting_str("no_such_setting").is_err());
}

#[test]
fn using_a_setting_as_the_wrong_type_is_refused() {
    let mut session = session();

    // download_rate_limit is an int, enable_dht a bool, user_agent a string.
    assert!(session
        .apply_setting(Setting::boolean("download_rate_limit", true))
        .is_err());
    assert!(session
        .apply_setting(Setting::int("enable_dht", 1))
        .is_err());
    assert!(session
        .apply_setting(Setting::int("user_agent", 1))
        .is_err());

    assert!(session.setting_bool("download_rate_limit").is_err());
    assert!(session.setting_int("enable_dht").is_err());

    // And the session still works afterwards.
    session
        .apply_setting(Setting::int("download_rate_limit", 42))
        .unwrap();
    assert_eq!(session.setting_int("download_rate_limit").unwrap(), 42);
}

#[test]
fn a_value_too_large_for_libtorrent_is_refused_rather_than_wrapped() {
    // libtorrent settings are 32-bit. Silently wrapping a large value would set
    // a limit the operator did not ask for, possibly a negative one.
    let mut session = session();
    let err = session
        .apply_setting(Setting::int("download_rate_limit", i64::from(i32::MAX) + 1))
        .expect_err("an out-of-range value must be refused");
    assert!(err.to_string().contains("range"), "unexpected error: {err}");

    // The boundary itself is fine.
    session
        .apply_setting(Setting::int("download_rate_limit", i64::from(i32::MAX)))
        .unwrap();
}

#[test]
fn a_failed_pack_leaves_the_session_untouched() {
    // The pack is validated before any of it is applied, so one bad entry does
    // not leave half the settings changed.
    let mut session = session();
    session
        .apply_setting(Setting::int("connections_limit", 200))
        .unwrap();

    assert!(session
        .apply_settings(&[
            Setting::int("connections_limit", 999),
            Setting::int("no_such_setting", 1),
        ])
        .is_err());

    assert_eq!(
        session.setting_int("connections_limit").unwrap(),
        200,
        "a refused pack must not have applied its first entry"
    );
}

#[test]
fn the_stat_names_are_available_and_look_like_libtorrent() {
    let names = Session::stat_names();
    assert!(names.len() > 50, "libtorrent exposes many counters");
    for expected in [
        "peer.num_peers_connected",
        "net.has_incoming_connections",
        "dht.dht_nodes",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "the daemon reads `{expected}` and it is not there"
        );
    }
}

#[test]
fn an_offline_session_still_listens_somewhere() {
    let session = session();
    assert!(session.is_listening());
    assert!(session.listen_port() > 0, "a port should have been chosen");
}

#[test]
fn session_counters_arrive_on_the_alert_with_their_names() {
    // The Web UI's status bar is drawn from these, and the alert is the only
    // way libtorrent hands them over.
    use std::time::Duration;

    let mut session = session();
    session.post_session_stats();

    let mut counters = None;
    for _ in 0..25 {
        session.wait_for_alert(Duration::from_millis(200));
        for alert in session.pop_alerts() {
            if let Some(values) = alert.counters() {
                counters = Some(values.to_vec());
            }
        }
        if counters.is_some() {
            break;
        }
    }

    let counters = counters.expect("post_session_stats must answer");
    let names = Session::stat_names();
    assert_eq!(
        counters.len(),
        names.len(),
        "one counter per name, or the indexes mean nothing"
    );

    // Spot-check one the daemon actually reads.
    let index = names
        .iter()
        .position(|name| name == "peer.num_peers_connected")
        .expect("the daemon reads this counter");
    assert_eq!(counters[index], 0, "an offline session has no peers");
}
