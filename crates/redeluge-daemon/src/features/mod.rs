// SPDX-License-Identifier: GPL-3.0-or-later
//! The four features that were plugins in Deluge.
//!
//! Labels, automatic adding, the block list and the schedule. They are part of
//! the daemon now rather than things to install, which is why none of them has
//! an RPC namespace of its own: there is no `autoadd.set_options` to call,
//! because there is no plugin to configure. Each one reads a key of
//! `core.conf`, so `core.get_config` and `core.set_config` are the whole
//! interface and every existing client already speaks it.
//!
//! Labels are split between two places, for a reason. Which label a torrent
//! carries is a torrent option, so it lives in `torrent.rs` and `core.rs` with
//! the other options. Which labels *exist*, and what each one does to the
//! torrents in it, is in `label.rs` here: a label with nothing in it yet still
//! has to be listable, because that is the one an external client is about to
//! start using.
//!
//! Each module keeps its decisions pure and testable. This file is where those
//! decisions meet a running session, on a timer.

pub mod autoadd;
pub mod blocklist;
pub mod countrydb;
pub mod diskspace;
pub mod identity;
pub mod idlepause;
pub mod label;
pub mod scheduler;
pub mod stuck;
pub mod tracker;
pub mod webhook;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redeluge_libtorrent::{AddTorrent, Setting};
use redeluge_rencode::Value;
use serde_json::Value as Json;

use crate::activity::{did, rule, Action};
use crate::core::Core;
use crate::events::Event;

/// Starts the timers. Called once, after the daemon is up.
pub fn spawn(core: Arc<Core>) {
    tokio::spawn(watch_directories(Arc::clone(&core)));
    tokio::spawn(follow_schedule(Arc::clone(&core)));
    tokio::spawn(rotate_idle_downloads(Arc::clone(&core)));
    tokio::spawn(guard_disk_space(Arc::clone(&core)));
    tokio::spawn(apply_tracker_rules(Arc::clone(&core)));
    tokio::spawn(remove_stuck_downloads(Arc::clone(&core)));
    tokio::spawn(keep_the_peer_ledger(Arc::clone(&core)));
    tokio::spawn(announce_torrents(Arc::clone(&core)));
    tokio::spawn(watch_tracker_health(Arc::clone(&core)));
    tokio::spawn(watch_for_a_test_message(Arc::clone(&core)));
    tokio::spawn(maintain_country_database(Arc::clone(&core)));
    tokio::spawn(maintain_blocklist(core));
}

/// The same value with every `null` taken out of it.
///
/// `serde`'s `default` fills in a key that is absent, not one that is present
/// and null, so a single null in the dictionary made the whole feature
/// configuration unreadable and the feature fell back to its defaults. The Web
/// UI wrote nulls for a while: a blank number field reads as `NaN`, and `NaN`
/// serialises as `null`. That is fixed where it was written, but a
/// configuration already carrying one has to keep working, and a null has no
/// meaning for any of these settings in any case.
pub fn without_nulls(value: &Json) -> Json {
    match value {
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .filter(|(_, field)| !field.is_null())
                .map(|(name, field)| (name.clone(), without_nulls(field)))
                .collect(),
        ),
        Json::Array(items) => Json::Array(items.iter().map(without_nulls).collect()),
        other => other.clone(),
    }
}

/// Reports a configuration this cannot read, once per distinct complaint.
///
/// Every one of these is re-read on a timer, so a mistake nobody has corrected
/// would otherwise write the same line to the log every few seconds and bury
/// everything else.
pub fn warn_malformed(feature: &str, error: &str) {
    if !first_time(feature, error) {
        return;
    }
    tracing::warn!(
        feature,
        error,
        "the configuration is malformed, ignoring it"
    );
}

/// Reports something this cannot do, once per distinct complaint.
///
/// The same guard, for the rules that are re-decided on a timer: a move that
/// is refused because the destination is full is refused again a minute later,
/// and every minute after that until somebody frees the space.
pub fn warn_once(subject: &str, detail: &str) {
    if !first_time(subject, detail) {
        return;
    }
    tracing::warn!(subject, detail, "a rule could not be carried out");
}

/// Whether this complaint about this subject is a new one.
fn first_time(subject: &str, detail: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static REPORTED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let reported = REPORTED.get_or_init(Default::default);

    let Ok(mut reported) = reported.lock() else {
        return false;
    };
    if reported.get(subject).map(String::as_str) == Some(detail) {
        return false;
    }
    reported.insert(subject.to_owned(), detail.to_owned());
    true
}

/// Seconds since the Unix epoch, as the configuration stores them.
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or(0.0)
}

/// Torrents a sweep acted on: the id, and the name for the history.
///
/// A pair rather than the id alone because the interface has to say what was
/// acted on, and for a removal the torrent is gone before anybody reads it.
type Acted = Vec<(String, String)>;

/// Records something a rule did on its own, for the interface to show.
///
/// A line in the log says it too, and the log is where an operator looks when
/// something has gone wrong. This is where a person looks when nothing has:
/// "why is that paused", "what happened to that download".
fn record(core: &Core, action: crate::activity::Action) {
    core.manager.activity().record(action);
}

async fn setting(core: &Core, key: &str) -> Option<Json> {
    core.config.lock().await.get(key).cloned()
}

// --------------------------------------------------------- country database

/// Keeps a country database on disk, so peers can have flags.
///
/// The same shape as the block list above and for the same reasons: fetch,
/// cache, check on a timer, and never replace a working file with something
/// that is not one.
async fn maintain_country_database(core: Arc<Core>) {
    // A minute after start rather than at once, so a daemon that is still
    // opening its session is not also opening a connection to a web server.
    tokio::time::sleep(Duration::from_secs(60)).await;

    loop {
        let settings = countrydb::Settings::from_config(setting(&core, "countrydb").await.as_ref());

        if settings.enabled && !settings.url.is_empty() {
            let cache = countrydb::Settings::cache_path(&core.config_dir);
            // A file that is there and not stale is the whole job done.
            if !cache.is_file() || settings.is_stale(now()) {
                fetch_country_database(&core, &settings, &cache).await;
            }
        }

        // Hourly. The published file is monthly, so this is only ever asking
        // whether the week is up.
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

async fn fetch_country_database(core: &Core, settings: &countrydb::Settings, cache: &Path) {
    let (year, month) = current_year_and_month();
    let url = settings.resolved_url(year, month);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(settings.timeout))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "could not build the http client");
            return;
        }
    };

    for attempt in 1..=settings.try_times.max(1) {
        let last = match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => match response.bytes().await {
                Ok(body) => {
                    install_country_database(core, cache, &body, &url).await;
                    return;
                }
                Err(err) => err.to_string(),
            },
            Ok(response) => format!("the server answered {}", response.status()),
            Err(err) => err.to_string(),
        };
        tracing::warn!(attempt, url = %url, error = %last,
            "could not download the country database");
    }
}

/// Unpacks, checks and installs a downloaded database.
async fn install_country_database(core: &Core, cache: &Path, body: &[u8], url: &str) {
    let Some(unpacked) = countrydb::gunzip(body) else {
        tracing::warn!(url, "the country database could not be unpacked");
        return;
    };

    // Checked before it is written. Something that answers 200 with an error
    // page would otherwise replace a working database with nothing, and the
    // only symptom would be flags quietly disappearing.
    if !countrydb::looks_like_a_database(&unpacked) {
        tracing::warn!(
            url,
            bytes = unpacked.len(),
            "what was downloaded is not a MaxMind DB file, keeping the old one"
        );
        return;
    }

    // Written beside the target and renamed, so a failure halfway through
    // leaves the previous database intact rather than a truncated one.
    let temporary = cache.with_extension("mmdb.part");
    if let Err(err) = std::fs::write(&temporary, &unpacked) {
        tracing::warn!(error = %err, "could not write the country database");
        return;
    }
    if let Err(err) = std::fs::rename(&temporary, cache) {
        tracing::warn!(error = %err, "could not install the country database");
        let _ = std::fs::remove_file(&temporary);
        return;
    }

    tracing::info!(
        url,
        bytes = unpacked.len(),
        "installed the country database"
    );
    record_country_fetch(core).await;
    core.load_country_database().await;
}

/// Writes back when the database was fetched, so staleness survives a restart.
async fn record_country_fetch(core: &Core) {
    let mut config = core.config.lock().await;
    let Some(Json::Object(mut stored)) = config.get("countrydb").cloned() else {
        return;
    };
    stored.insert("last_update".to_owned(), Json::from(now()));
    if let Err(err) = config.set("countrydb", Json::Object(stored)) {
        tracing::warn!(error = %err, "could not record the country database fetch");
    }
}

/// The year and month, for a URL that names one.
fn current_year_and_month() -> (i32, u32) {
    // Days since the epoch to a civil date, by Howard Hinnant's algorithm. The
    // alternative is a date library for two numbers used once an hour.
    let days = (now() / 86_400.0) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m as u32)
}

// ------------------------------------------------------------- idle pause

/// Pauses downloads that are getting nowhere, so the queue can move.
///
/// Runs every five seconds because the interface shows a countdown off the
/// times this writes, and a coarser tick makes that countdown jump.
///
/// Everything it does is undone when the rule is turned off: a torrent this
/// paused is released on the next pass. A feature that leaves things paused
/// after being switched off is one nobody dares switch on.
async fn rotate_idle_downloads(core: Arc<Core>) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;

        let settings =
            idlepause::Settings::from_config(setting(&core, "idle_pause").await.as_ref()).sane();
        let now = now();

        let outcome = core
            .manager
            .with(move |state| sweep_idle(state, &settings, now))
            .await;

        match outcome {
            Ok((released, paused)) => {
                for (id, name) in released {
                    tracing::info!(torrent = %id, "released by the idle rule");
                    record(
                        &core,
                        Action::new(rule::IDLE, did::RESUMED, now)
                            .torrent(&id, &name)
                            .detail("its time paused was up"),
                    );
                }
                for (id, name) in paused {
                    tracing::info!(torrent = %id, "paused: idle while something was queued");
                    record(
                        &core,
                        Action::new(rule::IDLE, did::PAUSED, now)
                            .torrent(&id, &name)
                            .detail("it was transferring nothing and something was queued"),
                    );
                }
            }
            Err(err) => tracing::warn!(error = %err, "the torrent manager is not answering"),
        }
    }
}

