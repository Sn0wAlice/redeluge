// SPDX-License-Identifier: GPL-3.0-or-later
//! Configuration, accounts, events and the settings mapping.

use redeluge_contract::{Contract, Transport};
use redeluge_daemon::auth::{AuthLevel, AuthManager};
use redeluge_daemon::config::Config;
use redeluge_daemon::events::Event;
use redeluge_daemon::prefs;
use redeluge_rencode::Value;
use serde_json::json;

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

// ------------------------------------------------------------------- config

#[test]
fn a_fresh_configuration_has_every_key() {
    let dir = scratch();
    let config = Config::load(dir.path()).unwrap();

    // The 77 keys the contract lists, minus none: a missing key is a method
    // that returns null where a client expects a number.
    let expected = Contract::get().core_config().len();
    assert!(
        config.all().len() >= expected,
        "{} keys, the Python daemon has {expected}",
        config.all().len()
    );

    for key in Contract::get().core_config() {
        assert!(
            config.get(&key.key).is_some(),
            "`{}` is missing from the defaults",
            key.key
        );
    }
}

#[test]
fn an_existing_file_is_read_and_its_values_kept() {
    let dir = scratch();
    std::fs::write(
        dir.path().join("core.conf"),
        r#"{"file": 1, "format": 1}{"daemon_port": 12345, "allow_remote": true}"#,
    )
    .unwrap();

    let config = Config::load(dir.path()).unwrap();
    assert_eq!(config.integer("daemon_port"), Some(12345));
    assert_eq!(config.boolean("allow_remote"), Some(true));
    // And the keys the file did not have were filled in.
    assert!(config.get("max_active_limit").is_some());
}

#[test]
fn a_fresh_configuration_is_written_on_the_first_save() {
    // Loading fills in every key, which counts as a change: without writing,
    // a first run leaves nothing on disk for an operator to edit.
    let dir = scratch();
    let mut config = Config::load(dir.path()).unwrap();
    assert!(config.is_dirty(), "a fresh load has filled in the defaults");

    config.save().unwrap();
    assert!(dir.path().join("core.conf").is_file());

    let reloaded = Config::load(dir.path()).unwrap();
    assert!(!reloaded.is_dirty(), "nothing was missing the second time");
}

#[test]
fn the_written_file_is_in_deluges_two_object_format() {
    let dir = scratch();
    let mut config = Config::load(dir.path()).unwrap();
    config.save().unwrap();

    let text = std::fs::read_to_string(dir.path().join("core.conf")).unwrap();
    assert!(text.starts_with('{'));
    assert!(
        text.matches("\n}{").count() == 1,
        "expected exactly two concatenated objects"
    );

    // And it reads back as itself.
    let reloaded = Config::load(dir.path()).unwrap();
    assert_eq!(reloaded.integer("daemon_port"), Some(58846));
}

#[test]
fn setting_an_unknown_key_is_refused() {
    // A typo in a client would otherwise add a key nothing ever reads.
    let dir = scratch();
    let mut config = Config::load(dir.path()).unwrap();
    assert!(config.set("no_such_setting", json!(1)).is_err());
}

#[test]
fn changing_the_type_of_a_setting_is_refused() {
    let dir = scratch();
    let mut config = Config::load(dir.path()).unwrap();

    assert!(config.set("daemon_port", json!("not a number")).is_err());
    assert!(config.set("allow_remote", json!(1)).is_err());

    // Integers and floats are interchangeable: a client that sends 200 where
    // 200.0 is stored must not break the setting.
    config.set("max_download_speed", json!(200)).unwrap();
    assert_eq!(config.number("max_download_speed"), Some(200.0));
}

#[test]
fn setting_a_value_to_what_it_already_is_changes_nothing() {
    let dir = scratch();
    let mut config = Config::load(dir.path()).unwrap();
    config.save().unwrap();

    config.set("daemon_port", json!(58846)).unwrap();
    assert!(!config.is_dirty(), "an unchanged value is not a change");
}

// --------------------------------------------------------------------- auth

#[test]
fn a_fresh_auth_file_has_a_usable_localclient_account() {
    // The account exists so a local tool can log in by reading the file. A
    // hashed password would make it unusable, which is exactly what the first
    // version of this daemon did.
    let dir = scratch();
    let mut auth = AuthManager::open(dir.path()).unwrap();

    let (username, password) = auth.localclient().expect("localclient exists");
    let (username, password) = (username.to_owned(), password.to_owned());
    assert_eq!(username, "localclient");
    assert!(
        !password.starts_with("$scrypt$"),
        "localclient must stay readable"
    );
    assert_eq!(password.len(), 40, "twenty random bytes in hex");

    assert_eq!(
        auth.authorize(&username, &password).unwrap(),
        AuthLevel::Admin
    );
}

