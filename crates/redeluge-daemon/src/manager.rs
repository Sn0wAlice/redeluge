// SPDX-License-Identifier: GPL-3.0-or-later
//! The torrent manager.
//!
//! # Why a thread rather than a lock
//!
//! The libtorrent session is `Send` but not `Sync`, and its alert loop parks on
//! a blocking call. Putting it behind a mutex would mean either holding that
//! mutex across the blocking wait, which stalls every caller, or waking
//! constantly, which burns a core. So the session lives on one thread that owns
//! it outright, and everything else sends it work.
//!
//! Work is a closure rather than a command enum. Fifty-odd operations would
//! otherwise be fifty-odd variants, each with its own reply type, for no gain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use redeluge_libtorrent::{
    AddTorrent, Alert, AlertKind, Session, SessionSettings, TorrentStatus as LtStatus,
};
use tokio::sync::oneshot;

use crate::events::Event;
use crate::state::TorrentState;
use crate::torrent::{Torrent, TorrentOptions};

/// What the session thread owns.
pub struct SessionState {
    pub session: Session,
    pub torrents: BTreeMap<String, Torrent>,
    /// Set by `core.pause_session`, which pauses everything at once.
    pub session_paused: bool,
    /// The address libtorrent last observed for us.
    pub external_ip: Option<String>,
    /// The last session counters, from `session_stats_alert`.
    pub counters: Vec<i64>,
    /// Resume data waiting to be written, by infohash.
    pub resume_data: BTreeMap<String, Vec<u8>>,
    config_dir: PathBuf,
    /// Set when something changed that the state file does not yet reflect.
    dirty: bool,
}

impl SessionState {
    /// Where per-torrent files live: state, resume data, and `.torrent` copies.
    pub fn state_dir(&self) -> PathBuf {
        self.config_dir.join("state")
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// The status of one torrent, or None when it is gone.
    pub fn status_of(&self, id: &str) -> Option<LtStatus> {
        self.session.torrent_status(id).ok()
    }

    /// Writes the torrent list. JSON, not pickle: see the wiki, Migration Phase 3.
    pub fn save_state(&mut self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let dir = self.state_dir();
        std::fs::create_dir_all(&dir)?;

        let options: Vec<&TorrentOptions> = self
            .torrents
            .values()
            .map(|torrent| &torrent.options)
            .collect();
        let body = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "torrents": options,
        }))?;

        let path = dir.join("torrents.json");
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, &body)?;
        if path.exists() {
            let _ = std::fs::rename(&path, path.with_extension("bak"));
        }
        std::fs::rename(&temporary, &path)?;

        self.dirty = false;
        Ok(())
    }

    /// Writes every resume blob that has arrived since the last write.
    ///
    /// One file per torrent rather than one file for all of them: a bad write
    /// then costs one torrent's resume data instead of the whole session's.
    pub fn save_resume_data(&mut self) -> std::io::Result<usize> {
        if self.resume_data.is_empty() {
            return Ok(0);
        }
        let dir = self.state_dir().join("resume");
        std::fs::create_dir_all(&dir)?;

        let pending = std::mem::take(&mut self.resume_data);
        let count = pending.len();
        for (id, blob) in pending {
            let path = dir.join(format!("{id}.resume"));
            let temporary = path.with_extension("tmp");
            std::fs::write(&temporary, &blob)?;
            std::fs::rename(&temporary, &path)?;
        }
        Ok(count)
    }

    /// Reads a torrent's resume data, when there is any.
    pub fn load_resume_data(&self, id: &str) -> Option<Vec<u8>> {
        std::fs::read(self.state_dir().join("resume").join(format!("{id}.resume"))).ok()
    }

    /// Reads the torrent list written by a previous run.
    pub fn load_state(config_dir: &Path) -> Vec<TorrentOptions> {
        let path = config_dir.join("state").join("torrents.json");

        for candidate in [path.clone(), path.with_extension("bak")] {
            let Ok(text) = std::fs::read_to_string(&candidate) else {
                continue;
            };
            match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(value) => {
                    let torrents = value
                        .get("torrents")
                        .and_then(|v| v.as_array())
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| {
                                    serde_json::from_value::<TorrentOptions>(item.clone()).ok()
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    tracing::info!(count = torrents.len(), path = %candidate.display(),
                        "loaded the torrent state");
                    return torrents;
                }
                Err(err) => {
                    tracing::warn!(path = %candidate.display(), error = %err,
                        "unreadable torrent state, trying the backup");
                }
            }
        }

        // An old install has a pickle here, which this daemon cannot read. Say
        // so loudly rather than starting with an empty list and looking like
        // every torrent was lost.
        if config_dir.join("state").join("torrents.state").exists() {
            tracing::error!(
                "found torrents.state from the Python daemon. Convert it first: \
                 python3 tools/migrate_state.py <config dir>"
            );
        }
        Vec::new()
    }
}

