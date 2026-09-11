// SPDX-License-Identifier: GPL-3.0-or-later
//! Passwords and sessions.
//!
//! The password is the only secret the Web UI holds, and the session is the
//! only thing standing between a browser and the daemon, so both get checked
//! rather than assumed.

use std::time::Duration;

use redeluge_web::auth::{hash_password, Sessions, StoredPassword};

/// A SHA-1 password exactly as the Python Web UI wrote it: hex(sha1(salt || password)).
fn legacy(salt: &str, password: &str) -> StoredPassword {
    let mut input = salt.as_bytes().to_vec();
    input.extend_from_slice(password.as_bytes());
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &input);
    StoredPassword::from_config(Some(salt), Some(&hex::encode(digest.as_ref())))
}

#[test]
fn a_password_written_by_the_python_web_ui_still_works() {
    // An existing installation must survive the switch without anyone having to
    // reset a password.
    let stored = legacy("c26ab3bbd8b137f99cd83c2c1c0963bcc1a35cad", "deluge");
    assert!(matches!(stored, StoredPassword::LegacySha1 { .. }));
    assert!(stored.verify("deluge"));
    assert!(!stored.verify("Deluge"));
    assert!(!stored.verify(""));
    assert!(!stored.verify("deluge "));
}

#[test]
fn the_documented_default_password_matches_the_shipped_hash() {
    // These two constants are from deluge/ui/web/server.py's CONFIG_DEFAULTS,
    // and they are the "deluge" that every fresh install starts with. If this
    // fails, the SHA-1 path is wrong rather than the test.
    let stored = StoredPassword::from_config(
        Some("c26ab3bbd8b137f99cd83c2c1c0963bcc1a35cad"),
        Some("2ce1a410bcdcc53064129b6d950f2e9fee4edc1e"),
    );
    assert!(stored.verify("deluge"), "the shipped default must verify");
    assert!(!stored.verify("wrong"));
}

#[test]
fn a_legacy_password_asks_to_be_upgraded_and_a_new_one_does_not() {
    assert!(legacy("salt", "hunter2").needs_upgrade());
    assert!(!hash_password("hunter2").unwrap().needs_upgrade());
}

#[test]
fn a_scrypt_password_round_trips_through_the_config_format() {
    let stored = hash_password("correct horse battery staple").unwrap();
    let encoded = stored.to_config_string().expect("scrypt is storable");
    assert!(encoded.starts_with("$scrypt$32768$8$1$"));

    let reloaded = StoredPassword::from_config(Some(""), Some(&encoded));
    assert!(reloaded.verify("correct horse battery staple"));
    assert!(!reloaded.verify("correct horse battery stapl"));
}

#[test]
fn two_hashes_of_the_same_password_differ() {
    // A per-password salt, or the whole file leaks which accounts share one.
    let first = hash_password("same").unwrap().to_config_string().unwrap();
    let second = hash_password("same").unwrap().to_config_string().unwrap();
    assert_ne!(first, second);
}

#[test]
fn an_unset_password_refuses_everything() {
    let stored = StoredPassword::from_config(None, None);
    assert!(matches!(stored, StoredPassword::Unset));
    assert!(!stored.verify(""));
    assert!(!stored.verify("anything"));
}

#[test]
fn a_malformed_stored_password_refuses_rather_than_panics() {
    for bad in [
        "$scrypt$",
        "$scrypt$notanumber$8$1$salt$aabb",
        "$scrypt$32768$8$1$salt$nothex",
        "$scrypt$32768$8$1$salt$aabb$extra",
        "$bcrypt$whatever",
    ] {
        let stored = StoredPassword::from_config(Some("salt"), Some(bad));
        assert!(!stored.verify("anything"), "{bad} should verify nothing");
    }
}

#[test]
fn absurd_scrypt_parameters_are_refused_instead_of_running() {
    // These come out of a configuration file. A work factor of 2^30 would hang
    // the server on the next login, which is a denial of service by config.
    let stored = StoredPassword::from_config(Some(""), Some("$scrypt$1073741824$32$16$salt$00"));
    assert!(!stored.verify("anything"));
}

#[test]
fn a_session_is_valid_until_it_expires() {
    let mut sessions = Sessions::new();
    let id = sessions
        .create("admin", 10, Duration::from_secs(60))
        .unwrap();

    let found = sessions
        .touch(&id, Duration::from_secs(60))
        .expect("a fresh session is valid");
    assert_eq!(found.login, "admin");
    assert_eq!(found.level, 10);
    assert_eq!(sessions.len(), 1);
}

#[test]
fn an_expired_session_is_refused_and_forgotten() {
    let mut sessions = Sessions::new();
    let id = sessions.create("admin", 10, Duration::ZERO).unwrap();

    assert!(sessions.touch(&id, Duration::from_secs(60)).is_none());
    assert!(
        sessions.is_empty(),
        "a refused session must not stay in the table"
    );
}

#[test]
fn an_unknown_session_id_is_refused() {
    let mut sessions = Sessions::new();
    assert!(sessions
        .touch("not a session", Duration::from_secs(60))
        .is_none());
    assert!(sessions.touch("", Duration::from_secs(60)).is_none());
}

#[test]
fn session_ids_do_not_repeat() {
    let mut sessions = Sessions::new();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..200 {
        let id = sessions
            .create("admin", 10, Duration::from_secs(60))
            .unwrap();
        assert_eq!(id.len(), 64, "256 bits of hex");
        assert!(seen.insert(id), "a session id was issued twice");
    }
}

#[test]
fn sweeping_drops_only_what_has_expired() {
    let mut sessions = Sessions::new();
    let live = sessions
        .create("admin", 10, Duration::from_secs(600))
        .unwrap();
    sessions.create("admin", 10, Duration::ZERO).unwrap();
    sessions.create("admin", 10, Duration::ZERO).unwrap();

    assert_eq!(sessions.sweep(), 2);
    assert_eq!(sessions.len(), 1);
    assert!(sessions.touch(&live, Duration::from_secs(600)).is_some());
}

#[test]
fn removing_a_session_ends_it() {
    let mut sessions = Sessions::new();
    let id = sessions
        .create("admin", 10, Duration::from_secs(600))
        .unwrap();

    assert!(sessions.remove(&id));
    assert!(!sessions.remove(&id), "removing twice is not a success");
    assert!(sessions.touch(&id, Duration::from_secs(600)).is_none());
}