/// One pass of the idle rule.
///
/// Pure enough to reason about: it is handed the session, the settings, the
/// time and the timers, and answers what it released and what it paused.
fn sweep_idle(
    state: &mut crate::manager::SessionState,
    settings: &idlepause::Settings,
    now: f64,
) -> (Acted, Acted) {
    use redeluge_libtorrent::{flags, FlagChange};

    let mut released = Vec::new();
    let mut paused = Vec::new();

    // One snapshot for the whole pass: deciding needs to know what is running,
    // what is waiting and what is actually paused, all as of the same moment.
    let statuses = state.session.all_torrent_status();
    let session_paused = state.session_paused;

    // Torrents this rule is holding, whose time is up or whose rule has been
    // turned off. A hold is only ever ended here or by somebody pressing
    // Resume, which `core.resume_torrent` handles: inferring it from whether
    // the torrent looks paused raced with startup, where a torrent being
    // checked is not yet reported as paused and lost its hold.
    //
    // The exception is a disk with no room on it: a hold whose time is up is
    // kept until there is somewhere to write, or this rule would start a
    // download that the disk-space rule has to stop again on its next pass.
    let low_space = state.low_space;
    let due_back: Vec<String> = state
        .torrents
        .iter()
        .filter(|(_, torrent)| torrent.options.idle_resume_at > 0.0)
        .filter(|(_, torrent)| {
            !settings.enabled || (torrent.options.idle_resume_at <= now && !low_space)
        })
        .map(|(id, _)| id.clone())
        .collect();

    for id in due_back {
        // Auto-management goes back on because the rule only ever takes
        // torrents that had it: one that is managed by hand is not the queue's
        // business and is never a candidate below.
        let change = FlagChange::new()
            .set_to(flags::PAUSED, false)
            .set_to(flags::AUTO_MANAGED, true);
        if state.session.set_flags(&id, change).is_ok() {
            let mut name = String::new();
            if let Some(torrent) = state.torrents.get_mut(&id) {
                name = torrent.options.name.clone().unwrap_or_default();
                torrent.options.idle_resume_at = 0.0;
                // The options, not just the session flags: a restart re-adds
                // every torrent from these.
                torrent.options.paused = false;
                torrent.options.auto_managed = true;
            }
            state.mark_dirty();
            state.idle_since.remove(&id);
            released.push((id, name));
        }
    }

    if !settings.enabled || session_paused {
        state.idle_since.clear();
        return (released, paused);
    }

    let mut running = 0i64;
    let mut queued = 0i64;
    let mut candidates: Vec<(String, bool)> = Vec::new();

    for status in &statuses {
        let Some(torrent) = state.torrents.get(&status.info_hash) else {
            continue;
        };
        // Already put away by this rule.
        if torrent.options.idle_resume_at > 0.0 {
            continue;
        }
        match torrent.state(status, session_paused) {
            crate::state::TorrentState::Downloading => {
                running += 1;
                // Only torrents the queue is managing. One taken out of
                // auto-management is being run by hand, and a rule that
                // paused it would be overruling a decision somebody made.
                if !torrent.options.auto_managed {
                    continue;
                }
                let idle = i64::from(status.download_payload_rate) < settings.inactive_rate;
                candidates.push((status.info_hash.clone(), idle));
            }
            crate::state::TorrentState::Queued => queued += 1,
            _ => {}
        }
    }

    // Forget torrents that have gone away or stopped downloading.
    let known: std::collections::HashSet<&String> = candidates.iter().map(|(id, _)| id).collect();
    state.idle_since.retain(|id, _| known.contains(id));

    for (id, idle) in &candidates {
        if *idle {
            state.idle_since.entry(id.clone()).or_insert(now);
        } else {
            state.idle_since.remove(id);
        }
    }

    // Nothing waiting means nothing to gain, and a torrent paused for no
    // reason is one that cannot find the peer that was about to turn up. The
    // timers are kept either way: a torrent that has been idle for four
    // minutes is still four minutes idle when something queues up behind it.
    if settings.only_when_queued && queued == 0 {
        return (released, paused);
    }

    // Longest idle first, so the one with the least to lose goes first.
    let mut due: Vec<(&String, f64)> = candidates
        .iter()
        .filter(|(_, idle)| *idle)
        .filter_map(|(id, _)| state.idle_since.get(id).map(|since| (id, *since)))
        .filter(|(_, since)| idlepause::due(*since, now, settings.grace))
        .collect();
    due.sort_by(|a, b| a.1.total_cmp(&b.1));

    for (id, _) in due {
        if running <= settings.min_active {
            break;
        }
        if settings.only_when_queued && queued == 0 {
            break;
        }

        // The flag matters as much as the pause. libtorrent's queue resumes an
        // auto-managed torrent it finds paused, within about half a minute, so
        // pausing without clearing it does nothing at all.
        let change = FlagChange::new()
            .set_to(flags::PAUSED, true)
            .set_to(flags::AUTO_MANAGED, false);
        if state.session.set_flags(id, change).is_ok() {
            let mut name = String::new();
            if let Some(torrent) = state.torrents.get_mut(id) {
                name = torrent.options.name.clone().unwrap_or_default();
                torrent.options.idle_resume_at = now + settings.pause_for as f64;
                torrent.options.paused = true;
                torrent.options.auto_managed = false;
            }
            state.mark_dirty();
            paused.push((id.clone(), name));
            running -= 1;
            queued -= 1;
        }
    }

    for (id, _) in &paused {
        state.idle_since.remove(id);
    }

    (released, paused)
}

/// What the interface shows for one torrent.
pub fn countdown_for(
    torrent: &crate::torrent::Torrent,
    idle_since: f64,
    grace: u64,
) -> idlepause::Countdown {
    idlepause::Countdown {
        idle_since,
        pause_at: idlepause::pause_at(idle_since, grace),
        resume_at: torrent.options.idle_resume_at,
    }
}

// ------------------------------------------------------------- disk space

/// Stops downloads before the disk fills, and starts them again after.
///
/// Every fifteen seconds rather than every five: free space moves slowly next
/// to a transfer rate, and each pass costs a `statvfs` per filesystem in use.
///
/// Three steps on purpose. The paths are collected on the session thread, the
/// filesystems are measured off it, and only the decision goes back. Measuring
/// inside the session thread would have put a blocking syscall in front of
/// every torrent operation, and one unresponsive network mount would then stop
/// the whole daemon rather than one rule.
async fn guard_disk_space(core: Arc<Core>) {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;

        let settings =
            diskspace::Settings::from_config(setting(&core, "disk_space").await.as_ref()).sane();

        let paths = match core.manager.with(save_paths_in_use).await {
            Ok(paths) => paths,
            Err(err) => {
                tracing::warn!(error = %err, "the torrent manager is not answering");
                continue;
            }
        };
        if paths.is_empty() {
            continue;
        }

        let free = tokio::task::spawn_blocking(move || {
            paths
                .into_iter()
                .map(|path| {
                    let free = crate::core::free_space(&path);
                    (path, free)
                })
                .collect::<std::collections::BTreeMap<String, i64>>()
        })
        .await
        .unwrap_or_default();

        match core
            .manager
            .with(move |state| sweep_space(state, &settings, &free))
            .await
        {
            Ok((released, paused)) => {
                let at = now();
                for (id, name) in released {
                    tracing::info!(torrent = %id, "released: there is room again");
                    record(
                        &core,
                        Action::new(rule::DISK, did::RESUMED, at)
                            .torrent(&id, &name)
                            .detail("there is room on the disk again"),
                    );
                }
                for (id, name) in paused {
                    tracing::warn!(torrent = %id, "paused: not enough free space to keep writing");
                    record(
                        &core,
                        Action::new(rule::DISK, did::PAUSED, at)
                            .torrent(&id, &name)
                            .detail("the disk it writes to is nearly full"),
                    );
                }
            }
            Err(err) => tracing::warn!(error = %err, "the torrent manager is not answering"),
        }
    }
}

