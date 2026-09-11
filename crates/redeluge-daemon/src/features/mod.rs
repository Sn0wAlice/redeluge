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
//! Labels are the exception to the file layout: they are a torrent option and
//! a status field, so they live in `torrent.rs` and `core.rs` with the other
//! options rather than here.
//!
//! Each module keeps its decisions pure and testable. This file is where those
//! decisions meet a running session, on a timer.

pub mod autoadd;
pub mod blocklist;
pub mod scheduler;

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
    tokio::spawn(maintain_blocklist(core));
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

            for path in settled.ready(&found) {
                settled.forget(&path);
                let dump = match std::fs::read(&path) {
                    Ok(dump) => dump,
                    Err(err) => {
                        tracing::warn!(path = %path.display(), error = %err,
                            "could not read a torrent file");
                        continue;
                    }
                };

                let mut options = directory.options();
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
                        let disposal = autoadd::disposal(directory, &path);
                        if let Err(err) = autoadd::dispose(&disposal, &path) {
                            // The torrent is added either way. Saying so
                            // matters because the file will be picked up again
                            // on the next scan and refused as a duplicate.
                            tracing::warn!(path = %path.display(), error = %err,
                                "could not dispose of a torrent file after adding it");
                        }
                    }
                    Err(err) => tracing::warn!(path = %path.display(), error = %err.message,
                        "could not add a torrent from a watched directory"),
                }
            }
        }
    }
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

        tokio::time::sleep(Duration::from_secs(seconds_to_the_next_hour())).await;
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

    loop {
        let settings = blocklist::Settings::from_config(setting(&core, "blocklist").await.as_ref());

        if !settings.enabled {
            // Turning it off has to put the filter back, or the last list
            // stays in force until a restart.
            let outcome = core
                .manager
                .with(|state| state.session.clear_ip_filter())
                .await;
            if let Ok(Err(err)) = outcome {
                tracing::warn!(error = %err, "could not clear the IP filter");
            }
        } else if !settings.url.is_empty() && settings.is_stale(now()) {
            match download(&settings).await {
                Ok(bytes) => {
                    let cache = blocklist::Settings::cache_path(&core.config_dir);
                    if let Err(err) = std::fs::write(&cache, &bytes) {
                        tracing::warn!(error = %err, "could not cache the block list");
                    }
                    install(&core, &settings, &bytes).await;
                }
                Err(err) => tracing::warn!(error = %err, url = %settings.url,
                    "could not download the block list"),
            }
        }

        tokio::time::sleep(Duration::from_secs(3600)).await;
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
