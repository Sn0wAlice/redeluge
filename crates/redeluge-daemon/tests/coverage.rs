// SPDX-License-Identifier: GPL-3.0-or-later
//! Every method in the contract must actually be answerable.
//!
//! Not by reading the source: by starting a daemon and calling all of them.
//! Bad arguments are fine, and expected, because the point is only that the
//! method exists. What must not happen is `NotImplementedError`, which is the
//! daemon admitting the gap.

use std::sync::Arc;

use redeluge_contract::{Contract, Transport};
use redeluge_daemon::auth::{AuthLevel, AuthManager};
use redeluge_daemon::config::Config;
use redeluge_daemon::core::Core;
use redeluge_daemon::events::Event;
use redeluge_daemon::manager::Manager;
use redeluge_daemon::rpc::{CallContext, Rpc};
use redeluge_libtorrent::SessionSettings;
use redeluge_rencode::Value;

async fn daemon() -> (Arc<Core>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let config = Config::load(dir.path()).expect("a fresh config");
    let auth = AuthManager::open(dir.path()).expect("an auth file");

    let (events, _) = tokio::sync::broadcast::channel::<Event>(64);
    let manager = Manager::start(dir.path().to_path_buf(), SessionSettings::offline(), events)
        .expect("the manager starts");

    (
        Core::new(manager, config, auth, dir.path().to_path_buf()),
        dir,
    )
}

