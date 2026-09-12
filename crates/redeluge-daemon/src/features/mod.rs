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
pub mod idlepause;
pub mod label;
pub mod scheduler;
pub mod webhook;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redeluge_libtorrent::{AddTorrent, Setting};
use serde_json::Value as Json;

use crate::core::Core;

/// Starts the timers. Called once, after the daemon is up.
pub fn spawn(core: Arc<Core>) {
    tokio::spawn(watch_directories(Arc::clone(&core)));
    tokio::spawn(follow_schedule(Arc::clone(&core)));
    tokio::spawn(rotate_idle_downloads(Arc::clone(&core)));
    tokio::spawn(guard_disk_space(Arc::clone(&core)));
    tokio::spawn(announce_torrents(Arc::clone(&core)));
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
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static REPORTED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let reported = REPORTED.get_or_init(Default::default);

    let Ok(mut reported) = reported.lock() else {
        return;
    };
    if reported.get(feature).map(String::as_str) == Some(error) {
        return;
    }
    reported.insert(feature.to_owned(), error.to_owned());
    tracing::warn!(
        feature,
        error,
        "the configuration is malformed, ignoring it"
    );
}

/// Seconds since the Unix epoch, as the configuration stores them.
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or(0.0)
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
                for id in released {
                    tracing::info!(torrent = %id, "released by the idle rule");
                }
                for id in paused {
                    tracing::info!(torrent = %id, "paused: idle while something was queued");
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
) -> (Vec<String>, Vec<String>) {
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
            if let Some(torrent) = state.torrents.get_mut(&id) {
                torrent.options.idle_resume_at = 0.0;
                // The options, not just the session flags: a restart re-adds
                // every torrent from these.
                torrent.options.paused = false;
                torrent.options.auto_managed = true;
            }
            state.mark_dirty();
            state.idle_since.remove(&id);
            released.push(id);
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
            if let Some(torrent) = state.torrents.get_mut(id) {
                torrent.options.idle_resume_at = now + settings.pause_for as f64;
                torrent.options.paused = true;
                torrent.options.auto_managed = false;
            }
            state.mark_dirty();
            paused.push(id.clone());
            running -= 1;
            queued -= 1;
        }
    }

    for id in &paused {
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
                for id in released {
                    tracing::info!(torrent = %id, "released: there is room again");
                }
                for id in paused {
                    tracing::warn!(torrent = %id, "paused: not enough free space to keep writing");
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
) -> (Vec<String>, Vec<String>) {
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
            if let Some(torrent) = state.torrents.get_mut(&id) {
                torrent.options.space_paused = false;
                torrent.options.paused = false;
                torrent.options.auto_managed = managed;
            }
            state.mark_dirty();
            released.push(id);
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
            if let Some(torrent) = state.torrents.get_mut(&id) {
                torrent.options.space_was_managed = torrent.options.auto_managed;
                torrent.options.space_paused = true;
                torrent.options.paused = true;
                torrent.options.auto_managed = false;
            }
            state.mark_dirty();
            state.idle_since.remove(&id);
            paused.push(id);
        }
    }

    (released, paused)
}

// ----------------------------------------------------------- notifications

/// Posts a message somewhere when a torrent finishes, arrives or breaks.
///
/// Driven by the same event stream every client subscribes to, so there is one
/// idea of what "finished" means rather than a second one written for this.
async fn announce_torrents(core: Arc<Core>) {
    use crate::events::Event;
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
            let torrent = state.torrents.get(&id)?;

            if trigger == webhook::Trigger::Finished {
                let stale = status.completed_time > 0 && now - status.completed_time as f64 > FRESH;
                if stale || torrent.options.announced_finished {
                    return None;
                }
            }

            let trackers = state.session.trackers(&id).unwrap_or_default();
            let current = crate::torrent::current_tracker(&status.current_tracker, &trackers);

            let notice = webhook::Notice {
                torrent_id: id.clone(),
                name: torrent.display_name(&status),
                size: status.total_wanted,
                save_path: status.save_path.clone(),
                label: torrent.options.label.clone(),
                tracker: crate::torrent::tracker_host(&current),
                ratio: torrent.ratio(&status),
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
}