#[test]
fn the_auth_file_is_not_world_readable() {
    let dir = scratch();
    let _ = AuthManager::open(dir.path()).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("auth"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "only the owner may read the auth file");
    }
}

#[test]
fn a_wrong_password_and_an_unknown_account_both_fail() {
    let dir = scratch();
    let mut auth = AuthManager::open(dir.path()).unwrap();
    assert!(auth.authorize("localclient", "wrong").is_err());
    assert!(auth.authorize("nobody", "anything").is_err());
}

#[test]
fn accounts_are_created_updated_and_removed() {
    let dir = scratch();
    let mut auth = AuthManager::open(dir.path()).unwrap();

    auth.create_account("alice", "hunter2", AuthLevel::Normal)
        .unwrap();
    assert_eq!(
        auth.authorize("alice", "hunter2").unwrap(),
        AuthLevel::Normal
    );
    assert!(
        auth.create_account("alice", "again", AuthLevel::Admin)
            .is_err(),
        "a duplicate must be refused"
    );

    auth.update_account("alice", "hunter3", AuthLevel::Admin)
        .unwrap();
    assert!(auth.authorize("alice", "hunter2").is_err());
    assert_eq!(
        auth.authorize("alice", "hunter3").unwrap(),
        AuthLevel::Admin
    );

    auth.remove_account("alice").unwrap();
    assert!(!auth.has_account("alice"));
    assert!(auth.remove_account("alice").is_err());
}

#[test]
fn a_created_account_is_stored_as_scrypt() {
    let dir = scratch();
    let mut auth = AuthManager::open(dir.path()).unwrap();
    auth.create_account("bob", "hunter2", AuthLevel::Normal)
        .unwrap();

    let text = std::fs::read_to_string(dir.path().join("auth")).unwrap();
    let line = text
        .lines()
        .find(|line| line.starts_with("bob:"))
        .expect("bob is in the file");
    assert!(line.contains("$scrypt$"), "not hashed: {line}");
    assert!(!line.contains("hunter2"), "the password is in the file");
}

#[test]
fn a_plaintext_account_from_an_old_file_works_and_is_upgraded() {
    // An installation must keep working across the switch, and then stop being
    // a file full of plaintext passwords.
    let dir = scratch();
    std::fs::write(
        dir.path().join("auth"),
        "localclient:abc123:10\nolduser:plaintext:5\n",
    )
    .unwrap();

    let mut auth = AuthManager::open(dir.path()).unwrap();
    assert_eq!(
        auth.authorize("olduser", "plaintext").unwrap(),
        AuthLevel::Normal
    );

    let text = std::fs::read_to_string(dir.path().join("auth")).unwrap();
    assert!(
        text.contains("olduser:$scrypt$"),
        "the account should have been upgraded:\n{text}"
    );
    assert!(
        text.contains("localclient:abc123"),
        "localclient must stay readable:\n{text}"
    );

    // And it still verifies afterwards.
    assert!(auth.authorize("olduser", "plaintext").is_ok());
    assert!(auth.authorize("olduser", "wrong").is_err());
}

#[test]
fn an_old_two_field_line_still_parses() {
    let dir = scratch();
    std::fs::write(dir.path().join("auth"), "localclient:secret\nalice:pw\n").unwrap();

    let mut auth = AuthManager::open(dir.path()).unwrap();
    // localclient has always been an administrator, whatever the file says.
    assert_eq!(
        auth.authorize("localclient", "secret").unwrap(),
        AuthLevel::Admin
    );
    assert_eq!(auth.authorize("alice", "pw").unwrap(), AuthLevel::Normal);
}

#[test]
fn a_named_auth_level_is_understood() {
    let dir = scratch();
    std::fs::write(dir.path().join("auth"), "alice:pw:ADMIN\nbob:pw:READONLY\n").unwrap();

    let mut auth = AuthManager::open(dir.path()).unwrap();
    assert_eq!(auth.authorize("alice", "pw").unwrap(), AuthLevel::Admin);
    assert_eq!(auth.authorize("bob", "pw").unwrap(), AuthLevel::ReadOnly);
}

#[test]
fn a_malformed_line_is_skipped_rather_than_guessed_at() {
    let dir = scratch();
    std::fs::write(
        dir.path().join("auth"),
        "# a comment\n\nalice:pw:5\nbroken:one:two:three:four\n",
    )
    .unwrap();

    let mut auth = AuthManager::open(dir.path()).unwrap();
    assert!(auth.authorize("alice", "pw").is_ok());
    assert!(!auth.has_account("broken"));
}