/// Every filesystem the session is writing to, once each.
fn save_paths_in_use(state: &mut crate::manager::SessionState) -> Vec<String> {
    let mut paths: Vec<String> = state
        .session
        .all_torrent_status()
        .into_iter()
        .map(|status| status.save_path)
        .filter(|path| !path.is_empty())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// One pass of the disk-space rule.
///
/// Per path rather than per daemon: a download landing on a disk with room is
/// not the reason another disk is full, and stopping it would fix nothing.
fn sweep_space(
    state: &mut crate::manager::SessionState,
    settings: &diskspace::Settings,
    free: &std::collections::BTreeMap<String, i64>,
) -> (Acted, Acted) {
    use redeluge_libtorrent::{flags, FlagChange};

    let statuses = state.session.all_torrent_status();
    let session_paused = state.session_paused;

    let mut to_release: Vec<String> = Vec::new();
    let mut to_pause: Vec<String> = Vec::new();
    let mut low_anywhere = false;

    for status in &statuses {
        let verdict = free
            .get(&status.save_path)
            .map(|free| diskspace::judge(*free, settings))
            .unwrap_or(diskspace::Verdict::Hold);
        let Some(torrent) = state.torrents.get(&status.info_hash) else {
            continue;
        };

        if settings.enabled && verdict == diskspace::Verdict::Low {
            // Recorded even when there is nothing left to pause here, because
            // the idle rule reads it to decide whether now is the moment to
            // start a download it has been holding.
            low_anywhere = true;
        }

        if torrent.options.space_paused {
            // A hold ends when there is room again, or when the rule is turned
            // off. Never on the reading in between, and never on a reading it
            // could not take.
            if !settings.enabled || verdict == diskspace::Verdict::Recovered {
                to_release.push(status.info_hash.clone());
            }
            continue;
        }

        if !settings.enabled || verdict != diskspace::Verdict::Low {
            continue;
        }
        // A seed writes nothing, so it cannot be the reason the disk fills,
        // and pausing it would take it off the swarm for nothing.
        if status.is_seeding || status.is_finished {
            continue;
        }
        // Queued and checking count as well as downloading: a queued torrent
        // is one the queue is about to start writing, and leaving it to be
        // promoted onto a full disk is the failure this exists to prevent.
        // Anything already paused, moving or in error is left where it is.
        match torrent.state(status, session_paused) {
            crate::state::TorrentState::Downloading
            | crate::state::TorrentState::Queued
            | crate::state::TorrentState::Checking
            | crate::state::TorrentState::Allocating => to_pause.push(status.info_hash.clone()),
            _ => {}
        }
    }

    state.low_space = low_anywhere;

    let mut released = Vec::new();
    for id in to_release {
        let managed = state
            .torrents
            .get(&id)
            .map(|torrent| torrent.options.space_was_managed)
            .unwrap_or(true);
        let change = FlagChange::new()
            .set_to(flags::PAUSED, false)
            .set_to(flags::AUTO_MANAGED, managed);
        if state.session.set_flags(&id, change).is_ok() {
            let mut name = String::new();
            if let Some(torrent) = state.torrents.get_mut(&id) {
                name = torrent.options.name.clone().unwrap_or_default();
                torrent.options.space_paused = false;
                torrent.options.paused = false;
                torrent.options.auto_managed = managed;
            }
            state.mark_dirty();
            released.push((id, name));
        }
    }

    let mut paused = Vec::new();
    for id in to_pause {
        // Auto-management has to go, or libtorrent's queue starts the torrent
        // again within half a minute and the pause never takes.
        let change = FlagChange::new()
            .set_to(flags::PAUSED, true)
            .set_to(flags::AUTO_MANAGED, false);
        if state.session.set_flags(&id, change).is_ok() {
            let mut name = String::new();
            if let Some(torrent) = state.torrents.get_mut(&id) {
                name = torrent.options.name.clone().unwrap_or_default();
                torrent.options.space_was_managed = torrent.options.auto_managed;
                torrent.options.space_paused = true;
                torrent.options.paused = true;
                torrent.options.auto_managed = false;
            }
            state.mark_dirty();
            state.idle_since.remove(&id);
            paused.push((id, name));
        }
    }

    (released, paused)
}

// ------------------------------------------------------------ peer ledger

/// Keeps a running account of what each peer has done.
///
/// libtorrent's per-peer counters belong to a connection and die with it, so
/// nothing in a torrent client can answer "what has that address ever given
/// back" — every visit looks unremarkable on its own. This samples them and
/// adds the differences, so the answer accumulates.
///
/// Off unless asked for. It costs a peer list per torrent per pass and a
/// record per address, which nobody should pay for a question they never ask.
async fn keep_the_peer_ledger(core: Arc<Core>) {
    // How often the sample is taken. Short enough that a peer that connects,
    // takes what it wants and leaves is caught at least once; long enough that
    // a library of a few hundred torrents is not asked about its peers
    // constantly.
    const EVERY: Duration = Duration::from_secs(15);
    // The ledger is written on the way out, and on this timer as well: a
    // daemon that is killed rather than stopped should lose minutes, not days.
    const SAVE_EVERY: Duration = Duration::from_secs(300);

    let mut last_save = std::time::Instant::now();

    loop {
        tokio::time::sleep(EVERY).await;

        let settings =
            crate::peers::Settings::from_config(setting(&core, "peers").await.as_ref()).sane();
        if !settings.enabled {
            continue;
        }

        let now = now();
        let ttl = settings.ttl_seconds();
        let save = last_save.elapsed() >= SAVE_EVERY;
        if save {
            last_save = std::time::Instant::now();
        }

        let outcome = core
            .manager
            .with(move |state| sample_peers(state, now, ttl, save))
            .await;

        match outcome {
            Ok((seen, forgotten)) => {
                if forgotten > 0 {
                    tracing::debug!(forgotten, "peers dropped from the ledger");
                }
                tracing::trace!(seen, "peers sampled");
            }
            Err(err) => tracing::warn!(error = %err, "the torrent manager is not answering"),
        }
    }
}

/// One pass of the sampler: every peer of every torrent that has any.
///
/// Done inside the session thread, because that is where the peer lists are,
/// and in one pass rather than one call per torrent: the cost of this feature
/// is dominated by how often it crosses that boundary.
fn sample_peers(
    state: &mut crate::manager::SessionState,
    now: f64,
    ttl: f64,
    save: bool,
) -> (usize, usize) {
    let statuses = state.session.all_torrent_status();

    // Gathered first, then folded in, so the ledger's lock is taken once and
    // not once per torrent.
    let mut observations: Vec<crate::peers::Observation> = Vec::new();

    for status in &statuses {
        // Nothing to ask about, and the call is not free.
        if status.num_peers <= 0 {
            continue;
        }
        let Ok(peers) = state.session.peers(&status.info_hash) else {
            continue;
        };
        if peers.is_empty() {
            continue;
        }

        // The fingerprint of what this torrent carries, computed the first
        // time it is wanted and kept: the file list cannot change once the
        // metadata is there, and this is the one call in the pass that has to
        // go and fetch something.
        let content = match state.torrents.get(&status.info_hash) {
            Some(torrent) => match &torrent.content_id {
                Some(id) => id.clone(),
                None => {
                    let files = state.session.files(&status.info_hash).unwrap_or_default();
                    let id = crate::torrent::content_fingerprint(&files);
                    if let Some(torrent) = state.torrents.get_mut(&status.info_hash) {
                        // Stored even when empty: an empty answer means the
                        // metadata is not there yet, and asking again next
                        // pass is the point.
                        if !id.is_empty() {
                            torrent.content_id = Some(id.clone());
                        }
                    }
                    id
                }
            },
            None => continue,
        };

        for peer in peers {
            observations.push(crate::peers::Observation {
                address: peer.ip.clone(),
                client: peer.client.clone(),
                torrent: status.info_hash.clone(),
                content: content.clone(),
                // Ours is what we sent them: libtorrent's `total_upload` is
                // the upload of this connection, which is us to them.
                sent: peer.total_upload,
                received: peer.total_download,
            });
        }
    }

    let seen = observations.len();
    let Ok(mut ledger) = state.peers.lock() else {
        return (0, 0);
    };
    for observation in &observations {
        ledger.observe(observation, now);
    }
    let forgotten = ledger.forget_old(now, ttl);
    let should_save = save;
    drop(ledger);

    if should_save {
        state.save_peers();
    }

    (seen, forgotten)
}

// ------------------------------------------------------------ tracker rules

/// Applies each tracker's rules to the torrents that announce to it.
///
/// Three rules, in the order that matters when a torrent qualifies for more
/// than one: label it, move it, then remove it. Labelling first so a torrent
/// is filed correctly while it still exists; removing last for the same
/// reason. A torrent whose move is starting in this pass is not removed in it,
/// because the removal would land in the middle of the move.
///
/// The removal is the one rule in the daemon that deletes something nobody
/// pressed a button for, so it is written to be boring and to be refused
/// easily: off unless a named tracker has it on, only ever a torrent
/// libtorrent calls finished, measured from a completion time libtorrent
/// recorded, and the files kept unless that tracker's entry says otherwise.
///
/// A minute between passes. The delays are in hours, so a sweep any more eager
/// than that would only be work; a sweep any lazier would make a delay of zero
/// look broken.
async fn apply_tracker_rules(core: Arc<Core>) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        let settings =
            tracker::Settings::from_config(setting(&core, "tracker").await.as_ref()).sane();
        // Nothing configured is the normal case, and what is behind this check
        // is a status sweep of the whole library.
        if !settings.any_rule() {
            continue;
        }

        let now = now();
        let work = match core
            .manager
            .with(move |state| decide_tracker_work(state, &settings, now))
            .await
        {
            Ok(work) => work,
            Err(err) => {
                tracing::warn!(error = %err, "the torrent manager is not answering");
                continue;
            }
        };

        // Limits first: a torrent about to be moved or removed may as well
        // spend its last minutes obeying the rule it is under, and a torrent
        // that is merely new gets them before it has downloaded much.
        for wanted in work.limits {
            let options = Value::Dict(
                wanted
                    .options
                    .iter()
                    .map(|(key, value)| {
                        (Value::Str(key.clone()), crate::core::json_to_value(value))
                    })
                    .collect(),
            );
            match core
                .apply_torrent_options(vec![wanted.id.clone()], options)
                .await
            {
                Ok(_) => {
                    core.manager
                        .with({
                            let id = wanted.id.clone();
                            let fingerprint = wanted.fingerprint.clone();
                            move |state| {
                                state.tracker_limits.insert(id, fingerprint);
                            }
                        })
                        .await
                        .ok();
                    tracing::info!(torrent = %wanted.id, tracker = %wanted.host,
                        "limits applied by the tracker's rule");
                    record(
                        &core,
                        Action::new(rule::TRACKER, did::LIMITED, now)
                            .torrent(&wanted.id, &wanted.name)
                            .detail(format!("{} sets the limits here", wanted.host)),
                    );
                }
                Err(err) => tracing::warn!(torrent = %wanted.id, error = %err.message,
                    "could not apply a tracker's limits"),
            }
        }

        for wanted in work.labels {
            // The same call `label.set_torrent` makes, so the label is created
            // if it has gone missing and whatever it applies is applied.
            match core.assign_label(&wanted.id, &wanted.label).await {
                Ok(()) => {
                    tracing::info!(torrent = %wanted.id, tracker = %wanted.host,
                        label = %wanted.label, "labelled by the tracker's rule");
                    record(
                        &core,
                        Action::new(rule::TRACKER, did::LABELLED, now)
                            .torrent(&wanted.id, &wanted.name)
                            .detail(format!(
                                "{} says its torrents go in {}",
                                wanted.host, wanted.label
                            )),
                    );
                }
                Err(err) => tracing::warn!(torrent = %wanted.id, label = %wanted.label,
                    error = %err.message,
                    "could not label a torrent its tracker's rule names"),
            }
        }

        for candidate in work.moves {
            start_tracker_move(&core, &candidate, now).await;
        }

        for removal in work.removals {
            remove_for_tracker(&core, &removal, now).await;
        }
    }
}

/// What one pass of the tracker rules has decided.
#[derive(Default)]
struct TrackerWork {
    limits: Vec<TrackerLimit>,
    labels: Vec<TrackerLabel>,
    moves: Vec<TrackerMove>,
    removals: Vec<TrackerRemoval>,
}

