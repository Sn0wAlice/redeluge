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
    flags, AddTorrent, Alert, AlertKind, FlagChange, Session, SessionSettings,
    TorrentStatus as LtStatus,
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
    /// When each download was first seen transferring below the idle rule's
    /// rate. Here rather than in the task that maintains it, because the
    /// torrent status reports the countdown and has to read the same numbers
    /// the rule is acting on.
    pub idle_since: BTreeMap<String, f64>,
    /// Whether a filesystem the session writes to is under the disk-space
    /// rule's floor. Written by that rule, read by the idle one, which must
    /// not start a download it was holding while there is nowhere to put it.
    pub low_space: bool,
    /// Which torrents the session-wide pause actually stopped.
    ///
    /// Resuming the session must start those and only those. It used to resume
    /// everything, so a torrent paused by hand, by the share-ratio rule or by
    /// the idle rule was started again by the next session resume, and the
    /// scheduler performs one at startup: every pause was lost on restart.
    pub paused_by_session: std::collections::BTreeSet<String>,
    /// What the daemon has done on its own, for the interface to show.
    ///
    /// Shared with everything else that acts by itself: the rules on this
    /// thread and the ones on the async side write to the same history, or it
    /// would answer "why is this paused" for half the reasons.
    pub activity: std::sync::Arc<crate::activity::Log>,
    /// Which torrents already carry their tracker's limits, and which
    /// version of them.
    ///
    /// Keyed by torrent, holding the rule's fingerprint. It is how the limits
    /// are applied when a torrent is first seen under a rule and again when
    /// the rule changes, without being re-applied every minute: a standing
    /// rule that overwrote a limit somebody set by hand, every minute, would
    /// be unusable. Not saved: after a restart every torrent is seen for the
    /// first time again, and re-applying a rule that has not changed writes
    /// the same numbers.
    pub tracker_limits: BTreeMap<String, String>,
    /// What each peer has done, across connections and restarts.
    ///
    /// Written by the sampler on the async side and saved from this thread on
    /// the way out, which is the only moment the connection counters have to
    /// be forgotten.
    pub peers: std::sync::Arc<std::sync::Mutex<crate::peers::Ledger>>,
    /// Where peer countries come from, when the operator provided a database.
    countries: Option<crate::geoip::CountryLookup>,
    config_dir: PathBuf,
    /// Set when something changed that the state file does not yet reflect.
    dirty: bool,
}

impl SessionState {
    /// Where per-torrent files live: state, resume data, and `.torrent` copies.
    pub fn state_dir(&self) -> PathBuf {
        self.config_dir.join("state")
    }

    /// Loads the country database named by the configuration, if there is one.
    pub fn set_country_lookup(&mut self, lookup: Option<crate::geoip::CountryLookup>) {
        self.countries = lookup;
    }

    /// The country of a peer address, when a database is loaded.
    pub fn country_of(&self, address: &str) -> Option<String> {
        self.countries
            .as_ref()
            .and_then(|lookup| lookup.country_of_text(address))
    }

