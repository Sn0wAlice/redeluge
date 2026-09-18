// SPDX-License-Identifier: GPL-3.0-or-later
//! Building a `.torrent`, and watching it happen.
//!
//! Hashing is the whole cost: every byte of the content is read, which for a
//! large directory is minutes. That one fact decides the shape of everything
//! here.
//!
//! `core.create_torrent` is Deluge's method and answers when the file is
//! finished, because that is what its callers expect. It is only usable from a
//! client with a connection to spare: the daemon serves one call at a time per
//! connection, so a caller that blocks on it blocks its own connection and
//! nothing else. The Web UI has exactly one connection and cannot afford that,
//! which is why `redeluge.create_torrent` exists beside it: it starts a job,
//! answers with its id, and the caller watches `CreateTorrentProgressEvent` or
//! asks `redeluge.get_create_torrent` how far along it is.
//!
//! Finished torrents are kept here rather than written somewhere and forgotten,
//! because the browser has to be able to fetch one back. They are kept for ten
//! minutes and no more than eight at a time: a `.torrent` for a large
//! collection is a few megabytes, and a daemon is not a file server.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use redeluge_libtorrent::{CreateTorrent, HashProgress, Session, TorrentFormat};
use redeluge_rencode::Value;

use crate::events::Event;
use crate::manager::Manager;

/// How long a finished job is kept before its bytes are dropped.
///
/// Long enough for a browser to be handed the id and fetch the file, short
/// enough that a forgotten one does not hold memory for the life of the daemon.
const KEEP_FOR: Duration = Duration::from_secs(600);

/// How many jobs are remembered at once, finished or not.
const KEEP_AT_MOST: usize = 8;

/// The smallest and largest piece a torrent may use.
///
/// libtorrent's own bounds. Below the floor the piece list dwarfs the content;
/// above the ceiling a client that has not seen one that large may refuse it.
const MIN_PIECE: i64 = 16 * 1024;
const MAX_PIECE: i64 = 64 * 1024 * 1024;

/// One request to build a torrent, however it was phrased.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub build: CreateTorrent,
    /// Where to write the finished file on the daemon's own disk, if anywhere.
    pub target: Option<String>,
    /// Whether to add it and start seeding the content it was built from.
    pub add_to_session: bool,
}

impl Request {
    /// Deluge's positional form, as `core.create_torrent` takes it.
    ///
    /// The order is the contract's and is not negotiable:
    /// `(path, tracker, piece_length, comment, target, webseeds, private,
    /// created_by, trackers, add_to_session, torrent_format, ca_cert)`.
    ///
    /// Argument one is the *primary* tracker and is a string, not a list. This
    /// read it as a list until now, so a client passing the announce URL Deluge
    /// documents got a torrent with no tracker at all and no error to say so.
    pub fn from_positional(args: &[Value]) -> Result<Self, String> {
        let path = args
            .first()
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| "a path is required".to_owned())?
            .to_owned();

        let mut trackers: Vec<String> = Vec::new();
        if let Some(primary) = args.get(1).and_then(Value::as_str) {
            if !primary.is_empty() {
                trackers.push(primary.to_owned());
            }
        }
        // And the rest, which Deluge passes separately. A URL already named as
        // the primary one is not added twice: duplicate announces in one tier
        // are announced to twice.
        for extra in strings(args.get(8)) {
            if !trackers.contains(&extra) {
                trackers.push(extra);
            }
        }