/// Limits a tracker's rule wants a torrent to carry.
struct TrackerLimit {
    id: String,
    name: String,
    host: String,
    /// The rule's limits as `core.set_torrent_options` takes them.
    options: Vec<(String, Json)>,
    /// What was applied, so the same rule is not applied twice.
    fingerprint: String,
}

/// Which torrent, and which tracker's rule is why.
///
/// The name travels with the id everywhere here because the history has to say
/// what was acted on after the fact, and after a removal there is nothing left
/// to look it up in.
struct TrackerLabel {
    id: String,
    name: String,
    host: String,
    label: String,
}

/// A move a tracker's rule asks for, before anybody has looked at the disk.
struct TrackerMove {
    id: String,
    name: String,
    host: String,
    from: String,
    to: String,
    /// What will have to fit at the destination.
    size: i64,
}

/// A removal a tracker's rule asks for.
struct TrackerRemoval {
    id: String,
    name: String,
    host: String,
    with_data: bool,
}

/// One pass over the library: what each tracker's rules have to say about it.
///
/// Decided here and carried out by the caller, so the session thread is not
/// held while a disk is measured or a label is written to the configuration.
fn decide_tracker_work(
    state: &mut crate::manager::SessionState,
    settings: &tracker::Settings,
    now: f64,
) -> TrackerWork {
    let mut work = TrackerWork::default();

    for status in state.session.all_torrent_status() {
        if !state.torrents.contains_key(&status.info_hash) {
            continue;
        }

        // The sidebar's grouping, so a rule applies to the row somebody set it
        // on. The two have to be the same function or the rule silently never
        // matches. Looked up before the torrent is borrowed, because it may
        // fill the cache behind it.
        let trackers = state.tracker_list(&status, false);
        let Some(torrent) = state.torrents.get(&status.info_hash) else {
            continue;
        };
        let announced = crate::torrent::current_tracker(&status.current_tracker, &trackers);
        let host = crate::torrent::tracker_host(&announced);

        let Some(options) = settings.options(&host) else {
            continue;
        };
        let id = status.info_hash.clone();

        // When the rules that can be undone consider this finished, and when
        // the one that cannot does. They differ, and `finished_at` says why.
        let finished = tracker::finished_at(status.completed_time, status.added_time, false);
        let finished_strictly =
            tracker::finished_at(status.completed_time, status.added_time, true);

        // ----------------------------------------------------------- limits
        // Applied when this torrent is first seen under the rule, and again
        // when the rule changes. Not every pass: a limit somebody sets by hand
        // is theirs to keep until the rule itself moves.
        if options.limits() {
            let fingerprint = options.limits_fingerprint();
            let applied = state.tracker_limits.get(&id);
            if applied != Some(&fingerprint) {
                work.limits.push(TrackerLimit {
                    id: id.clone(),
                    name: status.name.clone(),
                    host: host.clone(),
                    options: options.to_torrent_options(),
                    fingerprint,
                });
            }
        }

        // ------------------------------------------------------------ label
        if options.labels() {
            // Normalised here rather than compared raw: `assign_label` stores
            // the normal form, so comparing against what was typed would find
            // them different every minute and relabel for ever.
            let wanted = crate::core::normalise_label(&options.label);
            let on_arrival = options.label_on_add && torrent.options.label.is_empty();
            let on_completion = options.label_when_done
                && status.is_finished
                && tracker::due(finished, now, options.label_after_hours);

            if (on_arrival || on_completion)
                && torrent.options.label != wanted
                && !wanted.is_empty()
            {
                work.labels.push(TrackerLabel {
                    id: id.clone(),
                    name: status.name.clone(),
                    host: host.clone(),
                    label: wanted,
                });
            }
        }

        // ------------------------------------------------------------- move
        let mut moving = status.moving_storage;
        if options.moves()
            && status.is_finished
            && !status.moving_storage
            && tracker::due(finished, now, options.move_after_hours)
            && status.save_path.trim_end_matches('/') != options.move_path.trim_end_matches('/')
        {
            moving = true;
            work.moves.push(TrackerMove {
                id: id.clone(),
                name: status.name.clone(),
                host: host.clone(),
                from: status.save_path.clone(),
                to: options.move_path.clone(),
                // What the files will take at the other end. `total_wanted` is
                // the files that were asked for; `total_done` covers a torrent
                // that has more on disk than it now wants.
                size: status.total_wanted.max(status.total_done),
            });
        }

        // ----------------------------------------------------------- remove
        // Not while it is being moved, in this pass or already: removing a
        // torrent out from under libtorrent's own file copy is the one way to
        // end up with half of it in each place.
        if options.removes()
            && status.is_finished
            && !moving
            && tracker::due(finished_strictly, now, options.remove_after_hours)
        {
            work.removals.push(TrackerRemoval {
                id,
                name: status.name.clone(),
                host,
                with_data: options.remove_data,
            });
        }
    }

    work
}

/// Starts a move a tracker's rule asked for, if the destination has room.
///
/// The disk is measured here rather than in the pass above, so the session
/// thread is not held for a `statvfs` per torrent.
async fn start_tracker_move(core: &Core, candidate: &TrackerMove, at: f64) {
    let destination = std::path::Path::new(&candidate.to);
    let source = std::path::Path::new(&candidate.from);

    let free = free_space_at(destination);
    let same = on_the_same_filesystem(source, destination);

    if !tracker::room_to_move(free, candidate.size, same) {
        // Once per torrent and destination: this is re-decided every minute,
        // and a disk that is full stays full for longer than that.
        warn_once(
            &format!("tracker move {} -> {}", candidate.id, candidate.to),
            &format!(
                "not moving it: {} free at the destination, {} needed with a gibibyte to spare",
                free, candidate.size
            ),
        );
        return;
    }

    let id = candidate.id.clone();
    let to = candidate.to.clone();
    let started = core
        .manager
        .with(move |state| {
            let outcome = state.session.move_storage(&id, &to);
            if outcome.is_ok() {
                // What makes the status say "Moving" until libtorrent answers
                // with the alert that updates the save path.
                if let Some(torrent) = state.torrents.get_mut(&id) {
                    torrent.moving_to = Some(to);
                }
            }
            outcome
        })
        .await;

    match started {
        Ok(Ok(())) => {
            tracing::info!(torrent = %candidate.id, tracker = %candidate.host,
                destination = %candidate.to, "moving: the tracker's rule for finished torrents");
            record(
                core,
                Action::new(rule::TRACKER, did::MOVED, at)
                    .torrent(&candidate.id, &candidate.name)
                    .detail(format!(
                        "{} says its finished torrents go to {}",
                        candidate.host, candidate.to
                    )),
            );
        }
        Ok(Err(err)) => tracing::warn!(torrent = %candidate.id, error = %err,
            "could not move a torrent its tracker's rule is due to move"),
        Err(err) => tracing::warn!(error = %err, "the torrent manager is not answering"),
    }
}

/// Removes a torrent a tracker's rule is done with.
async fn remove_for_tracker(core: &Core, removal: &TrackerRemoval, at: f64) {
    let with_data = removal.with_data;
    remove_for_rule(
        core,
        &removal.id,
        &removal.name,
        with_data,
        rule::TRACKER,
        format!(
            "{} says its finished torrents go, {}",
            removal.host,
            if with_data {
                "with their files"
            } else {
                "keeping their files"
            }
        ),
        at,
    )
    .await;
}

/// Removes a torrent because a rule said so.
///
/// One path for every removal nobody pressed a button for: the announcements,
/// the state write and what the history records cannot drift apart between one
/// rule and the next, and the next rule to delete something has one place to
/// hook into rather than fifty lines to copy.
async fn remove_for_rule(
    core: &Core,
    id: &str,
    name: &str,
    with_data: bool,
    which: &'static str,
    detail: String,
    at: f64,
) {
    // Announced before and after, like `core.remove_torrent`, because a client
    // holding the torrent's details has to be told to let go of them before
    // they stop existing.
    core.manager.announce(Event::PreTorrentRemoved {
        torrent_id: id.to_owned(),
    });

    let removed = {
        let id = id.to_owned();
        core.manager
            .with(move |state| {
                let outcome = state.session.remove_torrent(&id, with_data);
                state.torrents.remove(&id);
                state.forget(&id);
                state.mark_dirty();
                let _ = state.save_state();
                outcome
            })
            .await
    };

    match removed {
        Ok(Ok(())) => {
            tracing::info!(torrent = %id, rule = which, data = with_data,
                reason = %detail, "removed by a rule");
            record(
                core,
                Action::new(which, did::REMOVED, at)
                    .torrent(id, name)
                    .detail(detail),
            );
            core.manager.announce(Event::TorrentRemoved {
                torrent_id: id.to_owned(),
            });
        }
        Ok(Err(err)) => tracing::warn!(torrent = %id, rule = which, error = %err,
            "could not remove a torrent a rule is due to remove"),
        Err(err) => tracing::warn!(error = %err, "the torrent manager is not answering"),
    }
}

// ------------------------------------------------------- downloads that stall