    /// The country's name, for the tooltip beside the flag.
    pub fn country_name_of(&self, address: &str) -> Option<String> {
        self.countries
            .as_ref()
            .and_then(|lookup| lookup.name_of_text(address))
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Writes the peer ledger out.
    ///
    /// The per-connection counters go first: they describe connections that
    /// will not exist next time, and a stored counter that the next sample
    /// cannot beat would swallow that peer's next few gibibytes.
    pub fn save_peers(&mut self) {
        let Ok(mut ledger) = self.peers.lock() else {
            return;
        };
        ledger.forget_connections();
        if let Err(err) = ledger.save(&self.config_dir) {
            tracing::warn!(error = %err, "could not write the peer ledger");
        }
    }

    /// Drops everything on disk that belongs to one torrent.
    ///
    /// Not the downloaded data: the stored `.torrent` and its resume file.
    /// Without this a removed torrent comes back at the next restart.
    pub fn forget(&mut self, id: &str) {
        let state_dir = self.state_dir();
        let _ = std::fs::remove_file(torrent_file_path(&state_dir, id));
        let _ = std::fs::remove_file(state_dir.join("resume").join(format!("{id}.resume")));
        self.resume_data.remove(id);
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

/// Unix seconds, for the history of what the rules on this thread did.
fn now_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or(0.0)
}

/// A job for the session thread.
type Job = Box<dyn FnOnce(&mut SessionState) + Send>;

/// A handle onto the session thread.
#[derive(Clone)]
pub struct Manager {
    jobs: mpsc::Sender<Job>,
    events: tokio::sync::broadcast::Sender<Event>,
    activity: std::sync::Arc<crate::activity::Log>,
    peers: std::sync::Arc<std::sync::Mutex<crate::peers::Ledger>>,
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
        let activity = std::sync::Arc::new(crate::activity::Log::default());
        // Read before the session runs, because the first sample has to know
        // what was already there or it would start everybody at zero.
        let peers = std::sync::Arc::new(std::sync::Mutex::new(crate::peers::Ledger::load(
            &config_dir,
        )));

        let state = SessionState {
            session,
            torrents: BTreeMap::new(),
            session_paused: false,
            external_ip: None,
            counters: Vec::new(),
            resume_data: BTreeMap::new(),
            idle_since: BTreeMap::new(),
            low_space: false,
            paused_by_session: std::collections::BTreeSet::new(),
            tracker_limits: BTreeMap::new(),
            activity: std::sync::Arc::clone(&activity),
            peers: std::sync::Arc::clone(&peers),
            countries: None,
            config_dir,
            dirty: false,
        };

        let manager = Self {
            jobs,
            events: events.clone(),
            activity,
            peers,
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

    /// What the daemon has done on its own.
    pub fn activity(&self) -> &std::sync::Arc<crate::activity::Log> {
        &self.activity
    }

    /// What each peer has done.
    pub fn peers(&self) -> &std::sync::Arc<std::sync::Mutex<crate::peers::Ledger>> {
        &self.peers
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

/// How long the session thread waits for a job before going round again.
///
/// This is what a caller waits for, and it used to be the alert timeout below:
/// every call that touched the session queued behind a hundred-millisecond
/// sleep, so `core.get_external_ip`, which reads one string out of memory,
/// answered in about a tenth of a second. A poll from the Web UI is six such
/// calls in a row on a connection that handles one at a time, which is where
/// half a second of lag on every filter click came from.
const JOB_WAIT: Duration = Duration::from_millis(10);

/// How often every torrent's state is compared with the last pass.
///
/// On a timer rather than every time round the loop. The sweep asks libtorrent
/// for the status of every torrent, which is the most expensive thing this
/// thread does and grows with the library; doing it a hundred times a second
/// now that the loop is quick would be absurd, and doing it before answering
/// each call is what made the calls slow.
const SWEEP_EVERY: Duration = Duration::from_millis(250);

/// The session thread: run jobs, drain alerts, save on a timer.
fn run(
    mut state: SessionState,
    inbox: mpsc::Receiver<Job>,
    events: tokio::sync::broadcast::Sender<Event>,
) {
    let mut last_save = Instant::now();
    let mut last_resume_save = Instant::now();
    let mut last_ratio_check = Instant::now();
    let mut states: BTreeMap<String, TorrentState> = BTreeMap::new();

    let emit = |event: Event| {
        let _ = events.send(event);
    };

    // Far enough in the past that the first pass sweeps rather than waiting a
    // quarter of a second to notice what was restored.
    let mut last_sweep = Instant::now() - SWEEP_EVERY;

    loop {
        // Jobs first, and waited for rather than polled: this is the whole of
        // what a caller's latency is, so the thread sleeps here and nowhere
        // else.
        match inbox.recv_timeout(JOB_WAIT) {
            Ok(job) => {
                job(&mut state);
                // Then whatever else is already waiting, before alerts or the
                // sweep. A poll from the Web UI arrives as several calls in a
                // row and they should cost one pass, not one each.
                loop {
                    match inbox.try_recv() {
                        Ok(job) => job(&mut state),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            stop(&mut state);
                            return;
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stop(&mut state);
                return;
            }
        }

        // Never blocks, so it costs nothing on a pass with no alerts, and the
        // loop now comes round often enough that alerts are handled sooner
        // than they were with a hundred-millisecond wait.
        for alert in state.session.pop_alerts() {
            handle_alert(&mut state, &alert, &emit);
        }

        // State changes are noticed here rather than pushed from each
        // operation, because libtorrent changes state on its own too: a torrent
        // finishes, the queue starts one, a tracker fails.
        if last_sweep.elapsed() >= SWEEP_EVERY {
            last_sweep = Instant::now();
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
        }

        // Seeding rules, checked on a timer rather than on an alert: a ratio
        // creeps past its limit while nothing happens, so there is no event to
        // hang this on.
        if last_ratio_check.elapsed() > Duration::from_secs(5) {
            last_ratio_check = Instant::now();
            enforce_seeding_rules(&mut state, &emit);
        }

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

/// Saves everything on the way out. Both ends of the job channel lead here.
fn stop(state: &mut SessionState) {
    tracing::info!("torrent manager stopping");
    let _ = state.save_state();
    let _ = state.save_resume_data();
    state.save_peers();
}

/// Pauses or removes torrents that have reached their share ratio.
///
/// Deluge's own rule, and its two surprises. A torrent is only considered once
/// it has finished, so a ratio reached while still downloading does not stop
/// it. And removing is checked before pausing, because `remove_at_ratio`
/// without `stop_at_ratio` means nothing in Deluge: the remove is what the
/// stop turns into.
fn enforce_seeding_rules<F: Fn(Event)>(state: &mut SessionState, emit: &F) {
    if state.session_paused {
        return;
    }

    let mut pause = Vec::new();
    let mut remove = Vec::new();

    for status in state.session.all_torrent_status() {
        let Some(torrent) = state.torrents.get(&status.info_hash) else {
            continue;
        };
        if !torrent.options.stop_at_ratio || !torrent.options.is_finished {
            continue;
        }
        let ratio = torrent.ratio(&status);
        if ratio < 0.0 || ratio < torrent.options.stop_ratio {
            continue;
        }

        if torrent.options.remove_at_ratio {
            remove.push(status.info_hash.clone());
        } else if !status.is_paused {
            pause.push(status.info_hash.clone());
        }
    }

    for id in pause {
        // Clearing auto-manage, not just pausing. libtorrent's queue resumes
        // an auto-managed torrent it finds paused, within about half a minute,
        // so a bare `pause()` here did nothing at all on the torrents that had
        // reached their ratio: they stopped and started again. Auto-management
        // is on by default, so that was most of them.
        let change = FlagChange::new()
            .set_to(flags::PAUSED, true)
            .set_to(flags::AUTO_MANAGED, false);
        match state.session.set_flags(&id, change) {
            Ok(()) => {
                // Recorded on the torrent as well, because a restart re-adds
                // every torrent from its stored options: a stop kept only in
                // the session came back running.
                let mut name = String::new();
                if let Some(torrent) = state.torrents.get_mut(&id) {
                    torrent.options.paused = true;
                    torrent.options.auto_managed = false;
                    name = torrent.options.name.clone().unwrap_or_default();
                }
                state.dirty = true;
                tracing::info!(torrent = %id, "stopped at its share ratio");
                state.activity.record(
                    crate::activity::Action::new(
                        crate::activity::rule::RATIO,
                        crate::activity::did::PAUSED,
                        now_seconds(),
                    )
                    .torrent(&id, &name)
                    .detail("it reached its share ratio"),
                );
            }
            Err(err) => tracing::warn!(torrent = %id, error = %err,
                "could not stop a torrent at its share ratio"),
        }
    }

    for id in remove {
        emit(Event::PreTorrentRemoved {
            torrent_id: id.clone(),
        });
        // The data stays. Deluge removes the torrent, not the download, and a
        // rule that deleted files on a timer would be a bad surprise.
        let name = state
            .torrents
            .get(&id)
            .and_then(|torrent| torrent.options.name.clone())
            .unwrap_or_default();
        match state.session.remove_torrent(&id, false) {
            Ok(()) => {
                state.torrents.remove(&id);
                state.forget(&id);
                state.dirty = true;
                tracing::info!(torrent = %id, "removed at its share ratio");
                state.activity.record(
                    crate::activity::Action::new(
                        crate::activity::rule::RATIO,
                        crate::activity::did::REMOVED,
                        now_seconds(),
                    )
                    .torrent(&id, &name)
                    .detail("it reached its share ratio; the files were kept"),
                );
                emit(Event::TorrentRemoved { torrent_id: id });
            }
            Err(err) => tracing::warn!(torrent = %id, error = %err,
                "could not remove a torrent at its share ratio"),
        }
    }
}

/// Moves a finished torrent to its completion directory, if it has one.
///
/// The move is asynchronous: libtorrent answers with a `storage_moved` alert,
/// which is where `save_path` is updated. Recording the destination in
/// `moving_to` is what makes the status say "Moving" in the meantime.
fn move_on_completion(state: &mut SessionState, id: &str) {
    let Some(torrent) = state.torrents.get(id) else {
        return;
    };
    if !torrent.options.move_completed {
        return;
    }
    let Some(destination) = torrent.options.move_completed_path.clone() else {
        return;
    };
    if destination.is_empty() || torrent.options.save_path.as_deref() == Some(destination.as_str())
    {
        return;
    }

    match state.session.move_storage(id, &destination) {
        Ok(()) => {
            tracing::info!(torrent = %id, destination, "moving a finished torrent");
            if let Some(torrent) = state.torrents.get_mut(id) {
                torrent.moving_to = Some(destination);
            }
        }
        Err(err) => {
            tracing::warn!(torrent = %id, error = %err, "could not move a finished torrent");
            if let Some(torrent) = state.torrents.get_mut(id) {
                torrent.forced_error = Some(format!("could not move the files: {err}"));
            }
        }
    }
}

/// Turns a libtorrent alert into daemon state and daemon events.
fn handle_alert<F: Fn(Event)>(state: &mut SessionState, alert: &Alert, emit: &F) {
    let id = alert.info_hash.clone().unwrap_or_default();

    match alert.kind {
        AlertKind::TorrentFinished => {
            // libtorrent posts this on the transition, but also after a
            // recheck of a torrent that was already complete. Moving the files
            // every time that happened would move them out from under
            // themselves on each restart, so the move is only for a torrent
            // that was not already finished.
            let was_finished = state
                .torrents
                .get(&id)
                .map(|torrent| torrent.options.is_finished)
                .unwrap_or(true);

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

            if !was_finished {
                move_on_completion(state, &id);
            }
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