        Ok(Self {
            build: CreateTorrent {
                path,
                piece_length: piece_length(args.get(2))?,
                comment: text(args.get(3)),
                creator: creator(args.get(7)),
                private: args.get(6).and_then(Value::as_bool).unwrap_or(false),
                trackers,
                web_seeds: strings(args.get(5)),
                format: format(args.get(10)),
            },
            target: path_option(args.get(4)),
            add_to_session: args.get(9).and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// This fork's own form: one dictionary, named keys.
    ///
    /// Positional arguments are how a method acquires twelve of them. The Web
    /// UI sends this one, and a key it does not know about is ignored rather
    /// than shifting everything after it.
    pub fn from_options(value: Option<&Value>) -> Result<Self, String> {
        let options = value.unwrap_or(&Value::None);
        let path = options
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| "a path is required".to_owned())?
            .to_owned();

        Ok(Self {
            build: CreateTorrent {
                path,
                piece_length: piece_length(options.get("piece_length"))?,
                comment: text(options.get("comment")),
                creator: creator(options.get("created_by")),
                private: options
                    .get("private")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                trackers: strings(options.get("trackers")),
                web_seeds: strings(options.get("webseeds")),
                format: format(options.get("torrent_format")),
            },
            target: path_option(options.get("target")),
            add_to_session: options
                .get("add_to_session")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// The name the torrent will carry, which is the content's own.
    ///
    /// Read from the path rather than from the finished file: the daemon has no
    /// bencode decoder, deliberately, and this is the same answer libtorrent
    /// arrives at.
    pub fn name(&self) -> String {
        let trimmed = self.build.path.trim_end_matches(['/', '\\']);
        match trimmed.rsplit(['/', '\\']).next() {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => trimmed.to_owned(),
        }
    }

    /// Where the content sits, which is where a seeding copy has to be told to
    /// look for it.
    pub fn parent(&self) -> String {
        let trimmed = self.build.path.trim_end_matches(['/', '\\']);
        match trimmed.rfind(['/', '\\']) {
            // Directly under the root: the parent is the separator itself.
            Some(0) => trimmed[..1].to_owned(),
            Some(at) => trimmed[..at].to_owned(),
            None => ".".to_owned(),
        }
    }
}

/// Whether a piece is one of the hundred that are worth an event.
///
/// A torrent can have hundreds of thousands of pieces, and one event each would
/// drown every connected client to move a progress bar. The step is rounded up
/// so that the hundred holds at the bottom of the range as well: a hundred and
/// ninety-nine divided by a hundred is one, which is an event per piece. The
/// last piece always reports, so a watcher always sees the run end.
fn worth_reporting(piece: i64, total: i64) -> bool {
    piece == total || piece % ((total + 99) / 100).max(1) == 0
}

/// Hashes the content and returns the finished `.torrent`.
///
/// Blocks for as long as the content takes to read, so it belongs on a thread
/// that is allowed to block. Progress is announced as it goes, and handed to
/// `watching` in the same breath so that a job can record it without a second
/// subscription to its own events.
pub fn build(
    request: &CreateTorrent,
    manager: &Manager,
    watching: impl Fn(i64, i64) + Send + 'static,
) -> Result<Vec<u8>, String> {
    let manager = manager.clone();
    let mut last_reported = -1i64;
    let mut progress = HashProgress::new(Box::new(move |piece, total| {
        let total = i64::from(total).max(1);
        // libtorrent counts from zero and a progress bar counts from one.
        let piece = i64::from(piece) + 1;
        if !worth_reporting(piece, total) || piece == last_reported {
            return;
        }
        last_reported = piece;
        watching(piece, total);
        manager.announce(Event::CreateTorrentProgress {
            piece_count: piece,
            num_pieces: total,
        });
    }));

    Session::create_torrent_with_progress(request, &mut progress).map_err(|err| err.to_string())
}

// ---------------------------------------------------------------------- jobs

/// What a job is doing, or what came of it.
#[derive(Debug, Clone)]
pub enum State {
    Running { piece: i64, pieces: i64 },
    Done(Outcome),
    Failed(String),
}

/// A finished torrent, and what was done with it.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub name: String,
    pub info_hash: String,
    /// The bencoded file, kept so the browser can be handed it.
    pub torrent_file: Vec<u8>,
    /// Where it was written on the daemon's disk, if it was asked for.
    pub written_to: Option<String>,
    /// The id it was added under, if it was added.
    pub torrent_id: Option<String>,
}

#[derive(Debug)]
struct Job {
    name: String,
    /// When it last changed, which is what expiry is measured from.
    touched: Instant,
    state: State,
}

/// Every job this daemon is running or has recently run.
#[derive(Debug, Default)]
pub struct Jobs {
    inner: Mutex<HashMap<String, Job>>,
}

impl Jobs {
    /// Registers a new job and returns its id.
    ///
    /// The id is not a secret: it is only ever handed back to the client that
    /// asked, over an authenticated connection. It is random so that two jobs
    /// started in the same second cannot collide.
    pub fn start(&self, name: &str) -> String {
        let id = new_id();
        let mut jobs = self.lock();
        reap(&mut jobs);
        jobs.insert(
            id.clone(),
            Job {
                name: name.to_owned(),
                touched: Instant::now(),
                state: State::Running {
                    piece: 0,
                    pieces: 0,
                },
            },
        );
        id
    }

    pub fn note_progress(&self, id: &str, piece: i64, pieces: i64) {
        let mut jobs = self.lock();
        if let Some(job) = jobs.get_mut(id) {
            job.touched = Instant::now();
            job.state = State::Running { piece, pieces };
        }
    }

    pub fn finish(&self, id: &str, state: State) {
        let mut jobs = self.lock();
        if let Some(job) = jobs.get_mut(id) {
            job.touched = Instant::now();
            job.state = state;
        }
    }

    /// What a client is told about a job.
    ///
    /// The file itself is not in here: it is bytes, and the Web UI's bridge
    /// renders anything that is not text lossily. It is fetched on its own.
    pub fn status(&self, id: &str) -> Option<Value> {
        let jobs = self.lock();
        let job = jobs.get(id)?;

        let mut fields = vec![
            (Value::Str("id".into()), Value::Str(id.to_owned())),
            (Value::Str("name".into()), Value::Str(job.name.clone())),
        ];
        match &job.state {
            State::Running { piece, pieces } => {
                fields.push((Value::Str("state".into()), Value::Str("running".into())));
                fields.push((
                    Value::Str("progress".into()),
                    Value::Float64(if *pieces > 0 {
                        *piece as f64 / *pieces as f64
                    } else {
                        0.0
                    }),
                ));
                fields.push((Value::Str("piece".into()), Value::Int(*piece)));
                fields.push((Value::Str("pieces".into()), Value::Int(*pieces)));
            }
            State::Done(outcome) => {
                fields.push((Value::Str("state".into()), Value::Str("done".into())));
                fields.push((Value::Str("progress".into()), Value::Float64(1.0)));
                fields.push((
                    Value::Str("info_hash".into()),
                    Value::Str(outcome.info_hash.clone()),
                ));
                fields.push((
                    Value::Str("size".into()),
                    Value::Int(outcome.torrent_file.len() as i64),
                ));
                fields.push((
                    Value::Str("written_to".into()),
                    match &outcome.written_to {
                        Some(path) => Value::Str(path.clone()),
                        None => Value::None,
                    },
                ));
                fields.push((
                    Value::Str("torrent_id".into()),
                    match &outcome.torrent_id {
                        Some(id) => Value::Str(id.clone()),
                        None => Value::None,
                    },
                ));
            }
            State::Failed(error) => {
                fields.push((Value::Str("state".into()), Value::Str("failed".into())));
                fields.push((Value::Str("progress".into()), Value::Float64(0.0)));
                fields.push((Value::Str("error".into()), Value::Str(error.clone())));
            }
        }
        Some(Value::Dict(fields))
    }

    /// The finished file, for the one caller that is going to serve it.
    pub fn file(&self, id: &str) -> Option<(String, Vec<u8>)> {
        let jobs = self.lock();
        match &jobs.get(id)?.state {
            State::Done(outcome) => Some((outcome.name.clone(), outcome.torrent_file.clone())),
            _ => None,
        }
    }

    /// A lock that a panicking job cannot take down with it.
    ///
    /// Nothing under this lock can panic on its own, but a poisoned registry
    /// would turn one failed build into a daemon that can never build again.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Job>> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner())
    }
}

/// Drops what has expired, then the oldest if there are still too many.
fn reap(jobs: &mut HashMap<String, Job>) {
    let now = Instant::now();
    jobs.retain(|_, job| {
        matches!(job.state, State::Running { .. }) || now.duration_since(job.touched) < KEEP_FOR
    });

    while jobs.len() >= KEEP_AT_MOST {
        // The oldest that is not still running: a job in flight is the one
        // thing that must not be forgotten while its task is holding its id.
        let oldest = jobs
            .iter()
            .filter(|(_, job)| !matches!(job.state, State::Running { .. }))
            .min_by_key(|(_, job)| job.touched)
            .map(|(id, _)| id.clone());
        match oldest {
            Some(id) => {
                jobs.remove(&id);
            }
            None => break,
        }
    }
}

/// Sixteen hex characters from the system's randomness.
fn new_id() -> String {
    let mut bytes = [0u8; 8];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .expect("the system random source");
    hex::encode(bytes)
}

// ------------------------------------------------------------------ arguments

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// A path argument, with Deluge's empty string meaning the same as absent.
fn path_option(value: Option<&Value>) -> Option<String> {
    let path = text(value);
    (!path.is_empty()).then_some(path)
}

/// Who made it. This fork's own name and version when the caller says nothing.
fn creator(value: Option<&Value>) -> String {
    let given = text(value);
    if given.is_empty() {
        concat!("redeluge ", env!("CARGO_PKG_VERSION")).to_owned()
    } else {
        given
    }
}

/// A list of strings, skipping anything blank.
///
/// A trackers box in an interface is one URL per line, and a trailing newline
/// becoming an empty announce URL is how a torrent acquires a tracker that
/// every client then tries and fails to reach.
fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_list)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn format(value: Option<&Value>) -> TorrentFormat {
    match value.and_then(Value::as_str) {
        Some(name) => TorrentFormat::from_name(name),
        None => TorrentFormat::default(),
    }
}

/// Validates a piece length rather than clamping one.
///
/// It has to be a power of two, which libtorrent enforces by throwing rather
/// than by rounding, so a clamp would let `100000` through to come back as an
/// exception from C++ with nothing to say which argument was wrong. Absent or
/// zero means libtorrent chooses from the total size, which is the right answer
/// for a caller with no opinion.
fn piece_length(value: Option<&Value>) -> Result<i32, String> {
    let raw = value.and_then(Value::as_i64).unwrap_or(0);
    if raw == 0 {
        return Ok(0);
    }
    if !(MIN_PIECE..=MAX_PIECE).contains(&raw) {
        return Err(format!(
            "a piece length of {raw} is outside {MIN_PIECE}..{MAX_PIECE}"
        ));
    }
    if raw & (raw - 1) != 0 {
        return Err(format!("a piece length of {raw} is not a power of two"));
    }
    Ok(raw as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn str_list(items: &[&str]) -> Value {
        Value::List(
            items
                .iter()
                .map(|item| Value::Str((*item).into()))
                .collect(),
        )
    }

    #[test]
    fn the_primary_tracker_is_a_string_and_comes_first() {
        // Deluge's own signature. Read as a list until now, which meant a
        // client passing the documented announce URL got no tracker at all.
        let request = Request::from_positional(&[
            Value::Str("/downloads/Thing".into()),
            Value::Str("http://primary/announce".into()),
            Value::Int(0),
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            str_list(&["http://backup/announce"]),
        ])
        .expect("a request");

        assert_eq!(
            request.build.trackers,
            vec![
                "http://primary/announce".to_owned(),
                "http://backup/announce".to_owned()
            ]
        );
    }

    #[test]
    fn a_tracker_named_twice_is_announced_to_once() {
        let request = Request::from_positional(&[
            Value::Str("/downloads/Thing".into()),
            Value::Str("http://one/announce".into()),
            Value::Int(0),
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            Value::None,
            str_list(&["http://one/announce", "http://two/announce"]),
        ])
        .expect("a request");

        assert_eq!(request.build.trackers.len(), 2);
    }

    #[test]
    fn the_rest_of_the_positional_arguments_land_where_deluge_puts_them() {
        let request = Request::from_positional(&[
            Value::Str("/downloads/Thing".into()),
            Value::Str("http://primary/announce".into()),
            Value::Int(32 * 1024),
            Value::Str("a comment".into()),
            Value::Str("/config/out.torrent".into()),
            str_list(&["http://seed/"]),
            Value::Bool(true),
            Value::Str("someone".into()),
            Value::None,
            Value::Bool(true),
            Value::Str("hybrid".into()),
        ])
        .expect("a request");

        assert_eq!(request.build.piece_length, 32 * 1024);
        assert_eq!(request.build.comment, "a comment");
        assert_eq!(request.target.as_deref(), Some("/config/out.torrent"));
        assert_eq!(request.build.web_seeds, vec!["http://seed/".to_owned()]);
        assert!(request.build.private);
        assert_eq!(request.build.creator, "someone");
        assert!(request.add_to_session);
        assert_eq!(request.build.format, TorrentFormat::Hybrid);
    }

    #[test]
    fn a_torrent_is_v1_unless_something_else_is_asked_for() {
        // libtorrent 2.0 writes a hybrid torrent when it is not told
        // otherwise, and a private tracker that only knows v1 refuses one.
        let request =
            Request::from_positional(&[Value::Str("/downloads/Thing".into())]).expect("a request");
        assert_eq!(request.build.format, TorrentFormat::V1);
        assert_eq!(TorrentFormat::from_name("nonsense"), TorrentFormat::V1);
    }

    #[test]
    fn a_path_is_required() {
        assert!(Request::from_positional(&[]).is_err());
        assert!(Request::from_positional(&[Value::Str(String::new())]).is_err());
        assert!(Request::from_options(None).is_err());
    }

    #[test]
    fn a_piece_length_that_is_not_a_power_of_two_is_refused_by_name() {
        let outcome = Request::from_positional(&[
            Value::Str("/downloads/Thing".into()),
            Value::None,
            Value::Int(100_000),
        ]);
        let message = outcome.expect_err("refused");
        assert!(message.contains("power of two"), "{message}");

        for good in [0, 16 * 1024, 64 * 1024 * 1024] {
            assert!(piece_length(Some(&Value::Int(good))).is_ok(), "{good}");
        }
        assert!(piece_length(Some(&Value::Int(8 * 1024))).is_err());
        assert!(piece_length(Some(&Value::Int(128 * 1024 * 1024))).is_err());
    }

    #[test]
    fn blank_trackers_do_not_become_announce_urls() {
        // A trackers box is one URL per line, and a trailing newline is how a
        // torrent acquires a tracker every client then fails to reach.
        let request = Request::from_options(Some(&Value::Dict(vec![
            (Value::Str("path".into()), Value::Str("/d/Thing".into())),
            (
                Value::Str("trackers".into()),
                str_list(&["http://one/announce", "", "  ", "http://two/announce"]),
            ),
        ])))
        .expect("a request");

        assert_eq!(request.build.trackers.len(), 2);
    }

    #[test]
    fn the_name_and_the_parent_come_off_the_path() {
        let cases = [
            ("/downloads/Thing", "Thing", "/downloads"),
            ("/downloads/Thing/", "Thing", "/downloads"),
            // Directly under the root: the parent is the separator, not "."
            // and not nothing, which is the mistake the C++ used to make.
            ("/downloads", "downloads", "/"),
            ("Thing", "Thing", "."),
        ];
        for (path, name, parent) in cases {
            let request = Request {
                build: CreateTorrent {
                    path: path.to_owned(),
                    ..CreateTorrent::default()
                },
                ..Request::default()
            };
            assert_eq!(request.name(), name, "name of {path}");
            assert_eq!(request.parent(), parent, "parent of {path}");
        }
    }

    #[test]
    fn progress_is_a_hundred_events_however_many_pieces_there_are() {
        // The point of the throttle. Before it was rounded up, anything under
        // two hundred pieces reported every single one.
        for total in [1i64, 7, 100, 199, 999, 250_000] {
            let reported: Vec<i64> = (1..=total)
                .filter(|piece| worth_reporting(*piece, total))
                .collect();

            assert!(
                reported.len() <= 101,
                "{total} pieces would send {} events",
                reported.len()
            );
            assert_eq!(
                reported.last().copied(),
                Some(total),
                "{total} pieces: the last piece has to report, or the bar stops short"
            );
        }
    }

    #[test]
    fn a_finished_job_reports_where_its_file_went() {
        let jobs = Jobs::default();
        let id = jobs.start("Thing");

        jobs.note_progress(&id, 5, 10);
        let running = jobs.status(&id).expect("a status");
        assert_eq!(
            running.get("state").and_then(Value::as_str),
            Some("running")
        );

        jobs.finish(
            &id,
            State::Done(Outcome {
                name: "Thing".into(),
                info_hash: "a".repeat(40),
                torrent_file: vec![1, 2, 3],
                written_to: Some("/config/out.torrent".into()),
                torrent_id: Some("b".repeat(40)),
            }),
        );

        let done = jobs.status(&id).expect("a status");
        assert_eq!(done.get("state").and_then(Value::as_str), Some("done"));
        assert_eq!(done.get("size").and_then(Value::as_i64), Some(3));
        assert_eq!(
            done.get("written_to").and_then(Value::as_str),
            Some("/config/out.torrent")
        );
        assert_eq!(jobs.file(&id).expect("the file").1, vec![1, 2, 3]);
    }

    #[test]
    fn a_job_that_failed_says_why_and_has_no_file() {
        let jobs = Jobs::default();
        let id = jobs.start("Thing");
        jobs.finish(&id, State::Failed("no files found".into()));

        let status = jobs.status(&id).expect("a status");
        assert_eq!(status.get("state").and_then(Value::as_str), Some("failed"));
        assert_eq!(
            status.get("error").and_then(Value::as_str),
            Some("no files found")
        );
        assert!(jobs.file(&id).is_none());
        assert!(jobs.status("nothing like an id").is_none());
    }

    #[test]
    fn the_registry_does_not_grow_without_bound() {
        // A `.torrent` for a large collection is megabytes. Finished jobs are
        // dropped oldest first; one still hashing is never dropped, because its
        // task is holding the id it would report back to.
        let jobs = Jobs::default();
        let running = jobs.start("still going");

        let mut ids = Vec::new();
        for index in 0..KEEP_AT_MOST * 2 {
            let id = jobs.start(&format!("job {index}"));
            jobs.finish(&id, State::Failed("done with".into()));
            ids.push(id);
        }

        assert!(jobs.lock().len() <= KEEP_AT_MOST);
        assert!(jobs.status(&running).is_some(), "a running job was dropped");
        assert!(
            jobs.status(ids.last().expect("an id")).is_some(),
            "the newest job was dropped"
        );
        assert!(jobs.status(&ids[0]).is_none(), "the oldest job was kept");
    }
}