/// Throws away the downloads of a label that never start.
///
/// A torrent that has been trying for hours and has not got a single byte is
/// not slow, it is dead: a magnet nobody is seeding, a `.torrent` for content
/// that has left the swarm, a private tracker that has stopped answering for
/// it. It sits in the list looking exactly like one that is merely between
/// peers, and the only way to tell is to remember when it was added.
///
/// So the rule is per label, off until somebody turns it on by name, and the
/// state it acts on is deliberately narrow — [`decide_stuck_removals`] says
/// what it will not take. It is the second rule in the daemon that deletes
/// files nobody pressed a button for, and like the first it is written to be
/// boring.
///
/// A minute between passes, the same as the tracker rules, and for the same
/// reason: the delay is in hours, so anything more eager is only work.
async fn remove_stuck_downloads(core: Arc<Core>) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        let labels = label::Settings::from_config(setting(&core, "label").await.as_ref());
        let global = stuck::Settings::from_config(setting(&core, "stuck").await.as_ref()).rule();
        // Nothing configured is the normal case, and what is behind this check
        // is a status sweep of the whole library. The clocks are kept by that
        // same sweep, so when nothing is watching there is nothing to keep.
        if global.is_none() && !labels.any_stuck_rule() {
            continue;
        }

        let now = now();
        let removals = match core
            .manager
            .with(move |state| decide_stuck_removals(state, &labels, global))
            .await
        {
            Ok(removals) => removals,
            Err(err) => {
                tracing::warn!(error = %err, "the torrent manager is not answering");
                continue;
            }
        };

        for removal in removals {
            let who = if removal.scope.is_empty() {
                "this daemon".to_owned()
            } else {
                removal.scope.clone()
            };
            let how_long = describe_delay(removal.hours);

            match removal.action {
                stuck::Action::Remove => {
                    remove_for_rule(
                        &core,
                        &removal.id,
                        &removal.name,
                        removal.with_data,
                        rule::LABEL,
                        format!(
                            "{who} takes downloads that have done nothing {how_long}, {}",
                            if removal.with_data {
                                "with their files"
                            } else {
                                "keeping their files"
                            }
                        ),
                        now,
                    )
                    .await;
                }
                stuck::Action::Pause => pause_stuck(&core, &removal, &who, &how_long, now).await,
            }
        }
    }
}

/// A torrent a label's rule has given up on.
struct StuckRemoval {
    id: String,
    /// Carried with the id because after a removal there is nothing left to
    /// look the name up in, and the history has to say what went.
    name: String,
    /// The label whose rule took it, or empty for the daemon's own rule. The
    /// history line says which, because "why did that go" has two answers now.
    scope: String,
    hours: f64,
    action: stuck::Action,
    /// The label to file it under when pausing. Empty to leave it alone.
    label_as: String,
    with_data: bool,
}

/// One pass over the library: which downloads have stopped getting anywhere.
///
/// Every torrent is measured, and which rule measures it is the most specific
/// one that exists: its label's, if that label has one, otherwise the global
/// rule. A label that has the rule off is a deliberate exemption from the
/// global one, not a fall-through — a label is how somebody says "these are
/// different", and it would be a poor rule that ignored them saying it.
///
/// What this will not take, each for a reason:
///
/// * A torrent that got **further along than the rule allows**. The ceiling
///   defaults to nothing at all, so out of the box this is still only about
///   downloads that never started.
/// * A **paused** torrent, including one the queue is holding back. Somebody
///   paused it, or the daemon did, and neither is the torrent failing.
/// * A **finished** one. A torrent whose files are all deselected is finished
///   with nothing downloaded, and it is finished on purpose.
/// * One that is **checking** or **moving**. Neither has had a chance to
///   download yet, and removing a torrent out from under libtorrent's own file
///   handling is how half of it ends up in each place.
/// * One whose clock has not been set yet, which is every torrent on the first
///   pass after a start. `stuck.rs` says why that direction is the safe one.
///
/// The marks are updated here as well, because the decision and the clock have
/// to be taken from the same status: reading them in two passes would let a
/// byte arrive between the two and be counted in neither.
fn decide_stuck_removals(
    state: &mut crate::manager::SessionState,
    labels: &label::Settings,
    global: Option<stuck::Rule>,
) -> Vec<StuckRemoval> {
    use redeluge_libtorrent::TorrentState as LtState;

    let mut out = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // Which tracker domains are answering at all. A tracker that is down is
    // the reason nothing is arriving, and the torrents behind it are not the
    // thing that is broken: see `held_back_by`.
    let health = tracker_health(state);

    for status in state.session.all_torrent_status() {
        let Some(torrent) = state.torrents.get(&status.info_hash) else {
            continue;
        };
        let label = torrent.options.label.clone();
        let name = torrent.display_name(&status);
        seen.insert(status.info_hash.clone());

        // The clock, whether or not any rule is watching it: a rule turned on
        // this afternoon should not find every torrent already overdue.
        let mark = stuck::mark_for(
            status.total_done,
            status.active_time,
            state.progress_marks.get(&status.info_hash),
        );

        let rule = labels
            .options(&label)
            .map(|options| options.stuck_rule())
            .unwrap_or_else(|| global.clone());

        let takeable = !status.is_finished
            && !status.is_paused
            && !status.moving_storage
            && !matches!(
                status.state,
                LtState::CheckingFiles | LtState::CheckingResumeData
            );

        if let (Some(rule), true) = (&rule, takeable) {
            if stuck::is_stuck(
                status.progress,
                status.active_time,
                state.progress_marks.get(&status.info_hash),
                rule,
            ) {
                // The interlock. Everything else here asks "is this torrent
                // getting anywhere"; this asks "is there anything it could be
                // getting anywhere through". A tracker outage looks exactly
                // like a dead swarm from the inside, and without this a
                // tracker down for an afternoon takes the library with it.
                match held_back_by(&status, state, &health) {
                    Some(reason) => {
                        tracing::debug!(torrent = %status.info_hash, %reason,
                            "a stuck rule is due but held back");
                    }
                    None => out.push(StuckRemoval {
                        id: status.info_hash.clone(),
                        name,
                        scope: if labels.options(&label).is_some() {
                            label
                        } else {
                            String::new()
                        },
                        hours: rule.seconds() / 3600.0,
                        action: rule.action,
                        label_as: rule.label.clone(),
                        with_data: rule.remove_data,
                    }),
                }
            }
        }

        state.progress_marks.insert(status.info_hash, mark);
    }

    // A torrent removed while this map was not looking would sit here for
    // ever. `forget` covers the ordinary removal; this covers every other way
    // one can leave.
    state.progress_marks.retain(|id, _| seen.contains(id));

    out
}

/// Stops a download that is getting nowhere, and files it where it can be seen.
///
/// The gentle half of the rule, and the one it defaults to. Nothing is deleted
/// and nothing is hidden: the torrent stops, keeps whatever it had, and lands
/// in a label if the rule names one — which is also how it stops being
/// reconsidered every minute, because a label with the rule off is an
/// exemption.
async fn pause_stuck(core: &Core, stopped: &StuckRemoval, who: &str, how_long: &str, at: f64) {
    use redeluge_libtorrent::{flags, FlagChange};

    let id = stopped.id.clone();
    // The flag matters as much as the pause: libtorrent's queue resumes an
    // auto-managed torrent it finds paused, so pausing without clearing it
    // does nothing at all. The idle rule learned this first.
    let paused = core
        .manager
        .with(move |state| {
            let change = FlagChange::new()
                .set_to(flags::PAUSED, true)
                .set_to(flags::AUTO_MANAGED, false);
            let outcome = state.session.set_flags(&id, change);
            if outcome.is_ok() {
                if let Some(torrent) = state.torrents.get_mut(&id) {
                    torrent.options.paused = true;
                    torrent.options.auto_managed = false;
                }
                state.mark_dirty();
            }
            outcome
        })
        .await;

    match paused {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            tracing::warn!(torrent = %stopped.id, error = %err,
                "could not pause a download that is getting nowhere");
            return;
        }
        Err(err) => {
            tracing::warn!(error = %err, "the torrent manager is not answering");
            return;
        }
    }

    let mut filed = String::new();
    if !stopped.label_as.is_empty() {
        match core.assign_label(&stopped.id, &stopped.label_as).await {
            Ok(()) => filed = format!(" and put in {}", stopped.label_as),
            Err(err) => tracing::warn!(torrent = %stopped.id, label = %stopped.label_as,
                error = %err.message, "could not label a download it paused"),
        }
    }

    tracing::info!(torrent = %stopped.id, rule = %who, "paused: it was getting nowhere");
    record(
        core,
        Action::new(rule::LABEL, did::PAUSED, at)
            .torrent(&stopped.id, &stopped.name)
            .detail(format!(
                "{who} stops downloads that have done nothing {how_long}{filed}"
            )),
    );
}

/// One word per tracker domain: whether it is answering.
///
/// The same arithmetic the sidebar colours its rows with, so the rule and the
/// interface cannot disagree about which tracker is down.
fn tracker_health(
    state: &mut crate::manager::SessionState,
) -> std::collections::BTreeMap<String, String> {
    tracker_totals(state)
        .into_iter()
        .map(|(host, totals)| (host, totals.health().to_owned()))
        .collect()
}

/// The same sweep, undivided: what every tracker domain adds up to.
///
/// The rules want the one word and nothing else; a message about an outage
/// wants the count of torrents behind it and the sentence the tracker refused
/// with, and neither survives being reduced to a colour.
fn tracker_totals(
    state: &mut crate::manager::SessionState,
) -> std::collections::BTreeMap<String, crate::trackerinfo::Totals> {
    let session_paused = state.session_paused;
    let rows: Vec<crate::trackerinfo::Row> = state
        .session
        .all_torrent_status()
        .into_iter()
        .filter_map(|status| {
            let torrent = state.torrents.get(&status.info_hash)?;
            let trackers = state
                .session
                .trackers(&status.info_hash)
                .unwrap_or_default();
            Some(crate::trackerinfo::Row {
                state: torrent.state(&status, session_paused),
                current: crate::torrent::current_tracker(&status.current_tracker, &trackers),
                trackers,
                tracker_status: torrent.tracker_status.clone(),
                size: status.total_wanted,
                downloaded: status.all_time_download,
                uploaded: status.all_time_upload,
                seeds: i64::from(status.num_complete),
                peers: i64::from(status.num_incomplete),
                next_announce: status.next_announce,
            })
        })
        .collect();

    crate::trackerinfo::by_domain(&rows)
}

/// Why this torrent is not being acted on, though its rule says it is due.
///
/// One reason so far, and it is the important one: a tracker that is failing
/// every announce is why no bytes are arriving, and the torrents behind it are
/// not the thing that is broken. A five hour outage with a four hour rule
/// would otherwise be a library-wide deletion, and the rule would have been
/// working exactly as written.
///
/// Not a setting. An interlock somebody can switch off is an interlock that
/// will be off on the day it was needed.
fn held_back_by(
    status: &redeluge_libtorrent::TorrentStatus,
    state: &mut crate::manager::SessionState,
    health: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    let trackers = state.tracker_list(status, false);
    let announced = crate::torrent::current_tracker(&status.current_tracker, &trackers);
    let host = crate::torrent::tracker_host(&announced);
    if host.is_empty() {
        return None;
    }
    (health.get(&host).map(String::as_str) == Some("down"))
        .then(|| format!("{host} is not answering"))
}

