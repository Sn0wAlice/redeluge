// SPDX-License-Identifier: GPL-3.0-or-later
//! Watched directories: torrent files dropped in a folder get added.
//!
//! Deluge's AutoAdd plugin, as a daemon feature. Each watched directory has
//! its own options, which is the point of it: one folder for films that go to
//! one place with one label, another for something else.
//!
//! The one subtle part is that a file appearing in a directory is not the same
//! as a file being finished. Something is still writing it, and reading it too
//! early gets a truncated torrent. The plugin retried a fixed number of times;
//! this waits for the size to stop changing between scans, which is the same
//! idea without the arbitrary count.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::torrent::TorrentOptions;

/// What to do with the file once its torrent has been added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AfterAdd {
    /// Rename it out of the way, so the next scan skips it. The default,
    /// because it is the only one that cannot lose a file.
    #[default]
    Rename,
    /// Leave it alone. Only sensible with a copy directory set, or the same
    /// torrent is added on every scan.
    Leave,
    /// Delete it.
    Delete,
}

/// One watched directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchDir {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub path: String,
    /// Where the torrents land. Empty means the daemon's own download
    /// location.
    #[serde(default)]
    pub download_location: String,
    /// The label every torrent from this directory gets.
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub add_paused: bool,
    #[serde(default)]
    pub after_add: AfterAdd,
    /// The extension `AfterAdd::Rename` appends.
    #[serde(default = "added")]
    pub rename_extension: String,
    /// A directory the original file is copied into before anything else
    /// happens to it. Empty for none.
    #[serde(default)]
    pub copy_to: String,
}

fn yes() -> bool {
    true
}
fn added() -> String {
    ".added".to_owned()
}

impl WatchDir {
    /// The per-torrent options a file from this directory is added with.
    pub fn options(&self) -> TorrentOptions {
        TorrentOptions {
            save_path: if self.download_location.is_empty() {
                None
            } else {
                Some(self.download_location.clone())
            },
            label: crate::core::normalise_label(&self.label),
            paused: self.add_paused,
            ..TorrentOptions::default()
        }
    }
}

/// The watched directories, under the `autoadd` key of `core.conf`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    /// Seconds between scans.
    #[serde(default = "five")]
    pub interval: u64,
    #[serde(default)]
    pub watchdirs: Vec<WatchDir>,
}

fn five() -> u64 {
    5
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: 5,
            watchdirs: Vec::new(),
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => serde_json::from_value(value.clone()).unwrap_or_else(|err| {
                tracing::warn!(error = %err, "the autoadd configuration is malformed, ignoring it");
                Self::default()
            }),
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// The directories worth scanning.
    pub fn active(&self) -> impl Iterator<Item = &WatchDir> {
        let enabled = self.enabled;
        self.watchdirs
            .iter()
            .filter(move |dir| enabled && dir.enabled && !dir.path.is_empty())
    }
}

/// A torrent file seen in a watched directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub size: u64,
}

/// Whether a name is one this looks at.
///
/// Only `.torrent`. The plugin also read `.magnet` files, a list of magnet
/// links one per line; that is not here, and the TODO says so.
pub fn is_torrent_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("torrent"))
        .unwrap_or(false)
}

/// Every torrent file directly in a directory, with its size.
///
/// Not recursive, which is the plugin's behaviour: a watched directory is a
/// drop box, not a tree to crawl.
pub fn scan(directory: &Path) -> std::io::Result<Vec<Candidate>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if !is_torrent_file(&path) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        found.push(Candidate {
            path,
            size: metadata.len(),
        });
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// Remembers what each file looked like on the previous scan.
///
/// A file is only ready once it has been the same size twice running. Adding
/// it the moment it appears reads whatever part of it has been written, and a
/// truncated torrent fails to parse for reasons that look nothing like the
/// cause.
#[derive(Debug, Default)]
pub struct Settled {
    sizes: HashMap<PathBuf, u64>,
}

impl Settled {
    /// Files that have stopped changing since the last call.
    pub fn ready(&mut self, seen: &[Candidate]) -> Vec<PathBuf> {
        let mut ready = Vec::new();
        let mut next = HashMap::with_capacity(seen.len());

        for candidate in seen {
            let previous = self.sizes.get(&candidate.path).copied();
            if previous == Some(candidate.size) {
                ready.push(candidate.path.clone());
            }
            next.insert(candidate.path.clone(), candidate.size);
        }

        // Files that are gone drop out, so one that comes back later has to
        // settle again rather than being added from a stale size.
        self.sizes = next;
        ready
    }

    /// Forgets a file, after it has been added or has failed.
    pub fn forget(&mut self, path: &Path) {
        self.sizes.remove(path);
    }
}