#[test]
fn auth_levels_round_trip_through_both_spellings() {
    for level in [
        AuthLevel::None,
        AuthLevel::ReadOnly,
        AuthLevel::Normal,
        AuthLevel::Admin,
    ] {
        assert_eq!(AuthLevel::from_i64(level.as_i64()), Some(level));
        assert_eq!(AuthLevel::from_name(level.as_str()), Some(level));
    }
    // DEFAULT is what old files call NORMAL.
    assert_eq!(AuthLevel::from_name("DEFAULT"), Some(AuthLevel::Normal));
    assert_eq!(AuthLevel::from_i64(7), None);
}

#[test]
fn levels_order_the_way_authorisation_needs() {
    assert!(AuthLevel::None < AuthLevel::ReadOnly);
    assert!(AuthLevel::ReadOnly < AuthLevel::Normal);
    assert!(AuthLevel::Normal < AuthLevel::Admin);
}

// ------------------------------------------------------------------- events

#[test]
fn every_event_matches_the_frozen_contract() {
    // The names and argument order are what clients unpack positionally.
    let contract: Vec<&str> = Contract::get()
        .events()
        .iter()
        .map(|event| event.name.as_str())
        .filter(|name| !name.starts_with("Plugin"))
        .collect();

    let ours: Vec<&str> = Event::NAMES.to_vec();
    assert_eq!(
        ours, contract,
        "the event list no longer matches contract/events.json"
    );
}

#[test]
fn events_carry_their_arguments_in_order() {
    let event = Event::TorrentFileRenamed {
        torrent_id: "abc".to_owned(),
        index: 3,
        name: "new.bin".to_owned(),
    };
    assert_eq!(event.name(), "TorrentFileRenamedEvent");
    assert_eq!(
        event.args(),
        vec![
            Value::Str("abc".into()),
            Value::Int(3),
            Value::Str("new.bin".into()),
        ]
    );

    // The contract says how many arguments each one takes.
    let expected = Contract::get()
        .events()
        .iter()
        .find(|entry| entry.name == "TorrentFileRenamedEvent")
        .expect("it is in the contract");
    assert_eq!(event.args().len(), expected.args.len());
}

#[test]
fn the_events_with_no_arguments_have_none() {
    for event in [
        Event::SessionPaused,
        Event::SessionResumed,
        Event::SessionStarted,
        Event::TorrentQueueChanged,
    ] {
        assert!(event.args().is_empty(), "{} takes arguments", event.name());
    }
}

// -------------------------------------------------------------------- prefs

#[test]
fn the_configuration_maps_onto_libtorrent_settings() {
    let dir = scratch();
    let config = Config::load(dir.path()).unwrap();
    let settings = prefs::to_settings(&config);

    assert!(
        settings.len() > 25,
        "only {} settings mapped",
        settings.len()
    );

    // Every name must be one libtorrent knows, which the bridge's own test
    // checks. Here the point is that the mapping produces no duplicates: two
    // entries for one setting means one of them silently wins.
    let mut names: Vec<&str> = settings.iter().map(|s| s.name()).collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total, "a setting is mapped twice");
}

#[test]
fn speeds_are_converted_from_kibibytes_to_bytes() {
    // Deluge stores KiB/s and libtorrent wants bytes per second. Forgetting the
    // conversion makes a 100 KiB/s limit into 100 bytes per second.
    assert_eq!(prefs::kib_to_bytes(100.0), 102_400);
    assert_eq!(prefs::kib_to_bytes(0.5), 512);
    assert_eq!(prefs::kib_to_bytes(0.0), 0);
}

#[test]
fn no_limit_stays_no_limit_through_the_conversion() {
    assert_eq!(prefs::kib_to_bytes(-1.0), -1);
    assert_eq!(prefs::kib_to_bytes(-100.0), -1);
}

#[test]
fn an_absurd_speed_does_not_overflow() {
    // The value comes from a config file, and libtorrent's setting is 32-bit.
    let clamped = prefs::kib_to_bytes(1e18);
    assert!(clamped <= i64::from(i32::MAX));
}

// ------------------------------------------------------------------ contract

#[test]
fn the_daemon_claims_exactly_the_contracts_methods() {
    let daemon_methods: Vec<&str> = Contract::get()
        .methods_for(Transport::Daemon)
        .map(|method| method.name.as_str())
        .collect();

    assert_eq!(daemon_methods.len(), 70);
    assert!(daemon_methods.contains(&"core.add_torrent_magnet"));
    assert!(daemon_methods.contains(&"daemon.shutdown"));
    assert!(
        !daemon_methods.iter().any(|name| name.contains("plugin")),
        "the plugin methods should have been removed"
    );
}

// ------------------------------------------------------------------ features