/// A delay as the clause that finishes the sentence the history keeps.
///
/// A clause rather than a number, because the two cases do not take the same
/// preposition: a rule with no delay is not waiting *for* anything, it takes a
/// torrent that has never moved a byte since it started.
fn describe_delay(hours: f64) -> String {
    if hours <= 0.0 {
        return "since they started".to_owned();
    }
    if (hours - 1.0).abs() < f64::EPSILON {
        return "for an hour".to_owned();
    }
    format!("for {} hours", trim_number(hours))
}

/// A number without the trailing zeroes a float prints, so a history line says
/// `4 hours` rather than `4 hours` spelled `4.000000000000001`.
fn trim_number(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_owned()
}

/// Free space on the filesystem a path is on, or would be created on.
///
/// The destination of a move usually does not exist yet, and `statvfs` on a
/// path that is not there answers nothing at all. What is being asked is which
/// filesystem the files will land on, and the nearest directory that does
/// exist is on it.
fn free_space_at(path: &Path) -> i64 {
    match nearest_existing(path) {
        Some(existing) => crate::core::free_space(&existing.to_string_lossy()),
        None => -1,
    }
}

/// Whether a move between these two is a rename rather than a copy.
///
/// The case somebody runs into first: the rule points at a subdirectory of
/// where the files already are, nothing is written, and refusing it for want
/// of a spare copy's worth of room would be nonsense. The opposite case is the
/// one the rule exists for, and it looks identical from the path alone: that
/// subdirectory is a mount point for another disk.
fn on_the_same_filesystem(source: &Path, destination: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let device = |path: &Path| {
            nearest_existing(path)
                .and_then(|existing| std::fs::metadata(existing).ok())
                .map(|meta| meta.dev())
        };
        match (device(source), device(destination)) {
            (Some(one), Some(other)) => one == other,
            // Unknown is not "the same": the answer only ever waives the space
            // check, so being wrong has to cost a refused move, not a full disk.
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (source, destination);
        false
    }
}

/// The path itself, or the nearest of its parents that exists.
fn nearest_existing(path: &Path) -> Option<&Path> {
    let mut current = path;
    loop {
        if current.exists() {
            return Some(current);
        }
        current = current.parent()?;
        if current.as_os_str().is_empty() {
            return None;
        }
    }
}

// ----------------------------------------------------------- notifications

/// Posts a message somewhere when a torrent finishes, arrives or breaks.
///
/// Driven by the same event stream every client subscribes to, so there is one
/// idea of what "finished" means rather than a second one written for this.
async fn announce_torrents(core: Arc<Core>) {
    use tokio::sync::broadcast::error::RecvError;

    let mut events = core.manager.subscribe();
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            // The stream is bounded and this task is slow by nature, so
            // falling behind is possible. Missing a message is not worth
            // stopping for; not knowing it happened would be.
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "notifications fell behind the event stream");
                continue;
            }
            Err(RecvError::Closed) => return,
        };

        let (trigger, id) = match &event {
            Event::TorrentFinished { torrent_id } => {
                (webhook::Trigger::Finished, torrent_id.clone())
            }
            // Only torrents that arrived while running. `from_state` is the
            // restore at startup, which would otherwise announce the whole
            // session every time the daemon restarts.
            Event::TorrentAdded {
                torrent_id,
                from_state,
            } if !from_state => (webhook::Trigger::Added, torrent_id.clone()),
            Event::TorrentStateChanged { torrent_id, state } if state == "Error" => {
                (webhook::Trigger::Error, torrent_id.clone())
            }
            // A torrent that is downloading again is one whose next completion
            // is news again, whatever was announced about the last one.
            Event::TorrentStateChanged { torrent_id, state } if state == "Downloading" => {
                let id = torrent_id.clone();
                let _ = core.manager.spawn(move |state| {
                    if let Some(torrent) = state.torrents.get_mut(&id) {
                        if torrent.options.announced_finished {
                            torrent.options.announced_finished = false;
                            state.mark_dirty();
                        }
                    }
                });
                continue;
            }
            _ => continue,
        };

        let settings =
            webhook::Settings::from_config(setting(&core, "webhook").await.as_ref()).sane();
        if !settings.enabled || !settings.wants(trigger) || settings.usable().next().is_none() {
            continue;
        }

        let Some(notice) = describe(&core, &id, trigger).await else {
            continue;
        };
        // Sending is a network round trip per destination, with retries. On
        // this task it would hold up every event behind it.
        tokio::spawn(deliver(settings, trigger, notice));
    }
}

/// What has been said about each tracker domain, and how many sweeps have
/// disagreed with it since: the whole memory the outage messages keep.
type TrackerNews = std::collections::HashMap<String, (bool, u8)>;

/// Whether this domain's health is news, and which way it went.
///
/// `Some(true)` is a tracker that has stopped answering, `Some(false)` one that
/// is answering again, and nothing at all is the usual answer.
///
/// A domain nobody has said anything about counts as up, so the first message
/// anybody gets is about a tracker that is down — including one that was
/// already down when the daemon started, which is the case somebody most wants
/// to hear about.
fn tracker_news(reported: &mut TrackerNews, host: &str, health: &str) -> Option<bool> {
    /// How many sweeps in a row a domain has to have changed its mind before
    /// the change is worth a message. One sweep is a single dropped announce
    /// away from crying wolf, and a tracker that misses one announce has not
    /// gone anywhere.
    const CONFIRM: u8 = 2;

    let down = match health {
        "down" => true,
        "ok" | "warning" => false,
        // Listed and never tried, or every torrent on it is paused. Nothing
        // has been confirmed either way, so nothing has changed and the run of
        // disagreeing sweeps is left where it was.
        _ => return None,
    };

    let seen = reported.entry(host.to_owned()).or_insert((false, 0));
    if seen.0 == down {
        seen.1 = 0;
        return None;
    }
    seen.1 += 1;
    if seen.1 < CONFIRM {
        return None;
    }
    *seen = (down, 0);
    Some(down)
}

/// Says when a tracker stops answering, and again when it starts.
///
/// Not driven by the event stream, because there is no event: a tracker going
/// away is the absence of one. It is a sweep of the same arithmetic the
/// sidebar colours its rows with, so a message and a red row cannot disagree
/// about which tracker is down.
async fn watch_tracker_health(core: Arc<Core>) {
    // Per domain: what was last reported, and how many sweeps have disagreed
    // with it since.
    let mut reported: TrackerNews = std::collections::HashMap::new();

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        let settings =
            webhook::Settings::from_config(setting(&core, "webhook").await.as_ref()).sane();
        if !settings.enabled
            || !settings.wants(webhook::Trigger::TrackerDown)
            || settings.usable().next().is_none()
        {
            // Nothing watched is nothing remembered. Switching this on is not
            // a reason to hear about an outage that started while it was off,
            // and behind the check is a status sweep of the whole library.
            reported.clear();
            continue;
        }

        let totals = match core.manager.with(tracker_totals).await {
            Ok(totals) => totals,
            Err(err) => {
                tracing::warn!(error = %err, "the torrent manager is not answering");
                continue;
            }
        };

        for (host, domain) in &totals {
            let Some(down) = tracker_news(&mut reported, host, domain.health()) else {
                continue;
            };

            let trigger = if down {
                webhook::Trigger::TrackerDown
            } else {
                webhook::Trigger::TrackerUp
            };
            let notice = webhook::Notice {
                name: host.clone(),
                tracker: host.clone(),
                torrents: domain.torrents,
                // Only the failure has a sentence to carry; a tracker that is
                // answering has said nothing worth repeating.
                message: if down {
                    domain.message.clone()
                } else {
                    String::new()
                },
                ..webhook::Notice::default()
            };

            let what = if down {
                "stopped answering"
            } else {
                "is answering again"
            };
            tracing::info!(tracker = %host, torrents = domain.torrents, "a tracker {what}");
            tokio::spawn(deliver(settings.clone(), trigger, notice));
        }

        // A domain whose last torrent was removed is not a tracker that came
        // back, and the next one to use that host starts from nothing said.
        reported.retain(|host, _| totals.contains_key(host));
    }
}

/// Gathers what a message is written from, or nothing if there is no message.
///
/// Also where a completion that is not one gets dropped. libtorrent posts
/// `torrent_finished` after re-checking a torrent that was already complete,
/// which happens to every finished torrent at startup: without this, restarting
/// the daemon would announce the entire library.
///
/// Two guards rather than one, because each covers the other's hole. The flag
/// on the torrent is exact but starts false, so it says nothing about torrents
/// that finished before this feature existed. The age of the completion covers
/// those, and is the weaker rule: a restart moments after a torrent finished
/// would pass it. Together they leave nothing.
async fn describe(core: &Core, id: &str, trigger: webhook::Trigger) -> Option<webhook::Notice> {
    /// How recently a torrent must have completed for the completion to be
    /// news. Minutes rather than seconds because a re-check of a large torrent
    /// takes a while to reach the alert, and the times being compared are the
    /// completion's, not the check's.
    const FRESH: f64 = 300.0;

    let id = id.to_owned();
    let now = now();

    core.manager
        .with(move |state| {
            let status = state.session.torrent_status(&id).ok()?;
            if !state.torrents.contains_key(&id) {
                return None;
            }
            let trackers = state.tracker_list(&status, false);
            let torrent = state.torrents.get(&id)?;

            if trigger == webhook::Trigger::Finished {
                let stale = status.completed_time > 0 && now - status.completed_time as f64 > FRESH;
                if stale || torrent.options.announced_finished {
                    return None;
                }
            }

            let current = crate::torrent::current_tracker(&status.current_tracker, &trackers);

            let notice = webhook::Notice {
                torrent_id: id.clone(),
                name: torrent.display_name(&status),
                size: status.total_wanted,
                save_path: status.save_path.clone(),
                label: torrent.options.label.clone(),
                tracker: crate::torrent::tracker_host(&current),
                ratio: torrent.ratio(&status),
                // A count of the tracker's torrents means nothing to an event
                // about one of them.
                torrents: 0,
                message: match trigger {
                    webhook::Trigger::Error => torrent.message(),
                    _ => String::new(),
                },
            };

            // Marked as announced here rather than after the POST: the flag
            // means the message was written, not that it arrived. A send that
            // fails is reported in the log and not repeated, which is better
            // than a destination that comes back and gets the backlog.
            if trigger == webhook::Trigger::Finished {
                if let Some(torrent) = state.torrents.get_mut(&id) {
                    torrent.options.announced_finished = true;
                }
                state.mark_dirty();
            }

            Some(notice)
        })
        .await
        .ok()
        .flatten()
}

