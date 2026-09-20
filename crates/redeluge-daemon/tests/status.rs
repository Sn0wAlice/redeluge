// SPDX-License-Identifier: GPL-3.0-or-later
//! What a poll asks for, and what it costs to answer.
//!
//! A status used to be built whole — about ninety keys, most of them a string
//! — and then filtered down to the thirty-five a client asked for, once per
//! torrent per poll. It is built to the keys now, so these tests are about the
//! answer staying the same: a key that was asked for is there, a key that was
//! not is absent rather than empty, and asking for none still gets everything.
//!
//! `redeluge.update_ui` answers a whole poll from one walk of the library. It
//! has to say exactly what the three calls it replaces say, or a client that
//! uses it sees a different library from one that does not.

use std::sync::Arc;

use redeluge_daemon::auth::{AuthLevel, AuthManager};
use redeluge_daemon::config::Config;
use redeluge_daemon::core::Core;
use redeluge_daemon::events::Event;
use redeluge_daemon::manager::Manager;
use redeluge_daemon::rpc::{CallContext, Rpc};
use redeluge_libtorrent::SessionSettings;
use redeluge_rencode::Value;

/// A magnet with no trackers: nothing here reaches the network.
const MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";

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

fn read_only() -> CallContext {
    CallContext {
        session_id: 2,
        username: "reader".to_owned(),
        level: AuthLevel::ReadOnly,
        peer: "127.0.0.1:0".to_owned(),
    }
}

async fn call(core: &Arc<Core>, method: &str, args: Vec<Value>) -> Value {
    core.call(&admin(), method, args, Vec::new())
        .await
        .unwrap_or_else(|err| panic!("{method} refused: {err:?}"))
}

/// One torrent in the session, so a walk of it has something to find.
async fn with_a_torrent() -> (Arc<Core>, tempfile::TempDir) {
    let (core, dir) = daemon().await;
    call(
        &core,
        "core.add_torrent_magnet",
        vec![Value::Str(MAGNET.to_owned()), Value::Dict(Vec::new())],
    )
    .await;
    (core, dir)
}

fn keys(names: &[&str]) -> Value {
    Value::List(
        names
            .iter()
            .map(|name| Value::Str((*name).to_owned()))
            .collect(),
    )
}

/// The one torrent's status out of a `core.get_torrents_status` answer.
fn only_status(answer: &Value) -> &Vec<(Value, Value)> {
    let Value::Dict(torrents) = answer else {
        panic!("a dictionary of torrents");
    };
    assert_eq!(torrents.len(), 1, "one torrent was added");
    match &torrents[0].1 {
        Value::Dict(status) => status,
        other => panic!("a status dictionary, got {other:?}"),
    }
}

fn names_of(status: &[(Value, Value)]) -> Vec<String> {
    let mut names: Vec<String> = status
        .iter()
        .filter_map(|(key, _)| key.as_str().map(str::to_owned))
        .collect();
    names.sort();
    names
}

// ------------------------------------------------------------------- keys