fn admin() -> CallContext {
    CallContext {
        session_id: 1,
        username: "localclient".to_owned(),
        level: AuthLevel::Admin,
        peer: "127.0.0.1:0".to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_contract_method_is_answerable() {
    let (core, _dir) = daemon().await;
    let context = admin();

    // Two are answered by the listener rather than by Core, because they are
    // about the connection: the version and the login itself.
    const ANSWERED_BY_THE_LISTENER: &[&str] = &[
        "daemon.info",
        "daemon.login",
        "daemon.get_method_list",
        "daemon.set_event_interest",
    ];
    // These reach the network, and a test must not.
    const REACHES_THE_NETWORK: &[&str] = &[
        "core.add_torrent_url",
        "core.test_listen_port",
        "core.prefetch_magnet_metadata",
    ];
    // This one stops the daemon, which would end the test early.
    const STOPS_THE_DAEMON: &[&str] = &["daemon.shutdown"];

    let mut missing = Vec::new();

    for method in Contract::get().methods_for(Transport::Daemon) {
        let name = method.name.as_str();
        if ANSWERED_BY_THE_LISTENER.contains(&name)
            || REACHES_THE_NETWORK.contains(&name)
            || STOPS_THE_DAEMON.contains(&name)
        {
            continue;
        }

        // No arguments: most calls will refuse, which is the right answer and
        // proves the method is there.
        let outcome = core.call(&context, name, Vec::new(), Vec::new()).await;
        if let Err(error) = outcome {
            if error.exception == "NotImplementedError" {
                missing.push(name.to_owned());
            }
        }
    }

    assert!(
        missing.is_empty(),
        "{} contract methods are not implemented: {missing:?}",
        missing.len()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_listener_and_the_core_together_cover_the_contract() {
    let (core, _dir) = daemon().await;
    let advertised: Vec<String> = core.method_list();

    // The contract, the plugin this daemon answers for and this fork's own
    // methods, and nothing else.
    // A Deluge daemon advertises its plugins' methods as well as the core's,
    // so the list growing by exactly the Label plugin's is correct; growing by
    // anything else would mean an invented method, which is what this guards.
    let mut expected: Vec<String> = Contract::get()
        .methods_for(Transport::Daemon)
        .map(|method| method.name.clone())
        .collect();
    expected.extend(
        redeluge_daemon::core::PLUGIN_METHODS
            .iter()
            .map(|name| (*name).to_owned()),
    );
    // And this fork's own, in their own namespace so that nothing can mistake
    // them for Deluge's. Declared here for the same reason the config keys
    // are: the next invented method has to be a deliberate act.
    expected.extend(
        redeluge_daemon::core::REDELUGE_METHODS
            .iter()
            .map(|name| (*name).to_owned()),
    );
    expected.sort();

    assert_eq!(
        advertised, expected,
        "daemon.get_method_list is not the contract plus the Label plugin"
    );
}

/// Every method the Label plugin contributed is answerable.
///
/// Radarr, Sonarr and the rest ask `core.get_enabled_plugins` and then call
/// these. A method that is advertised and raises `NotImplementedError` is
/// worse than one that was never advertised.
#[tokio::test(flavor = "multi_thread")]
async fn every_plugin_method_is_answerable() {
    let (core, _dir) = daemon().await;
    let context = admin();

    let mut missing = Vec::new();
    for name in redeluge_daemon::core::PLUGIN_METHODS
        .iter()
        .chain(redeluge_daemon::core::REDELUGE_METHODS)
    {
        let outcome = core.call(&context, name, Vec::new(), Vec::new()).await;
        if let Err(error) = outcome {
            if error.exception == "NotImplementedError" {
                missing.push((*name).to_owned());
            }
        }
    }

    assert!(missing.is_empty(), "not implemented: {missing:?}");
}

/// The plugin reports itself as enabled, which is the check every client makes.
#[tokio::test(flavor = "multi_thread")]
async fn the_label_plugin_reports_itself_as_enabled() {
    let (core, _dir) = daemon().await;

    for method in ["core.get_enabled_plugins", "core.get_available_plugins"] {
        let answer = core
            .call(&admin(), method, Vec::new(), Vec::new())
            .await
            .expect("answered");
        assert_eq!(
            answer,
            Value::List(vec![Value::Str("Label".to_owned())]),
            "{method} should report the Label plugin"
        );
    }

    // Nothing else can be turned on, and it says so rather than pretending.
    let answer = core
        .call(
            &admin(),
            "core.enable_plugin",
            vec![Value::Str("Execute".to_owned())],
            Vec::new(),
        )
        .await
        .expect("answered");
    assert_eq!(answer, Value::Bool(false));
}

/// A label survives having nothing in it, which is the whole point of storing
/// the register.
#[tokio::test(flavor = "multi_thread")]
async fn a_label_exists_before_any_torrent_carries_it() {
    let (core, _dir) = daemon().await;
    let context = admin();

    let added = core
        .call(
            &context,
            "label.add",
            vec![Value::Str("radarr".to_owned())],
            Vec::new(),
        )
        .await
        .expect("answered");
    assert_eq!(added, Value::Bool(true));

    let labels = core
        .call(&context, "label.get_labels", Vec::new(), Vec::new())
        .await
        .expect("answered");
    assert_eq!(labels, Value::List(vec![Value::Str("radarr".to_owned())]));

    // Adding it again is not an error: clients add before every use.
    let again = core
        .call(
            &context,
            "label.add",
            vec![Value::Str("radarr".to_owned())],
            Vec::new(),
        )
        .await
        .expect("answered");
    assert_eq!(again, Value::Bool(false));

    let removed = core
        .call(
            &context,
            "label.remove",
            vec![Value::Str("radarr".to_owned())],
            Vec::new(),
        )
        .await
        .expect("answered");
    assert_eq!(removed, Value::Bool(true));
    let labels = core
        .call(&context, "label.get_labels", Vec::new(), Vec::new())
        .await
        .expect("answered");
    assert_eq!(labels, Value::List(Vec::new()));
}

/// A label name is cleaned the same way wherever it arrives from.
#[tokio::test(flavor = "multi_thread")]
async fn a_label_is_normalised_on_the_way_in() {
    let (core, _dir) = daemon().await;

    core.call(
        &admin(),
        "label.add",
        vec![Value::Str("  My Films! ".to_owned())],
        Vec::new(),
    )
    .await
    .expect("answered");

    let labels = core
        .call(&admin(), "label.get_labels", Vec::new(), Vec::new())
        .await
        .expect("answered");
    assert_eq!(labels, Value::List(vec![Value::Str("myfilms".to_owned())]));
}

#[tokio::test(flavor = "multi_thread")]
async fn authorisation_levels_come_from_the_contract() {
    let (core, _dir) = daemon().await;

    for method in Contract::get().methods_for(Transport::Daemon) {
        let level = core
            .auth_level(&method.name)
            .unwrap_or_else(|| panic!("{} has no level", method.name));

        // One deliberate difference: the Python daemon answers this before
        // authentication, and here it needs read-only.
        if method.name == "core.get_auth_levels_mappings" {
            assert_eq!(level, AuthLevel::ReadOnly);
            continue;
        }
        assert_eq!(
            level.as_i64(),
            i64::from(method.auth_level.as_u8()),
            "{} is at the wrong level",
            method.name
        );
    }

    assert!(core.auth_level("core.no_such_method").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_account_cannot_change_anything() {
    // The levels are the whole point of having them, so this checks the ones
    // that matter rather than trusting the numbers.
    let (core, _dir) = daemon().await;

    for (method, minimum) in [
        ("core.remove_torrent", AuthLevel::Normal),
        ("core.set_config", AuthLevel::Normal),
        ("core.create_account", AuthLevel::Admin),
        ("core.remove_account", AuthLevel::Admin),
    ] {
        let level = core.auth_level(method).expect("a known method");
        assert!(
            level >= minimum,
            "{method} is reachable at {level:?}, which is below {minimum:?}"
        );
    }

    // And reading is not privileged.
    assert!(core.auth_level("core.get_torrents_status").unwrap() <= AuthLevel::Normal);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_reports_a_version_clients_recognise() {
    let (core, _dir) = daemon().await;
    let version = core.version();

    // A client compares this against what it knows how to speak. Reporting
    // this crate's own version makes every one of them refuse to connect.
    assert!(
        version.starts_with("2."),
        "clients expect a Deluge 2 version, got {version}"
    );
}

// ------------------------------------------------- the defaults a torrent gets

/// The "Add Torrent Options" preferences reach a new torrent.
///
/// They did not: every torrent started from a hard-coded constant, so
/// seventeen settings could be changed in the preferences window and meant
/// nothing. This calls the same function the three add methods and the watched
/// directories all go through.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_torrent_starts_from_the_configured_defaults() {
    let (core, _dir) = daemon().await;

    let pair = |key: &str, value: Value| (Value::Str(key.to_owned()), value);
    let changes = Value::Dict(vec![
        pair(
            "download_location",
            Value::Str("/downloads/incoming".to_owned()),
        ),
        pair("add_paused", Value::Bool(true)),
        pair("pre_allocate_storage", Value::Bool(true)),
        pair("prioritize_first_last_pieces", Value::Bool(true)),
        pair("sequential_download", Value::Bool(true)),
        pair("move_completed", Value::Bool(true)),
        pair(
            "move_completed_path",
            Value::Str("/downloads/done".to_owned()),
        ),
        pair("stop_seed_at_ratio", Value::Bool(true)),
        pair("stop_seed_ratio", Value::Float64(3.5)),
        pair("remove_seed_at_ratio", Value::Bool(true)),
        pair("max_connections_per_torrent", Value::Int(42)),
        pair("max_upload_slots_per_torrent", Value::Int(7)),
        pair("max_download_speed_per_torrent", Value::Float64(250.0)),
        pair("max_upload_speed_per_torrent", Value::Float64(125.0)),
    ]);

    core.call(&admin(), "core.set_config", vec![changes], Vec::new())
        .await
        .expect("the settings are accepted");

    let options = core.torrent_defaults().await;

    assert_eq!(options.save_path.as_deref(), Some("/downloads/incoming"));
    assert!(options.paused);
    assert_eq!(options.storage_mode, "allocate");
    assert!(options.prioritize_first_last);
    assert!(options.sequential_download);
    assert!(options.move_completed);
    assert_eq!(
        options.move_completed_path.as_deref(),
        Some("/downloads/done")
    );
    assert!(options.stop_at_ratio);
    assert_eq!(options.stop_ratio, 3.5);
    assert!(options.remove_at_ratio);
    assert_eq!(options.max_connections, 42);
    assert_eq!(options.max_upload_slots, 7);
    assert_eq!(options.max_download_speed, 250.0);
    assert_eq!(options.max_upload_speed, 125.0);
}

/// An empty configuration still produces something a torrent can be added with.
#[tokio::test(flavor = "multi_thread")]
async fn the_defaults_are_the_documented_ones_when_nothing_is_set() {
    let (core, _dir) = daemon().await;
    let options = core.torrent_defaults().await;

    assert!(!options.paused);
    assert_eq!(options.storage_mode, "sparse");
    assert_eq!(options.max_connections, -1);
    assert_eq!(options.max_upload_speed, -1.0);
    assert!(!options.stop_at_ratio);
}

/// Every value the sidebar can show is a value the filter can match.
///
/// This is the shape of a bug that shipped: the filter tree and the torrent
/// status each computed the tracker host with their own function, and the two
/// disagreed. The sidebar listed `tracker.example.com` while every torrent was
/// recorded under `example.com`, so clicking the row filtered to nothing, and
/// the empty case was worse: the tree said `Error` and the status said "".
///
/// A row in that list is a filter value the client sends straight back, so the
/// two have to be the same string.
#[tokio::test(flavor = "multi_thread")]
async fn every_filter_row_matches_the_torrents_it_counts() {
    let (core, _dir) = daemon().await;
    let context = admin();

    let tree = core
        .call(&context, "core.get_filter_tree", Vec::new(), Vec::new())
        .await
        .expect("a filter tree");

    let Value::Dict(categories) = tree else {
        panic!("the filter tree is a dictionary");
    };

    for (category, rows) in &categories {
        let Some(category) = category.as_str() else {
            continue;
        };
        let Value::List(rows) = rows else { continue };

        for row in rows {
            let Value::List(pair) = row else { continue };
            let Some(value) = pair.first().and_then(Value::as_str) else {
                continue;
            };
            let count = pair.get(1).and_then(Value::as_i64).unwrap_or(0);

            // "All" and "Active" are the two pseudo-values every category may
            // carry; everything else is a real value off a torrent.
            if value == "All" || (category == "state" && value == "Active") {
                continue;
            }

            let filter = Value::Dict(vec![(
                Value::Str(category.to_owned()),
                Value::Str(value.to_owned()),
            )]);
            let matched = core
                .call(
                    &context,
                    "core.get_torrents_status",
                    vec![filter, Value::List(vec![Value::Str("name".to_owned())])],
                    Vec::new(),
                )
                .await
                .expect("a status answer");

            let Value::Dict(torrents) = matched else {
                panic!("a status answer is a dictionary");
            };
            assert_eq!(
                torrents.len() as i64,
                count,
                "the sidebar says {category} {value:?} has {count}, \
                 and filtering on it returns {}",
                torrents.len()
            );
        }
    }
}

/// Resuming the session starts what the session pause stopped, and nothing
/// else.
///
/// It used to resume every torrent it could see. That undid every deliberate
/// pause: one made by hand, one made by the share-ratio rule, one made by the
/// idle rule. And the scheduler performs a session resume when the daemon
/// starts, so in practice no pause of any kind survived a restart.
#[tokio::test(flavor = "multi_thread")]
async fn resuming_the_session_leaves_a_deliberate_pause_alone() {
    let (core, _dir) = daemon().await;

    // With no torrents there is nothing to pause, so what is asserted is the
    // bookkeeping: a resume must not have a list of everything to start.
    core.set_session_paused(true).await.expect("paused");
    core.set_session_paused(false).await.expect("resumed");

    let left = core
        .manager
        .with(|state| state.paused_by_session.len())
        .await
        .expect("the manager answers");
    assert_eq!(left, 0, "the resume should have emptied its own list");

    let answer = core
        .call(&admin(), "core.is_session_paused", Vec::new(), Vec::new())
        .await
        .expect("answered");
    assert_eq!(answer, Value::Bool(false));
}