/// Sends one notice to every destination, and says how it went.
///
/// Returns what to show in the interface after a test: the first thing that
/// went wrong, or how many destinations took it.
async fn deliver(
    settings: webhook::Settings,
    trigger: webhook::Trigger,
    notice: webhook::Notice,
) -> String {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(settings.timeout))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "could not build the http client");
            return format!("could not build the http client: {err}");
        }
    };

    let mut sent = 0usize;
    let mut total = 0usize;
    let mut first_failure = None;

    for endpoint in settings.usable() {
        total += 1;
        let Some(delivery) = webhook::delivery(endpoint, trigger, &notice) else {
            continue;
        };

        match post(&client, &delivery, settings.try_times).await {
            Ok(()) => {
                sent += 1;
                tracing::info!(
                    kind = %endpoint.kind,
                    event = trigger.as_str(),
                    torrent = %notice.name,
                    "sent a notification"
                );
            }
            Err(err) => {
                tracing::warn!(kind = %endpoint.kind, url = %endpoint.url, error = %err,
                    "could not send a notification");
                first_failure.get_or_insert(format!("{}: {err}", endpoint.kind));
            }
        }
    }

    match first_failure {
        Some(err) => err,
        None if total == 0 => "no destination is set up".to_owned(),
        None => format!("sent to {sent} of {total}"),
    }
}

/// One POST, retried, with the body a service actually rejects reported.
async fn post(
    client: &reqwest::Client,
    delivery: &webhook::Delivery,
    tries: u32,
) -> Result<(), String> {
    let mut last = String::new();
    // Serialised here rather than through reqwest's `json`, which this build
    // does not carry: the client is compiled without its default features so
    // the daemon pulls in one TLS stack and nothing else.
    let body = match serde_json::to_vec(&delivery.body) {
        Ok(body) => body,
        Err(err) => return Err(err.to_string()),
    };

    for attempt in 1..=tries.max(1) {
        let mut request = client.post(&delivery.url);
        for (name, value) in &delivery.headers {
            request = request.header(name, value);
        }

        match request.body(body.clone()).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                // The status alone is rarely enough: Discord answers 400 with
                // a body saying which field it did not like.
                let body = response.text().await.unwrap_or_default();
                let body: String = body.chars().take(200).collect();
                last = if body.trim().is_empty() {
                    format!("the server answered {status}")
                } else {
                    format!("the server answered {status}: {}", body.trim())
                };
                // A refusal is not going to become an acceptance.
                if status.is_client_error() {
                    return Err(last);
                }
            }
            Err(err) => last = err.to_string(),
        }

        if attempt < tries.max(1) {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    Err(last)
}

/// Sends the sample message when somebody presses Send Test.
///
/// The button writes `test: true` into the settings and this clears it, rather
/// than an RPC method of its own: the method list is the frozen contract, and
/// a button is not worth breaking it for.
async fn watch_for_a_test_message(core: Arc<Core>) {
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;

        let settings =
            webhook::Settings::from_config(setting(&core, "webhook").await.as_ref()).sane();
        if !settings.test {
            continue;
        }

        // Cleared first, so a destination that takes thirty seconds to fail
        // cannot send a second copy in the meantime.
        record_test(&core, false, "sending...").await;
        let outcome = deliver(settings, webhook::Trigger::Test, webhook::Notice::sample()).await;
        tracing::info!(outcome, "sent the test notification");
        record_test(&core, false, &outcome).await;
    }
}

/// Writes the test flag and its result back into the settings.
async fn record_test(core: &Core, test: bool, outcome: &str) {
    let mut config = core.config.lock().await;
    let Some(Json::Object(mut stored)) = config.get("webhook").cloned() else {
        return;
    };
    stored.insert("test".to_owned(), Json::Bool(test));
    stored.insert("last_test".to_owned(), Json::String(outcome.to_owned()));
    if let Err(err) = config.set("webhook", Json::Object(stored)) {
        tracing::warn!(error = %err, "could not record the test notification");
    }
}

// ---------------------------------------------------------------- auto add

/// Scans the watched directories and adds what has finished arriving.
async fn watch_directories(core: Arc<Core>) {
    let mut settled = autoadd::Settled::default();
    let mut interval = 5u64;

    loop {
        tokio::time::sleep(Duration::from_secs(interval.clamp(1, 3600))).await;

        let settings = autoadd::Settings::from_config(setting(&core, "autoadd").await.as_ref());
        interval = settings.interval;
        if settings.active().next().is_none() {
            continue;
        }

        for directory in settings.active() {
            let found = match autoadd::scan(Path::new(&directory.path)) {
                Ok(found) => found,
                Err(err) => {
                    tracing::warn!(path = %directory.path, error = %err,
                        "could not read a watched directory");
                    continue;
                }
            };

            for (path, kind) in settled.ready(&found) {
                settled.forget(&path);
                let added = match kind {
                    autoadd::Kind::Torrent => add_torrent_file(&core, directory, &path).await,
                    autoadd::Kind::Magnet => add_magnet_file(&core, directory, &path).await,
                };

                if added {
                    let disposal = autoadd::disposal(directory, &path);
                    if let Err(err) = autoadd::dispose(&disposal, &path) {
                        // The torrent is added either way. Saying so matters
                        // because the file will be picked up again on the next
                        // scan and refused as a duplicate.
                        tracing::warn!(path = %path.display(), error = %err,
                            "could not dispose of a file after adding it");
                    }
                }
            }
        }
    }
}

/// Adds one `.torrent`. True if it is now the daemon's problem rather than the
/// directory's.
async fn add_torrent_file(core: &Core, directory: &autoadd::WatchDir, path: &Path) -> bool {
    let dump = match std::fs::read(path) {
        Ok(dump) => dump,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err,
                "could not read a torrent file");
            return false;
        }
    };

    let mut options = directory.options(core.torrent_defaults().await);
    options.filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let save_path = options.save_path.clone().unwrap_or_default();

    match core
        .add(AddTorrent::from_file(dump, save_path), options)
        .await
    {
        Ok(_) => {
            tracing::info!(path = %path.display(), "added from a watched directory");
            true
        }
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err.message,
                "could not add a torrent from a watched directory");
            false
        }
    }
}

/// Adds every magnet link in a `.magnet` file.
///
/// The file is disposed of if any link was added. One bad link among several
/// would otherwise leave the file in place and re-add the good ones on every
/// scan.
async fn add_magnet_file(core: &Core, directory: &autoadd::WatchDir, path: &Path) -> bool {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err,
                "could not read a magnet file");
            return false;
        }
    };

    let links = autoadd::magnet_links(&text);
    if links.is_empty() {
        tracing::warn!(path = %path.display(), "no magnet links in the file, leaving it alone");
        return false;
    }

    let mut added = 0;
    for link in &links {
        let mut options = directory.options(core.torrent_defaults().await);
        options.magnet = Some(link.clone());
        let save_path = options.save_path.clone().unwrap_or_default();

        match core
            .add(AddTorrent::from_magnet(link.clone(), save_path), options)
            .await
        {
            Ok(_) => added += 1,
            Err(err) => tracing::warn!(path = %path.display(), error = %err.message,
                "could not add a magnet from a watched directory"),
        }
    }

    if added > 0 {
        tracing::info!(path = %path.display(), added, of = links.len(),
            "added magnets from a watched directory");
    }
    added > 0
}

// --------------------------------------------------------------- scheduler