/// Keys this daemon has that the Python one did not.
///
/// The four plugins became features, and their settings had to live somewhere.
/// Listing them here rather than leaving the check loose is what keeps the next
/// invented key visible: a typo in `defaults()` shipped once already, and a test
/// against the contract is how it was found.
const ADDED_BY_REDELUGE: &[&str] = &["autoadd", "blocklist", "idle_pause", "label", "scheduler"];

#[test]
fn no_configuration_key_was_invented_without_saying_so() {
    let dir = scratch();
    let config = Config::load(dir.path()).unwrap();

    let contract: Vec<&str> = Contract::get()
        .core_config()
        .iter()
        .map(|key| key.key.as_str())
        .collect();

    for key in config.all().keys() {
        assert!(
            contract.contains(&key.as_str()) || ADDED_BY_REDELUGE.contains(&key.as_str()),
            "`{key}` is in neither the contract nor the declared additions"
        );
    }
}

#[test]
fn a_fresh_configuration_carries_each_feature() {
    let dir = scratch();
    let config = Config::load(dir.path()).unwrap();

    for key in ADDED_BY_REDELUGE {
        let value = config
            .get(key)
            .unwrap_or_else(|| panic!("`{key}` is missing"));
        assert!(value.is_object(), "`{key}` should be a dictionary");
    }
}

#[test]
fn each_feature_is_off_until_someone_turns_it_on() {
    // A watched directory nobody asked for, or a schedule that pauses the
    // session on a Monday, would be a nasty surprise on an upgrade.
    let dir = scratch();
    let config = Config::load(dir.path()).unwrap();

    for key in ADDED_BY_REDELUGE {
        // `label` is a register, not a switch: it holds the labels that exist
        // and starts empty, which is the same thing as off and is why it has
        // nothing to turn on. Labels cannot be disabled in any case, because
        // every client that asks is told the Label plugin is enabled.
        if *key == "label" {
            assert_eq!(
                config.get(key).and_then(|value| value.get("labels")),
                Some(&json!({})),
                "`label` should start with no labels in it"
            );
            continue;
        }
        assert_eq!(
            config.get(key).and_then(|value| value.get("enabled")),
            Some(&json!(false)),
            "`{key}` should default to off"
        );
    }
}

#[test]
fn a_feature_is_configured_through_set_config_like_anything_else() {
    // The point of holding each feature in one key: no new RPC method, so a
    // client that already speaks core.set_config can configure all of them.
    let dir = scratch();
    let mut config = Config::load(dir.path()).unwrap();

    config
        .set(
            "scheduler",
            json!({"enabled": true, "low_down": 50.0, "button_state": vec![vec![0u8; 7]; 24]}),
        )
        .unwrap();

    let settings =
        redeluge_daemon::features::scheduler::Settings::from_config(config.get("scheduler"));
    assert!(settings.enabled);
    assert_eq!(settings.low_down, 50.0);
}

#[test]
fn a_feature_key_still_refuses_a_value_of_the_wrong_type() {
    let dir = scratch();
    let mut config = Config::load(dir.path()).unwrap();
    assert!(config.set("blocklist", json!("on")).is_err());
}

#[test]
fn each_feature_default_parses_back_into_its_own_settings() {
    // Serialising defaults and parsing them again sounds circular, and is not:
    // it catches a field renamed on one side only, which would otherwise show
    // up as a silently ignored setting.
    use redeluge_daemon::features::{autoadd, blocklist, scheduler};

    let dir = scratch();
    let config = Config::load(dir.path()).unwrap();

    let autoadd = autoadd::Settings::from_config(config.get("autoadd"));
    assert_eq!(autoadd.interval, 5);
    assert!(autoadd.watchdirs.is_empty());

    let blocklist = blocklist::Settings::from_config(config.get("blocklist"));
    assert_eq!(blocklist.check_after_days, 4);
    assert_eq!(blocklist.try_times, 3);

    let scheduler = scheduler::Settings::from_config(config.get("scheduler"));
    assert_eq!(scheduler.button_state.len(), 24);
    assert_eq!(scheduler.low_down, -1.0);
}

// -------------------------------------------------------------------- labels

#[test]
fn a_label_is_lower_case_and_loses_what_the_plugin_would_have_refused() {
    use redeluge_daemon::core::normalise_label;

    assert_eq!(normalise_label("Films"), "films");
    assert_eq!(normalise_label("  tv-shows  "), "tv-shows");
    assert_eq!(normalise_label("a_b.c-d9"), "a_b.c-d9");
    // The plugin refused these outright. Cleaning is kinder than a torrent
    // silently keeping its old label because of one character.
    assert_eq!(normalise_label("my label!"), "mylabel");
    assert_eq!(normalise_label(""), "");
}
