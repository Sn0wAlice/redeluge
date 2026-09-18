// SPDX-License-Identifier: GPL-3.0-or-later
//! Making a `.torrent`, through the daemon, with libtorrent doing the hashing.
//!
//! The unit tests in `maketorrent.rs` cover the arguments and the job registry
//! without touching a disk. What they cannot cover is the part that crosses the
//! bridge: whether a directory is really taken whole, whether a hidden file is
//! really left out, and whether the file that comes back is the format that was
//! asked for. Those are assertions about libtorrent's output, so they need
//! libtorrent.
//!
//! The finished file is inspected as bytes rather than decoded. The daemon has
//! no bencode decoder, deliberately, and the three things worth asserting —
//! a name, a tracker, a `meta version` key — are each a literal sequence in the
//! file. A decoder here would be a second implementation to keep right.

use std::path::Path;
use std::sync::Arc;

use redeluge_contract::Contract;
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

    let (events, _) = tokio::sync::broadcast::channel::<Event>(256);
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

/// A directory with something in a subdirectory and something hidden.
///
/// The subdirectory is the whole point of "recursive": a torrent that only
/// carried the top level would look right in the file list and seed nothing.
fn content(root: &Path) -> String {
    let top = root.join("Some Release");
    std::fs::create_dir_all(top.join("extras")).expect("the content directory");
    std::fs::write(top.join("main.bin"), vec![7u8; 96 * 1024]).expect("a file");
    std::fs::write(top.join("extras").join("inner.bin"), vec![9u8; 32 * 1024]).expect("a file");
    // What every torrent creator leaves out, and nobody wants to seed.
    std::fs::write(top.join(".hidden-note"), b"not for the swarm").expect("a file");
    top.display().to_string()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn bytes_of(value: Value) -> Vec<u8> {
    match value {
        Value::Bytes(raw) => raw,
        other => panic!("expected the torrent file, got {other:?}"),
    }
}

/// `core.create_torrent`'s arguments, in the contract's order.
fn positional(path: &str, tracker: &str) -> Vec<Value> {
    vec![
        Value::Str(path.to_owned()),
        Value::Str(tracker.to_owned()),
        Value::Int(16 * 1024),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_is_taken_whole_and_recursively() {
    let (core, dir) = daemon().await;
    let path = content(dir.path());

    let built = bytes_of(
        core.call(
            &admin(),
            "core.create_torrent",
            positional(&path, "http://tracker.example/announce"),
            Vec::new(),
        )
        .await
        .expect("a torrent"),
    );

    assert!(contains(&built, b"Some Release"), "the name is missing");
    assert!(
        contains(&built, b"main.bin"),
        "the top-level file is missing"
    );
    assert!(
        contains(&built, b"inner.bin"),
        "the file in the subdirectory is missing, so nothing below the top \
         level was walked"
    );
    assert!(contains(&built, b"extras"), "the subdirectory is missing");
    assert!(
        !contains(&built, b".hidden-note"),
        "a hidden file was put in the torrent"
    );
    assert!(
        contains(&built, b"http://tracker.example/announce"),
        "the primary tracker is missing: it is argument one and a string, \
         which is the shape Deluge's own callers send"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_file_makes_a_single_file_torrent() {
    let (core, dir) = daemon().await;
    let file = dir.path().join("one.bin");
    std::fs::write(&file, vec![3u8; 48 * 1024]).expect("a file");

    let built = bytes_of(
        core.call(
            &admin(),
            "core.create_torrent",
            positional(&file.display().to_string(), ""),
            Vec::new(),
        )
        .await
        .expect("a torrent"),
    );

    assert!(contains(&built, b"one.bin"));
    // A single-file torrent carries a length and no file list.
    assert!(contains(&built, b"6:lengthi49152e"), "no length key");
    assert!(!contains(&built, b"5:files"), "a file list for one file");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_torrent_is_v1_unless_the_caller_asks_for_more() {
    // libtorrent 2.0 writes a hybrid torrent when it is not told otherwise, and
    // a tracker that only knows v1 refuses one. Deluge writes v1, so this does.
    let (core, dir) = daemon().await;
    let path = content(dir.path());

    let built = bytes_of(
        core.call(
            &admin(),
            "core.create_torrent",
            positional(&path, ""),
            Vec::new(),
        )
        .await
        .expect("a torrent"),
    );
    assert!(
        !contains(&built, b"meta version"),
        "the default carried v2 hashes"
    );

    let mut hybrid = positional(&path, "");
    hybrid.resize(10, Value::None);
    hybrid.push(Value::Str("hybrid".to_owned()));
    let built = bytes_of(
        core.call(&admin(), "core.create_torrent", hybrid, Vec::new())
            .await
            .expect("a torrent"),
    );
    assert!(
        contains(&built, b"meta version"),
        "asking for a hybrid torrent did not produce one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_target_is_written_and_nothing_is_returned() {
    // Deluge answers with the file when it was not asked to write one, and with
    // nothing when it was. Clients branch on that.
    let (core, dir) = daemon().await;
    let path = content(dir.path());
    let target = dir.path().join("out.torrent");

    let mut args = positional(&path, "");
    args.push(Value::None);
    args.push(Value::Str(target.display().to_string()));

    let answer = core
        .call(&admin(), "core.create_torrent", args, Vec::new())
        .await
        .expect("a torrent");

    assert_eq!(answer, Value::None);
    let written = std::fs::read(&target).expect("the file was written");
    assert!(contains(&written, b"Some Release"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_path_that_is_not_there_fails_without_a_job_or_a_panic() {
    let (core, _dir) = daemon().await;

    let outcome = core
        .call(
            &admin(),
            "core.create_torrent",
            positional("/nowhere/at/all", ""),
            Vec::new(),
        )
        .await;
    assert!(outcome.is_err(), "a missing path produced a torrent");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_torrent_can_be_created_and_seeded_in_one_call() {
    let (core, dir) = daemon().await;
    let path = content(dir.path());

    let mut args = positional(&path, "http://tracker.example/announce");
    // target, webseeds, private, created_by, trackers, add_to_session
    args.resize(9, Value::None);
    args.push(Value::Bool(true));

    core.call(&admin(), "core.create_torrent", args, Vec::new())
        .await
        .expect("a torrent");

    let state = core
        .call(&admin(), "core.get_session_state", Vec::new(), Vec::new())
        .await
        .expect("the session state");
    let ids = state.as_list().expect("a list of ids");
    assert_eq!(ids.len(), 1, "the created torrent was not added");

    let id = ids[0].as_str().expect("an id").to_owned();
    let status = core
        .call(
            &admin(),
            "core.get_torrent_status",
            vec![Value::Str(id), Value::List(Vec::new())],
            Vec::new(),
        )
        .await
        .expect("a status");

    assert_eq!(
        status.get("name").and_then(Value::as_str),
        Some("Some Release")
    );
    // It is seeding what it was built from, not looking for it somewhere else.
    assert_eq!(
        status.get("save_path").and_then(Value::as_str),
        Some(dir.path().display().to_string().as_str())
    );
}

// ------------------------------------------------------------- the job form

#[tokio::test(flavor = "multi_thread")]
async fn the_job_form_answers_at_once_and_reports_its_way_to_done() {
    // The whole reason it exists: the Web UI has one connection to the daemon
    // and the daemon serves one call at a time on it, so a call that waits for
    // the hashing freezes the interface for as long as the content takes.
    let (core, dir) = daemon().await;
    let path = content(dir.path());

    let options = Value::Dict(vec![
        (Value::Str("path".into()), Value::Str(path)),
        (
            Value::Str("trackers".into()),
            Value::List(vec![Value::Str("http://tracker.example/announce".into())]),
        ),
        (Value::Str("piece_length".into()), Value::Int(16 * 1024)),
    ]);

    let job = core
        .call(
            &admin(),
            "redeluge.create_torrent",
            vec![options],
            Vec::new(),
        )
        .await
        .expect("a job id");
    let job = job.as_str().expect("an id").to_owned();

    let mut status = Value::None;
    for _ in 0..200 {
        status = core
            .call(
                &admin(),
                "redeluge.get_create_torrent",
                vec![Value::Str(job.clone())],
                Vec::new(),
            )
            .await
            .expect("a status");
        if status.get("state").and_then(Value::as_str) != Some("running") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    assert_eq!(
        status.get("state").and_then(Value::as_str),
        Some("done"),
        "the job did not finish: {status:?}"
    );
    assert_eq!(
        status.get("name").and_then(Value::as_str),
        Some("Some Release")
    );
    assert_eq!(
        status
            .get("info_hash")
            .and_then(Value::as_str)
            .map(str::len),
        Some(40)
    );

    let file = bytes_of(
        core.call(
            &admin(),
            "redeluge.get_created_torrent",
            vec![Value::Str(job)],
            Vec::new(),
        )
        .await
        .expect("the file"),
    );
    assert!(contains(&file, b"Some Release"));
    assert!(contains(&file, b"http://tracker.example/announce"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_that_was_never_started_is_an_error_rather_than_an_empty_answer() {
    let (core, _dir) = daemon().await;

    for method in [
        "redeluge.get_create_torrent",
        "redeluge.get_created_torrent",
    ] {
        let outcome = core
            .call(
                &admin(),
                method,
                vec![Value::Str("not an id".to_owned())],
                Vec::new(),
            )
            .await;
        assert!(outcome.is_err(), "{method} invented an answer");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn building_a_torrent_is_not_something_a_read_only_account_can_do() {
    // The rest of the `redeluge.` namespace is reads, and a read-only account
    // is entitled to them. This one hashes a directory, writes a file and can
    // add a torrent, so it takes the level Deluge gives `core.create_torrent`.
    let (core, _dir) = daemon().await;

    assert_eq!(
        core.auth_level("redeluge.create_torrent"),
        Some(AuthLevel::Normal)
    );
    assert_eq!(
        core.auth_level("redeluge.get_create_torrent"),
        Some(AuthLevel::ReadOnly)
    );
    // And the Deluge method keeps the level the contract gives it.
    assert_eq!(
        core.auth_level("core.create_torrent"),
        Contract::get()
            .method("core.create_torrent")
            .and_then(|entry| AuthLevel::from_i64(entry.auth_level.as_u8().into()))
    );
}

// ------------------------------------------------------- browsing the disk

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_lists_its_folders_first_then_its_files() {
    let (core, dir) = daemon().await;
    let path = content(dir.path());
    std::fs::write(Path::new(&path).join("another.bin"), b"x").expect("a file");

    let listing = core
        .call(
            &admin(),
            "redeluge.list_directory",
            vec![Value::Str(path.clone())],
            Vec::new(),
        )
        .await
        .expect("a listing");

    assert_eq!(listing.get("path").and_then(Value::as_str), Some(&path[..]));
    let entries = listing
        .get("entries")
        .and_then(Value::as_list)
        .expect("entries");

    let names: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .collect();
    assert_eq!(
        names,
        vec!["extras", "another.bin", "main.bin"],
        "directories come first, then files, each alphabetically"
    );

    // A hidden entry is not offered: the builder skips it, so choosing one
    // would be choosing a torrent that comes back empty.
    assert!(!names.contains(&".hidden-note"));

    // A file says how big it is; a directory would mean walking all of it,
    // which is `core.get_path_size` and is asked one directory at a time.
    let file = entries.last().expect("a file");
    assert_eq!(file.get("kind").and_then(Value::as_str), Some("file"));
    assert_eq!(file.get("size").and_then(Value::as_i64), Some(96 * 1024));
    assert!(entries[0].get("size").expect("a size key").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listing_says_what_is_above_it_so_a_browser_can_climb() {
    let (core, dir) = daemon().await;

    let listing = core
        .call(
            &admin(),
            "redeluge.list_directory",
            vec![Value::Str(dir.path().display().to_string())],
            Vec::new(),
        )
        .await
        .expect("a listing");
    assert_eq!(
        listing.get("parent").and_then(Value::as_str),
        dir.path()
            .parent()
            .map(|parent| parent.display().to_string())
            .as_deref()
    );

    // The root has nowhere above it, and a browser has to be told rather than
    // shown a button that does nothing.
    let root = core
        .call(
            &admin(),
            "redeluge.list_directory",
            vec![Value::Str("/".to_owned())],
            Vec::new(),
        )
        .await
        .expect("a listing");
    assert!(root.get("parent").expect("a parent key").is_none());
    assert_eq!(root.get("path").and_then(Value::as_str), Some("/"));
}

#[tokio::test(flavor = "multi_thread")]
async fn asking_about_a_file_lists_the_directory_it_is_in() {
    // The dialog keeps one path, and a person clicking a file to select it
    // should not empty the list they are choosing from.
    let (core, dir) = daemon().await;
    let file = dir.path().join("one.bin");
    std::fs::write(&file, b"x").expect("a file");

    let listing = core
        .call(
            &admin(),
            "redeluge.list_directory",
            vec![Value::Str(file.display().to_string())],
            Vec::new(),
        )
        .await
        .expect("a listing");

    assert_eq!(
        listing.get("path").and_then(Value::as_str),
        Some(dir.path().display().to_string().as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_that_cannot_be_read_is_empty_rather_than_an_error() {
    // The daemon runs as its own user and a browser walking a filesystem will
    // meet directories it cannot open. That is a fact about the directory.
    let (core, _dir) = daemon().await;

    let listing = core
        .call(
            &admin(),
            "redeluge.list_directory",
            vec![Value::Str("/nowhere/at/all".to_owned())],
            Vec::new(),
        )
        .await
        .expect("a listing rather than an error");
    assert_eq!(
        listing
            .get("entries")
            .and_then(Value::as_list)
            .map(<[Value]>::len),
        Some(0)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn browsing_the_disk_is_not_something_a_read_only_account_can_do() {
    // It hands back the daemon's filesystem, which is nothing to do with any
    // torrent that account can see. Deluge's nearest method is `Normal` too.
    let (core, _dir) = daemon().await;
    assert_eq!(
        core.auth_level("redeluge.list_directory"),
        Some(AuthLevel::Normal)
    );
}