/// Applies the schedule, on the hour.
async fn follow_schedule(core: Arc<Core>) {
    let mut current: Option<scheduler::State> = None;

    loop {
        let settings = scheduler::Settings::from_config(setting(&core, "scheduler").await.as_ref());
        let (weekday, hour) = local_weekday_and_hour();
        let state = settings.state_at(weekday, hour);

        // Applying only on a change keeps this from fighting a client that
        // sets a rate limit by hand within the same hour.
        if current != Some(state) {
            tracing::info!(state = state.as_str(), "the schedule changed");
            // Only on a change, which is the same condition the rule itself
            // uses: an entry an hour, every hour, would bury everything else.
            record(
                &core,
                Action::new(rule::SCHEDULE, did::CHANGED, now())
                    .detail(format!("the schedule moved to {}", state.as_str())),
            );
            apply_schedule(&core, &settings, state).await;
            current = Some(state);
        }

        // Woken on the hour, which is when the grid can change by itself, but
        // also every minute in between, because someone who just edited the
        // grid expects it to mean something before the hour is out. Both are
        // cheap: nothing happens unless the state actually differs.
        let wait = seconds_to_the_next_hour().min(60);
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}

async fn apply_schedule(core: &Core, settings: &scheduler::Settings, state: scheduler::State) {
    match state {
        scheduler::State::Full => {
            core.apply_config().await;
            set_paused(core, false).await;
        }
        scheduler::State::Slow => {
            core.apply_config().await;
            let overrides = vec![
                Setting::int("active_limit", settings.low_active),
                Setting::int("active_downloads", settings.low_active_down),
                Setting::int("active_seeds", settings.low_active_up),
                Setting::int("download_rate_limit", kib_to_bytes(settings.low_down)),
                Setting::int("upload_rate_limit", kib_to_bytes(settings.low_up)),
            ];
            let outcome = core
                .manager
                .with(move |state| state.session.apply_settings(&overrides))
                .await;
            if let Ok(Err(err)) = outcome {
                tracing::error!(error = %err, "could not apply the reduced limits");
            }
            set_paused(core, false).await;
        }
        scheduler::State::Stopped => set_paused(core, true).await,
    }
}

async fn set_paused(core: &Core, paused: bool) {
    if let Err(err) = core.set_session_paused(paused).await {
        tracing::error!(error = %err, paused, "could not change the session pause state");
    }
}

/// Deluge's rate limits are KiB/s and libtorrent's are bytes per second, and
/// Deluge spells "no limit" as -1 where libtorrent spells it 0.
fn kib_to_bytes(rate: f64) -> i64 {
    if rate < 0.0 {
        0
    } else {
        (rate * 1024.0) as i64
    }
}

/// The local weekday, Monday as 0, and the hour.
fn local_weekday_and_hour() -> (usize, usize) {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    (
        now.weekday().num_days_from_monday() as usize,
        now.hour() as usize,
    )
}

fn seconds_to_the_next_hour() -> u64 {
    use chrono::{Local, Timelike};
    let now = Local::now();
    let past = u64::from(now.minute()) * 60 + u64::from(now.second());
    // Never zero: waking exactly on the hour twice would apply the same state
    // twice and, worse, spin if the clock reads the same second again.
    3600u64.saturating_sub(past).max(1)
}

// --------------------------------------------------------------- block list

/// Keeps the IP filter loaded and up to date.
async fn maintain_blocklist(core: Arc<Core>) {
    // The cached copy first, so a daemon that starts without a network still
    // filters with whatever it had.
    let settings = blocklist::Settings::from_config(setting(&core, "blocklist").await.as_ref());
    if settings.enabled {
        let cache = blocklist::Settings::cache_path(&core.config_dir);
        if cache.is_file() {
            match std::fs::read(&cache) {
                Ok(bytes) => install(&core, &settings, &bytes).await,
                Err(err) => tracing::warn!(error = %err, "could not read the cached block list"),
            }
        }
    }

    // What the installed filter was built from, so a changed URL or a changed
    // whitelist takes effect at the next check rather than at the next refetch.
    let mut installed_from = settings.url.clone();
    let mut installed_whitelist = settings.whitelisted.clone();
    let mut was_enabled = settings.enabled;

    loop {
        // Checked every minute rather than hourly: the download itself is
        // still governed by `check_after_days`, but a setting someone just
        // changed should not sit unread for an hour.
        tokio::time::sleep(Duration::from_secs(60)).await;
        let settings = blocklist::Settings::from_config(setting(&core, "blocklist").await.as_ref());

        if !settings.enabled {
            // Turning it off has to put the filter back, or the last list
            // stays in force until a restart.
            if was_enabled {
                let outcome = core
                    .manager
                    .with(|state| state.session.clear_ip_filter())
                    .await;
                match outcome {
                    Ok(Ok(())) => tracing::info!("the block list is off, cleared the IP filter"),
                    Ok(Err(err)) => tracing::warn!(error = %err, "could not clear the IP filter"),
                    Err(err) => {
                        tracing::warn!(error = %err, "the torrent manager is not answering")
                    }
                }
                was_enabled = false;
            }
            continue;
        }
        was_enabled = true;

        let url_changed = settings.url != installed_from;
        let whitelist_changed = settings.whitelisted != installed_whitelist;

        // A changed whitelist needs no download: the cached list is still the
        // right list, only the rules over the top of it have moved.
        if whitelist_changed && !url_changed {
            let cache = blocklist::Settings::cache_path(&core.config_dir);
            if let Ok(bytes) = std::fs::read(&cache) {
                tracing::info!("the whitelist changed, reinstalling the block list");
                install(&core, &settings, &bytes).await;
            }
            installed_whitelist = settings.whitelisted.clone();
            continue;
        }

        if settings.url.is_empty() {
            continue;
        }
        if !url_changed && !settings.is_stale(now()) {
            continue;
        }
        if url_changed {
            tracing::info!(url = %settings.url, "the block list URL changed, fetching it");
        }

        match download(&settings).await {
            Ok(bytes) => {
                let cache = blocklist::Settings::cache_path(&core.config_dir);
                if let Err(err) = std::fs::write(&cache, &bytes) {
                    tracing::warn!(error = %err, "could not cache the block list");
                }
                install(&core, &settings, &bytes).await;
                installed_from = settings.url.clone();
                installed_whitelist = settings.whitelisted.clone();
            }
            Err(err) => tracing::warn!(error = %err, url = %settings.url,
                "could not download the block list"),
        }
    }
}

/// Fetches the list, retrying as the configuration says.
async fn download(settings: &blocklist::Settings) -> Result<Vec<u8>, blocklist::Error> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(settings.timeout))
        .build()
        .map_err(|err| blocklist::Error::Download(err.to_string()))?;

    let mut last = String::from("no attempt was made");
    for attempt in 1..=settings.try_times.max(1) {
        match client.get(&settings.url).send().await {
            Ok(response) if response.status().is_success() => {
                return response
                    .bytes()
                    .await
                    .map(|body| body.to_vec())
                    .map_err(|err| blocklist::Error::Download(err.to_string()));
            }
            Ok(response) => last = format!("the server answered {}", response.status()),
            Err(err) => last = err.to_string(),
        }
        tracing::warn!(attempt, error = %last, "block list download failed");
    }
    Err(blocklist::Error::Download(last))
}

/// Parses a list and installs it as the IP filter.
async fn install(core: &Core, settings: &blocklist::Settings, bytes: &[u8]) {
    let plain = match blocklist::decompress(bytes) {
        Ok(plain) => plain,
        Err(err) => {
            tracing::error!(error = %err, "could not unpack the block list");
            return;
        }
    };
    let text = match String::from_utf8(plain) {
        Ok(text) => text,
        // Latin-1 is what these lists are when they are not UTF-8, and only
        // the names are affected, so the ranges are still readable.
        Err(err) => err.as_bytes().iter().map(|&b| b as char).collect(),
    };

    let (format, import) = match blocklist::parse(&text) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::error!(error = %err, "could not read the block list");
            return;
        }
    };

    let count = import.ranges.len();
    let rules = blocklist::rules(&import.ranges, &settings.whitelisted);
    let outcome = core
        .manager
        .with(move |state| state.session.set_ip_filter(&rules))
        .await;

    match outcome {
        Ok(Ok(())) => {
            tracing::info!(
                format = format.as_str(),
                ranges = count,
                skipped = import.skipped,
                whitelisted = settings.whitelisted.len(),
                "installed the block list"
            );
            record_import(core, count as i64).await;
        }
        Ok(Err(err)) => tracing::error!(error = %err, "libtorrent refused the block list"),
        Err(err) => tracing::error!(error = %err, "the torrent manager is not answering"),
    }
}

/// Writes back what the import produced, so staleness survives a restart.
async fn record_import(core: &Core, ranges: i64) {
    let mut config = core.config.lock().await;
    let Some(Json::Object(mut stored)) = config.get("blocklist").cloned() else {
        return;
    };
    stored.insert("last_update".to_owned(), Json::from(now()));
    stored.insert("list_size".to_owned(), Json::from(ranges));
    if let Err(err) = config.set("blocklist", Json::Object(stored)) {
        tracing::warn!(error = %err, "could not record the block list import");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds one domain a run of health words and collects what got said.
    fn sweeps(words: &[&str]) -> Vec<Option<bool>> {
        let mut reported = TrackerNews::new();
        words
            .iter()
            .map(|word| tracker_news(&mut reported, "tracker.example.com", word))
            .collect()
    }

    #[test]
    fn a_tracker_has_to_stay_down_before_anybody_is_told() {
        // The first failing sweep says nothing: one dropped announce is not an
        // outage. The second is, and the sweeps after it are the same outage,
        // not another one.
        assert_eq!(
            sweeps(&["down", "down", "down", "down"]),
            vec![None, Some(true), None, None]
        );
    }

    #[test]
    fn a_tracker_that_answers_again_is_news_once() {
        assert_eq!(
            sweeps(&["down", "down", "ok", "ok", "ok"]),
            vec![None, Some(true), None, Some(false), None]
        );
    }

    #[test]
    fn one_failing_announce_among_working_ones_is_not_an_outage() {
        // Down is failing everywhere and working nowhere; warning is a tracker
        // that is answering somebody, and nobody needs waking for it.
        assert_eq!(sweeps(&["ok", "warning", "warning", "ok"]), vec![None; 4]);
    }

    #[test]
    fn a_tracker_that_flaps_is_not_a_message_a_minute() {
        // Alternating answers never hold for two sweeps, so nothing is sent at
        // all: a message every minute about a tracker that cannot make up its
        // mind is a message people stop reading.
        assert_eq!(sweeps(&["down", "ok", "down", "ok", "down"]), vec![None; 5]);
    }

    #[test]
    fn a_tracker_nothing_is_known_about_changes_nothing() {
        // Added and not yet announced, or every torrent on it paused. It is
        // not evidence that the tracker came back, and it does not interrupt
        // the run of sweeps that says it went away.
        assert_eq!(
            sweeps(&["down", "unknown", "down"]),
            vec![None, None, Some(true)]
        );
    }

    #[test]
    fn a_rate_limit_of_minus_one_becomes_libtorrents_zero() {
        // Passing -1 through would be a limit of minus one byte per second,
        // which libtorrent normalises to something nobody asked for.
        assert_eq!(kib_to_bytes(-1.0), 0);
        assert_eq!(kib_to_bytes(0.0), 0);
        assert_eq!(kib_to_bytes(50.0), 51_200);
    }

    #[test]
    fn the_next_hour_is_never_now() {
        // A zero wait would spin, and would apply the same state twice.
        let wait = seconds_to_the_next_hour();
        assert!((1..=3600).contains(&wait), "{wait}");
    }

    #[test]
    fn the_local_weekday_and_hour_are_in_range() {
        let (weekday, hour) = local_weekday_and_hour();
        assert!(weekday < scheduler::DAYS);
        assert!(hour < scheduler::HOURS);
    }

    #[test]
    fn a_delay_reads_as_words_in_the_history() {
        // The line has to say what the rule was, because after a removal the
        // rule is the only thing left explaining the gap in the list. Each
        // answer has to finish the sentence it is dropped into: "... have done
        // nothing {}, with their files".
        assert_eq!(describe_delay(0.0), "since they started");
        assert_eq!(describe_delay(1.0), "for an hour");
        assert_eq!(describe_delay(4.0), "for 4 hours");
        // A float that came back from JSON, rather than `4.000000000000001`.
        assert_eq!(describe_delay(0.5), "for 0.5 hours");
    }
}