/// What to do with the file after its torrent was added.
///
/// Split out from doing it so the decision is testable without a filesystem.
pub fn disposal(directory: &WatchDir, path: &Path) -> Disposal {
    let copy_to = if directory.copy_to.is_empty() {
        None
    } else {
        path.file_name()
            .map(|name| Path::new(&directory.copy_to).join(name))
    };

    let then = match directory.after_add {
        AfterAdd::Leave => Then::Leave,
        AfterAdd::Delete => Then::Delete,
        AfterAdd::Rename => {
            let extension = if directory.rename_extension.is_empty() {
                added()
            } else {
                directory.rename_extension.clone()
            };
            let mut name = path.as_os_str().to_owned();
            name.push(&extension);
            Then::RenameTo(PathBuf::from(name))
        }
    };

    Disposal { copy_to, then }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disposal {
    pub copy_to: Option<PathBuf>,
    pub then: Then,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Then {
    Leave,
    Delete,
    RenameTo(PathBuf),
}

/// Carries out a disposal.
pub fn dispose(disposal: &Disposal, path: &Path) -> std::io::Result<()> {
    if let Some(destination) = &disposal.copy_to {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(path, destination)?;
    }
    match &disposal.then {
        Then::Leave => Ok(()),
        Then::Delete => std::fs::remove_file(path),
        Then::RenameTo(destination) => std::fs::rename(path, destination),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn watchdir(path: &str) -> WatchDir {
        WatchDir {
            enabled: true,
            path: path.to_owned(),
            download_location: String::new(),
            label: String::new(),
            add_paused: false,
            after_add: AfterAdd::Rename,
            rename_extension: added(),
            copy_to: String::new(),
        }
    }

    fn candidate(path: &str, size: u64) -> Candidate {
        Candidate {
            path: PathBuf::from(path),
            size,
        }
    }

    // ------------------------------------------------------------ settings

    #[test]
    fn a_missing_configuration_watches_nothing() {
        let settings = Settings::from_config(None);
        assert!(!settings.enabled);
        assert_eq!(settings.active().count(), 0);
    }

    #[test]
    fn a_disabled_feature_watches_nothing_however_many_directories_it_has() {
        let settings = Settings {
            enabled: false,
            watchdirs: vec![watchdir("/watch")],
            ..Settings::default()
        };
        assert_eq!(settings.active().count(), 0);
    }

    #[test]
    fn a_disabled_directory_is_skipped_and_the_others_are_not() {
        let mut off = watchdir("/off");
        off.enabled = false;
        let settings = Settings {
            enabled: true,
            watchdirs: vec![off, watchdir("/on"), watchdir("")],
            ..Settings::default()
        };

        let active: Vec<&str> = settings.active().map(|dir| dir.path.as_str()).collect();
        assert_eq!(active, vec!["/on"], "an empty path is not a directory");
    }

    #[test]
    fn a_partial_directory_entry_keeps_the_defaults_for_what_it_omits() {
        let settings = Settings::from_config(Some(
            &json!({"enabled": true, "watchdirs": [{"path": "/w"}]}),
        ));
        let dir = &settings.watchdirs[0];
        assert!(dir.enabled, "a directory someone listed is on by default");
        assert_eq!(dir.after_add, AfterAdd::Rename);
        assert_eq!(dir.rename_extension, ".added");
    }

    #[test]
    fn a_malformed_value_is_ignored_rather_than_fatal() {
        let settings = Settings::from_config(Some(&json!(["/watch"])));
        assert!(!settings.enabled);
    }

    #[test]
    fn a_directorys_options_carry_its_label_and_location() {
        let mut dir = watchdir("/watch");
        dir.download_location = "/films".to_owned();
        dir.label = "Films".to_owned();
        dir.add_paused = true;

        let options = dir.options();
        assert_eq!(options.save_path.as_deref(), Some("/films"));
        assert_eq!(options.label, "films", "labels are lower case");
        assert!(options.paused);
    }

    #[test]
    fn a_directory_with_no_location_leaves_it_to_the_daemon() {
        assert_eq!(watchdir("/watch").options().save_path, None);
    }

    // ---------------------------------------------------------- recognising

    #[test]
    fn only_torrent_files_are_looked_at() {
        assert!(is_torrent_file(Path::new("/w/a.torrent")));
        assert!(is_torrent_file(Path::new("/w/a.TORRENT")));
        assert!(!is_torrent_file(Path::new("/w/a.torrent.added")));
        assert!(!is_torrent_file(Path::new("/w/a.part")));
        assert!(!is_torrent_file(Path::new("/w/torrent")));
    }

    // -------------------------------------------------------------- settling

    #[test]
    fn a_file_is_not_added_the_first_time_it_is_seen() {
        let mut settled = Settled::default();
        assert!(settled.ready(&[candidate("/w/a.torrent", 100)]).is_empty());
    }

    #[test]
    fn a_file_whose_size_has_stopped_changing_is_ready() {
        let mut settled = Settled::default();
        settled.ready(&[candidate("/w/a.torrent", 100)]);

        let ready = settled.ready(&[candidate("/w/a.torrent", 100)]);
        assert_eq!(ready, vec![PathBuf::from("/w/a.torrent")]);
    }

    #[test]
    fn a_file_still_being_written_is_not_ready() {
        // The whole reason this exists: adding a half-written torrent fails to
        // parse, and the error says nothing about the cause.
        let mut settled = Settled::default();
        settled.ready(&[candidate("/w/a.torrent", 100)]);
        assert!(settled.ready(&[candidate("/w/a.torrent", 900)]).is_empty());
        assert_eq!(
            settled.ready(&[candidate("/w/a.torrent", 900)]),
            vec![PathBuf::from("/w/a.torrent")]
        );
    }

    #[test]
    fn a_file_that_disappears_and_returns_settles_again() {
        let mut settled = Settled::default();
        settled.ready(&[candidate("/w/a.torrent", 100)]);
        settled.ready(&[]);
        assert!(
            settled.ready(&[candidate("/w/a.torrent", 100)]).is_empty(),
            "the old size is gone, so it starts over"
        );
    }

    #[test]
    fn forgetting_a_file_makes_it_settle_again() {
        let mut settled = Settled::default();
        settled.ready(&[candidate("/w/a.torrent", 100)]);
        settled.forget(Path::new("/w/a.torrent"));
        assert!(settled.ready(&[candidate("/w/a.torrent", 100)]).is_empty());
    }

    #[test]
    fn several_files_settle_independently() {
        let mut settled = Settled::default();
        settled.ready(&[candidate("/w/a.torrent", 10), candidate("/w/b.torrent", 20)]);

        let ready = settled.ready(&[candidate("/w/a.torrent", 10), candidate("/w/b.torrent", 99)]);
        assert_eq!(ready, vec![PathBuf::from("/w/a.torrent")]);
    }

    // -------------------------------------------------------------- disposal

    #[test]
    fn renaming_appends_the_extension_to_the_whole_name() {
        // Not replacing the extension: `a.torrent` becomes `a.torrent.added`,
        // so the original name is still readable and the scan skips it.
        let disposal = disposal(&watchdir("/w"), Path::new("/w/a.torrent"));
        assert_eq!(disposal.copy_to, None);
        assert_eq!(
            disposal.then,
            Then::RenameTo(PathBuf::from("/w/a.torrent.added"))
        );
    }

    #[test]
    fn an_empty_extension_falls_back_to_the_default() {
        let mut dir = watchdir("/w");
        dir.rename_extension = String::new();
        assert_eq!(
            disposal(&dir, Path::new("/w/a.torrent")).then,
            Then::RenameTo(PathBuf::from("/w/a.torrent.added"))
        );
    }

    #[test]
    fn leaving_the_file_alone_is_a_choice() {
        let mut dir = watchdir("/w");
        dir.after_add = AfterAdd::Leave;
        assert_eq!(disposal(&dir, Path::new("/w/a.torrent")).then, Then::Leave);
    }

    #[test]
    fn the_copy_happens_before_the_file_is_disposed_of() {
        let mut dir = watchdir("/w");
        dir.after_add = AfterAdd::Delete;
        dir.copy_to = "/keep".to_owned();

        let disposal = disposal(&dir, Path::new("/w/a.torrent"));
        assert_eq!(disposal.copy_to, Some(PathBuf::from("/keep/a.torrent")));
        assert_eq!(disposal.then, Then::Delete);
    }

    // ------------------------------------------------------ against the disk

    #[test]
    fn a_scan_finds_torrent_files_and_ignores_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.torrent"), b"bbbb").unwrap();
        std::fs::write(dir.path().join("a.torrent"), b"aa").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("a.torrent.added"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("sub.torrent")).unwrap();

        let found = scan(dir.path()).unwrap();
        let names: Vec<String> = found
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();

        assert_eq!(names, vec!["a.torrent", "b.torrent"], "sorted, files only");
        assert_eq!(found[0].size, 2);
        assert_eq!(found[1].size, 4);
    }

    #[test]
    fn scanning_a_directory_that_is_not_there_is_an_error_not_a_panic() {
        assert!(scan(Path::new("/no/such/directory/here")).is_err());
    }

    #[test]
    fn disposal_renames_the_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.torrent");
        std::fs::write(&path, b"torrent").unwrap();

        dispose(&disposal(&watchdir("/w"), &path), &path).unwrap();
        assert!(!path.exists());
        assert!(dir.path().join("a.torrent.added").is_file());
    }

    #[test]
    fn disposal_copies_before_deleting_and_creates_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.torrent");
        std::fs::write(&path, b"torrent").unwrap();

        let mut watched = watchdir("/w");
        watched.after_add = AfterAdd::Delete;
        watched.copy_to = dir.path().join("keep").display().to_string();

        dispose(&disposal(&watched, &path), &path).unwrap();
        assert!(!path.exists(), "deleted");
        assert_eq!(
            std::fs::read(dir.path().join("keep").join("a.torrent")).unwrap(),
            b"torrent"
        );
    }
}