/// A job for the session thread.
type Job = Box<dyn FnOnce(&mut SessionState) + Send>;

/// A handle onto the session thread.
#[derive(Clone)]
pub struct Manager {
    jobs: mpsc::Sender<Job>,
    events: tokio::sync::broadcast::Sender<Event>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the torrent manager has stopped")]
    Stopped,
    #[error("{0}")]
    Libtorrent(#[from] redeluge_libtorrent::Error),
    #[error("no such torrent: {0}")]
    UnknownTorrent(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Manager {
    /// Starts the session thread.
    pub fn start(
        config_dir: PathBuf,
        settings: SessionSettings,
        events: tokio::sync::broadcast::Sender<Event>,
    ) -> Result<Self> {
        let session = Session::new(&settings)?;
        let (jobs, inbox) = mpsc::channel::<Job>();

        let state = SessionState {
            session,
            torrents: BTreeMap::new(),
            session_paused: false,
            external_ip: None,
            counters: Vec::new(),
            resume_data: BTreeMap::new(),
            config_dir,
            dirty: false,
        };

        let manager = Self {
            jobs,
            events: events.clone(),
        };

        std::thread::Builder::new()
            .name("libtorrent".to_owned())
            .spawn(move || run(state, inbox, events))
            .map_err(|err| Error::Other(err.to_string()))?;

        Ok(manager)
    }

    /// Runs a closure on the session thread and waits for its answer.
    pub async fn with<T, F>(&self, job: F) -> Result<T>
    where
        F: FnOnce(&mut SessionState) -> T + Send + 'static,
        T: Send + 'static,
    {
        let (reply, answer) = oneshot::channel();
        self.jobs
            .send(Box::new(move |state| {
                let _ = reply.send(job(state));
            }))
            .map_err(|_| Error::Stopped)?;
        answer.await.map_err(|_| Error::Stopped)
    }

    /// Runs a closure without waiting, for work whose result nobody reads.
    pub fn spawn<F>(&self, job: F) -> Result<()>
    where
        F: FnOnce(&mut SessionState) + Send + 'static,
    {
        self.jobs.send(Box::new(job)).map_err(|_| Error::Stopped)
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

/// The session thread: run jobs, drain alerts, save on a timer.
fn run(
    mut state: SessionState,
    inbox: mpsc::Receiver<Job>,
    events: tokio::sync::broadcast::Sender<Event>,
) {
    let mut last_save = Instant::now();
    let mut last_resume_save = Instant::now();
    let mut states: BTreeMap<String, TorrentState> = BTreeMap::new();

    let emit = |event: Event| {
        let _ = events.send(event);
    };

    loop {
        // Jobs first: a caller waiting on a reply should not wait behind a
        // hundred-millisecond alert poll.
        loop {
            match inbox.try_recv() {
                Ok(job) => job(&mut state),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::info!("torrent manager stopping");
                    let _ = state.save_state();
                    let _ = state.save_resume_data();
                    return;
                }
            }
        }

        state.session.wait_for_alert(Duration::from_millis(100));
        for alert in state.session.pop_alerts() {
            handle_alert(&mut state, &alert, &emit);
        }

        // State changes are noticed here rather than pushed from each
        // operation, because libtorrent changes state on its own too: a torrent
        // finishes, the queue starts one, a tracker fails.
        let session_paused = state.session_paused;
        for status in state.session.all_torrent_status() {
            let Some(torrent) = state.torrents.get(&status.info_hash) else {
                continue;
            };
            let now = torrent.state(&status, session_paused);
            let before = states.insert(status.info_hash.clone(), now);
            if before != Some(now) {
                emit(Event::TorrentStateChanged {
                    torrent_id: status.info_hash.clone(),
                    state: now.to_string(),
                });
            }
        }
        states.retain(|id, _| state.torrents.contains_key(id));

        if last_resume_save.elapsed() > Duration::from_secs(10) {
            last_resume_save = Instant::now();
            match state.save_resume_data() {
                Ok(count) if count > 0 => {
                    tracing::debug!(count, "wrote resume data")
                }
                Err(err) => tracing::warn!(error = %err, "could not write resume data"),
                _ => {}
            }
        }

        if last_save.elapsed() > Duration::from_secs(60) {
            last_save = Instant::now();
            if let Err(err) = state.save_state() {
                tracing::warn!(error = %err, "could not write the torrent state");
            }
        }
    }
}

/// Turns a libtorrent alert into daemon state and daemon events.
fn handle_alert<F: Fn(Event)>(state: &mut SessionState, alert: &Alert, emit: &F) {
    let id = alert.info_hash.clone().unwrap_or_default();

    match alert.kind {
        AlertKind::TorrentFinished => {
            if let Some(torrent) = state.torrents.get_mut(&id) {
                torrent.options.is_finished = true;
                state.dirty = true;
            }
            emit(Event::TorrentFinished {
                torrent_id: id.clone(),
            });
            // Resume data is worth having the moment a torrent completes, not
            // on the next timer tick.
            let _ = state.session.save_resume_data(&id, false);
        }

        AlertKind::TorrentPaused => {
            // The state sweep reports the change; nothing extra to do here.
        }
        AlertKind::TorrentResumed => emit(Event::TorrentResumed { torrent_id: id }),

        AlertKind::SaveResumeData => {
            if let Some(blob) = alert.resume_data() {
                state.resume_data.insert(id, blob.to_vec());
            }
        }
        AlertKind::SaveResumeDataFailed => {
            tracing::debug!(torrent = %id, reason = ?alert.error(),
                "libtorrent could not build resume data");
        }

        AlertKind::StorageMoved => {
            if let Some(path) = alert.path() {
                if let Some(torrent) = state.torrents.get_mut(&id) {
                    torrent.moving_to = None;
                    torrent.options.save_path = Some(path.to_owned());
                    state.dirty = true;
                }
                emit(Event::TorrentStorageMoved {
                    torrent_id: id,
                    path: path.to_owned(),
                });
            }
        }
        AlertKind::StorageMovedFailed => {
            if let Some(torrent) = state.torrents.get_mut(&id) {
                torrent.moving_to = None;
                torrent.forced_error = Some(
                    alert
                        .error()
                        .unwrap_or("could not move the torrent's files")
                        .to_owned(),
                );
            }
        }

        AlertKind::FileRenamed => {
            if let (Some(index), Some(name)) = (alert.file_index(), alert.path()) {
                emit(Event::TorrentFileRenamed {
                    torrent_id: id,
                    index,
                    name: name.to_owned(),
                });
            }
        }
        AlertKind::FileCompleted => {
            if let Some(index) = alert.file_index() {
                emit(Event::TorrentFileCompleted {
                    torrent_id: id,
                    index,
                });
            }
        }
        AlertKind::FileError => {
            if let Some(torrent) = state.torrents.get_mut(&id) {
                torrent.forced_error = Some(alert.error().unwrap_or("file error").to_owned());
            }
        }

        AlertKind::TrackerReply => {
            if let Some(torrent) = state.torrents.get_mut(&id) {
                let peers = alert.num_peers().unwrap_or(0);
                torrent.tracker_status = format!("Announce OK ({peers} peers)");
            }
            emit(Event::TorrentTrackerStatus {
                torrent_id: id,
                status: "Announce OK".to_owned(),
            });
        }
        AlertKind::TrackerAnnounce => {
            if let Some(torrent) = state.torrents.get_mut(&id) {
                torrent.tracker_status = "Announce Sent".to_owned();
            }
        }
        AlertKind::TrackerWarning | AlertKind::TrackerError => {
            let message = alert
                .error()
                .map(str::to_owned)
                .unwrap_or_else(|| alert.message.clone());
            if let Some(torrent) = state.torrents.get_mut(&id) {
                torrent.tracker_status = format!("Error: {message}");
            }
            emit(Event::TorrentTrackerStatus {
                torrent_id: id,
                status: format!("Error: {message}"),
            });
        }

        AlertKind::ExternalIp => {
            if let Some(ip) = alert.external_ip() {
                state.external_ip = Some(ip.to_owned());
                emit(Event::ExternalIp {
                    external_ip: ip.to_owned(),
                });
            }
        }
        AlertKind::SessionStats => {
            if let Some(counters) = alert.counters() {
                state.counters = counters.to_vec();
            }
        }

        AlertKind::FastresumeRejected => {
            // The files are not where the resume data said. libtorrent rechecks
            // on its own; saying so in the log is what the operator needs.
            tracing::warn!(torrent = %id, message = %alert.message,
                "resume data rejected, rechecking");
        }

        AlertKind::MetadataReceived => {
            // A magnet becomes a real torrent here. Writing the file now is
            // what lets it restart without re-fetching its metadata.
            let state_dir = state.state_dir();
            if let Ok(bytes) = state.session.torrent_file(&id) {
                let path = torrent_file_path(&state_dir, &id);
                if let Err(err) =
                    std::fs::create_dir_all(&state_dir).and_then(|()| std::fs::write(&path, &bytes))
                {
                    tracing::warn!(torrent = %id, error = %err,
                        "could not store the torrent file");
                }
            }
            state.dirty = true;
            let _ = state.session.save_resume_data(&id, false);
        }

        AlertKind::Unknown => {
            tracing::trace!(what = %alert.what, message = %alert.message, "unhandled alert");
        }

        _ => {}
    }
}

/// Where a torrent's `.torrent` file is kept.
///
/// Named by infohash, not by the name it was uploaded under: two torrents can
/// arrive as `download.torrent` and one would overwrite the other. The
/// `filename` field keeps the original name for display, which is what Deluge
/// uses it for too.
pub fn torrent_file_path(state_dir: &Path, id: &str) -> PathBuf {
    state_dir.join(format!("{id}.torrent"))
}

/// Builds the libtorrent request for a torrent the daemon is restoring.
pub fn restore_request(options: &TorrentOptions, state_dir: &Path) -> Option<AddTorrent> {
    let stored = torrent_file_path(state_dir, &options.torrent_id);

    let mut request = if stored.is_file() {
        AddTorrent::from_file(std::fs::read(stored).ok()?, String::new())
    } else if let Some(magnet) = &options.magnet {
        AddTorrent::from_magnet(magnet.clone(), String::new())
    } else {
        // No metadata and no magnet: nothing to add it from. A torrent whose
        // file went missing is better reported than silently forgotten.
        return None;
    };

    request.save_path = options.save_path.clone().unwrap_or_default();
    request.file_priorities = options.file_priorities.clone();
    request.pre_allocate = options.storage_mode == "allocate";
    request.flags = request
        .flags
        .set_to(redeluge_libtorrent::flags::PAUSED, options.paused)
        .set_to(
            redeluge_libtorrent::flags::AUTO_MANAGED,
            options.auto_managed,
        )
        .set_to(
            redeluge_libtorrent::flags::SEQUENTIAL_DOWNLOAD,
            options.sequential_download,
        );
    Some(request)
}

impl Manager {
    /// Emits an event without going through the session thread.
    pub fn announce(&self, event: Event) {
        self.emit(event);
    }
}
