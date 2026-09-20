// SPDX-License-Identifier: GPL-3.0-or-later
//! Writing state files off the session thread.
//!
//! The torrent list and the resume blobs used to be written by the thread that
//! owns the libtorrent session — the same thread that answers every RPC call.
//! A write of a few megabytes, or one file per torrent, is not slow in itself,
//! but while it happens nothing else on that thread happens either: every
//! client waiting on a status waits on the disk. On a spinning disk, an NFS
//! mount or a busy SD card, that is where the seconds come from.
//!
//! So the session thread serialises — it has to, the data is its own — and
//! hands the bytes here. One thread does the writing, in the order it was
//! given them, which is what keeps two saves of the same file from racing:
//! both write the same temporary path before renaming it into place.
//!
//! [`StateWriter::flush`] waits for the queue to drain. The daemon calls it on
//! the way out, because the alternative is a clean shutdown that loses the
//! state it just decided to save.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// One file to write.
pub struct Write {
    /// Where it belongs once it is written.
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    /// Whether the file already there should be kept as `.bak`.
    pub keep_backup: bool,
}

enum Job {
    Write(Write),
    /// Removed in the order it was asked for, which is the point: a file
    /// queued for writing and then deleted must not come back afterwards.
    Delete(PathBuf),
    /// Answered once everything queued before it has been written.
    Flush(mpsc::SyncSender<()>),
}

/// A handle onto the thread that writes state files.
#[derive(Debug)]
pub struct StateWriter {
    jobs: mpsc::Sender<Job>,
}

impl StateWriter {
    pub fn start() -> Self {
        let (jobs, inbox) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("state-writer".to_owned())
            .spawn(move || run(&inbox))
            .expect("the state writer thread starts");
        Self { jobs }
    }

    /// Queues one file. Returns whether the writer is still there.
    pub fn write(&self, job: Write) -> bool {
        self.jobs.send(Job::Write(job)).is_ok()
    }

    /// Queues a removal, behind whatever is already waiting.
    pub fn delete(&self, path: PathBuf) -> bool {
        self.jobs.send(Job::Delete(path)).is_ok()
    }

    /// Waits until everything queued so far has been written.
    ///
    /// Called on the way out, and by the tests, which would otherwise be
    /// asking the filesystem about a write that has not happened yet.
    pub fn flush(&self) {
        let (done, wait) = mpsc::sync_channel::<()>(0);
        if self.jobs.send(Job::Flush(done)).is_ok() {
            // The writer answers when it gets here; if it has gone away the
            // channel closes and this returns just the same.
            let _ = wait.recv();
        }
    }
}

fn run(inbox: &mpsc::Receiver<Job>) {
    while let Ok(job) = inbox.recv() {
        match job {
            Job::Write(job) => {
                if let Err(err) = write_file(&job) {
                    tracing::warn!(path = %job.path.display(), error = %err,
                        "could not write a state file");
                }
            }
            Job::Delete(path) => {
                let _ = std::fs::remove_file(&path);
            }
            Job::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

/// Writes one file the way state files have to be written: beside the target,
/// then renamed over it, so a kill in the middle leaves the previous one.
fn write_file(job: &Write) -> std::io::Result<()> {
    if let Some(parent) = job.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temporary_path(&job.path);
    std::fs::write(&temporary, &job.bytes)?;
    if job.keep_backup && job.path.exists() {
        let _ = std::fs::rename(&job.path, job.path.with_extension("bak"));
    }
    std::fs::rename(&temporary, &job.path)
}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension("tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_arrives_and_the_one_before_it_is_kept() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("state").join("torrents.json");
        let writer = StateWriter::start();

        writer.write(Write {
            path: path.clone(),
            bytes: b"first".to_vec(),
            keep_backup: true,
        });
        writer.write(Write {
            path: path.clone(),
            bytes: b"second".to_vec(),
            keep_backup: true,
        });
        writer.flush();

        assert_eq!(std::fs::read(&path).expect("the file"), b"second");
        assert_eq!(
            std::fs::read(path.with_extension("bak")).expect("the backup"),
            b"first",
            "the previous version was not kept"
        );
        assert!(
            !temporary_path(&path).exists(),
            "the temporary file was left behind"
        );
    }

    #[test]
    fn writes_happen_in_the_order_they_were_given() {
        // Two saves of the same file in quick succession is the ordinary case:
        // the timer fires while the previous write is still queued. The last
        // one has to be the one on disk.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("torrents.json");
        let writer = StateWriter::start();

        for round in 0..20u8 {
            writer.write(Write {
                path: path.clone(),
                bytes: vec![round],
                keep_backup: false,
            });
        }
        writer.flush();

        assert_eq!(std::fs::read(&path).expect("the file"), vec![19u8]);
    }

    #[test]
    fn a_file_deleted_after_it_was_queued_does_not_come_back() {
        // What happens when a torrent is removed a moment after its resume
        // data was queued. The write is still in the queue; if the removal
        // jumped it, the file would be written after the delete and the
        // torrent would leave litter behind for ever.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("abc.resume");
        let writer = StateWriter::start();

        writer.write(Write {
            path: path.clone(),
            bytes: b"resume".to_vec(),
            keep_backup: false,
        });
        writer.delete(path.clone());
        writer.flush();

        assert!(!path.exists(), "the file came back after being removed");
    }

    #[test]
    fn flushing_a_writer_with_nothing_to_do_returns() {
        let writer = StateWriter::start();
        writer.flush();
    }
}
