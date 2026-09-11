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

    let contract: Vec<String> = Contract::get()
        .methods_for(Transport::Daemon)
        .map(|method| method.name.clone())
        .collect();

    assert_eq!(
        advertised, contract,
        "daemon.get_method_list does not match the contract"
    );
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