#[tokio::test(flavor = "multi_thread")]
async fn a_status_carries_the_keys_that_were_asked_for_and_no_others() {
    let (core, _dir) = with_a_torrent().await;

    let answer = call(
        &core,
        "core.get_torrents_status",
        vec![Value::Dict(Vec::new()), keys(&["name", "state", "hash"])],
    )
    .await;

    assert_eq!(
        names_of(only_status(&answer)),
        vec!["hash".to_owned(), "name".to_owned(), "state".to_owned()],
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_the_daemon_does_not_have_is_left_out_rather_than_empty() {
    // A client asking for a key this daemon never reports must not be handed a
    // blank one: an invented key is worse than a missing one, because it looks
    // like an answer.
    let (core, _dir) = with_a_torrent().await;

    let answer = call(
        &core,
        "core.get_torrents_status",
        vec![
            Value::Dict(Vec::new()),
            keys(&["name", "a_key_no_daemon_reports"]),
        ],
    )
    .await;

    assert_eq!(names_of(only_status(&answer)), vec!["name".to_owned()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn asking_for_no_keys_still_gets_every_key() {
    // The expensive keys are the ones now built on demand, so this is where a
    // gate that was written backwards would show: a client that names no keys
    // is asking for all of them.
    let (core, _dir) = with_a_torrent().await;

    let answer = call(
        &core,
        "core.get_torrents_status",
        vec![Value::Dict(Vec::new())],
    )
    .await;

    let status = names_of(only_status(&answer));
    for key in [
        "name",
        "state",
        "save_path",
        "download_location",
        "storage_mode",
        "message",
        "label",
        "owner",
        "peers",
        "trackers",
        "tracker",
        "tracker_host",
        "tracker_status",
        "move_completed_path",
        "move_on_completed_path",
    ] {
        assert!(
            status.contains(&key.to_owned()),
            "`{key}` is missing from a status nobody narrowed"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_torrents_status_is_the_same_whether_it_is_narrowed_or_not() {
    // The narrow answer has to be the wide answer with keys taken out, not a
    // second way of computing them.
    let (core, _dir) = with_a_torrent().await;

    let wide = call(
        &core,
        "core.get_torrents_status",
        vec![Value::Dict(Vec::new())],
    )
    .await;
    let narrow = call(
        &core,
        "core.get_torrents_status",
        vec![
            Value::Dict(Vec::new()),
            keys(&["name", "state", "tracker_host", "label", "save_path"]),
        ],
    )
    .await;

    let wide = only_status(&wide).clone();
    for (key, value) in only_status(&narrow) {
        let found = wide
            .iter()
            .find(|(wide_key, _)| wide_key == key)
            .map(|(_, value)| value);
        assert_eq!(
            found,
            Some(value),
            "{key:?} differs between the wide and the narrow answer"
        );
    }
}

/// Keys whose value is a clock reading, and so may move between two calls.
const MOVES_ON_ITS_OWN: &[&str] = &[
    "active_time",
    "seeding_time",
    "finished_time",
    "time_since_transfer",
    "next_announce",
    "eta",
    "idle_pause_at",
    "tracker_move_at",
    "tracker_remove_at",
];

#[tokio::test(flavor = "multi_thread")]
async fn every_key_survives_being_asked_for_by_name() {
    // The one test that covers the whole surface: whatever the wide answer
    // holds, asking for exactly those keys has to give exactly those values.
    // Each key is now built only when it is named, and a gate written against
    // the wrong name would be invisible in any narrower test — the key would
    // simply stop being reported to the client that asked for it.
    let (core, _dir) = with_a_torrent().await;

    let wide = call(
        &core,
        "core.get_torrents_status",
        vec![Value::Dict(Vec::new())],
    )
    .await;
    let all = only_status(&wide).clone();
    let every_name: Vec<&str> = all.iter().filter_map(|(key, _)| key.as_str()).collect();
    assert!(every_name.len() > 60, "a status is about ninety keys");

    let narrow = call(
        &core,
        "core.get_torrents_status",
        vec![Value::Dict(Vec::new()), keys(&every_name)],
    )
    .await;
    let narrow = only_status(&narrow).clone();

    assert_eq!(
        narrow.len(),
        all.len(),
        "asking for every key by name lost one"
    );
    for (key, value) in &all {
        let name = key.as_str().unwrap_or_default();
        if MOVES_ON_ITS_OWN.contains(&name) {
            continue;
        }
        let found = narrow
            .iter()
            .find(|(narrow_key, _)| narrow_key == key)
            .map(|(_, value)| value);
        assert_eq!(found, Some(value), "{name} differs when it is asked for");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_asked_for_twice_is_answered_once() {
    let (core, _dir) = with_a_torrent().await;

    let answer = call(
        &core,
        "core.get_torrents_status",
        vec![Value::Dict(Vec::new()), keys(&["name", "name"])],
    )
    .await;

    assert_eq!(names_of(only_status(&answer)), vec!["name".to_owned()]);
}

// -------------------------------------------------------------- one poll

#[tokio::test(flavor = "multi_thread")]
async fn one_call_says_what_three_calls_say() {
    let (core, _dir) = with_a_torrent().await;

    let wanted = keys(&["name", "state", "tracker_host", "label", "progress"]);
    let stats = keys(&["payload_download_rate", "payload_upload_rate"]);

    let torrents = call(
        &core,
        "core.get_torrents_status",
        vec![Value::Dict(Vec::new()), wanted.clone()],
    )
    .await;
    let filters = call(&core, "core.get_filter_tree", vec![]).await;

    let combined = call(
        &core,
        "redeluge.update_ui",
        vec![wanted, Value::Dict(Vec::new()), stats.clone()],
    )
    .await;

    assert_eq!(combined.get("torrents"), Some(&torrents));
    assert_eq!(combined.get("filters"), Some(&filters));

    // The counters move between calls, so the stats are compared by shape:
    // what a client reads off them has to be there and has to be a number.
    let Some(Value::Dict(numbers)) = combined.get("stats") else {
        panic!("the stats are a dictionary");
    };
    let names: Vec<&str> = numbers.iter().filter_map(|(key, _)| key.as_str()).collect();
    assert!(names.contains(&"payload_download_rate"), "{names:?}");
    assert!(names.contains(&"payload_upload_rate"), "{names:?}");
    for (key, value) in numbers {
        assert!(
            matches!(value, Value::Int(_) | Value::Float64(_) | Value::Float32(_)),
            "{key:?} is not a number: {value:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_filter_narrows_the_combined_answer_the_same_way() {
    let (core, _dir) = with_a_torrent().await;

    let by_state = Value::Dict(vec![(
        Value::Str("state".to_owned()),
        Value::Str("Seeding".to_owned()),
    )]);

    let separate = call(
        &core,
        "core.get_torrents_status",
        vec![by_state.clone(), keys(&["name"])],
    )
    .await;
    let combined = call(
        &core,
        "redeluge.update_ui",
        vec![keys(&["name"]), by_state, Value::List(Vec::new())],
    )
    .await;

    // The torrent is not seeding, so both answers are empty — and they are
    // empty in the same way, which is what a client's grid reads.
    assert_eq!(separate, Value::Dict(Vec::new()));
    assert_eq!(combined.get("torrents"), Some(&separate));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_combined_call_is_advertised_and_a_reader_may_make_it() {
    // This is how the Web UI finds out whether the daemon it is talking to can
    // answer a poll in one call: `daemon.authorized_call` is false for a
    // method that does not exist, and the answer is remembered per connection.
    // A read-only account has to be allowed it, or half the accounts fall back
    // to three calls for a question that is reading either way.
    let (core, _dir) = daemon().await;

    assert!(
        core.method_list()
            .contains(&"redeluge.update_ui".to_owned()),
        "the combined call is not advertised"
    );

    let allowed = core
        .call(
            &read_only(),
            "daemon.authorized_call",
            vec![Value::Str("redeluge.update_ui".to_owned())],
            Vec::new(),
        )
        .await
        .expect("authorized_call answers");
    assert_eq!(allowed, Value::Bool(true));

    let unknown = core
        .call(
            &read_only(),
            "daemon.authorized_call",
            vec![Value::Str("redeluge.no_such_call".to_owned())],
            Vec::new(),
        )
        .await
        .expect("authorized_call answers");
    assert_eq!(
        unknown,
        Value::Bool(false),
        "a method that does not exist must answer false, or the Web UI would \
         use a call the daemon does not have"
    );
}
