// SPDX-License-Identifier: GPL-3.0-or-later
//! The session: owning libtorrent, adding torrents, draining alerts.

use std::time::Duration;

use cxx::UniquePtr;

use crate::alert::Alert;
use crate::bridge::ffi;
use crate::bridge::ffi::IpRange;
use crate::settings::Setting;
use crate::torrent::{AddTorrent, FileEntry, FlagChange, PeerInfo, TorrentStatus, TrackerEntry};
use crate::Error;

/// Session settings the spike needs.
///
/// The daemon's full settings pack, all 77 keys of it, arrives in phase 3. What
/// is here is what a session cannot start without.
#[derive(Debug, Clone)]
pub struct SessionSettings {
    /// libtorrent `listen_interfaces` syntax, e.g. `0.0.0.0:6881`.
    pub listen_interfaces: String,
    pub user_agent: String,
    pub enable_dht: bool,
    pub enable_lsd: bool,
    pub enable_upnp: bool,
    pub enable_natpmp: bool,
    /// Alerts libtorrent buffers before it starts dropping them.
    pub alert_queue_size: i32,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            listen_interfaces: "0.0.0.0:6881,[::]:6881".to_owned(),
            user_agent: concat!("redeluge/", env!("CARGO_PKG_VERSION")).to_owned(),
            enable_dht: true,
            enable_lsd: true,
            enable_upnp: true,
            enable_natpmp: true,
            // Matches the Python daemon, which raised it from libtorrent's
            // default precisely because alerts were being lost.
            alert_queue_size: 10_000,
        }
    }
}

impl SessionSettings {
    /// Settings for a session that must not touch the network: no peer port, no
    /// discovery, no port mapping. Used by the tests.
    pub fn offline() -> Self {
        Self {
            listen_interfaces: "127.0.0.1:0".to_owned(),
            enable_dht: false,
            enable_lsd: false,
            enable_upnp: false,
            enable_natpmp: false,
            ..Self::default()
        }
    }

    fn to_ffi(&self) -> ffi::SessionConfig {
        ffi::SessionConfig {
            listen_interfaces: self.listen_interfaces.clone(),
            user_agent: self.user_agent.clone(),
            enable_dht: self.enable_dht,
            enable_lsd: self.enable_lsd,
            enable_upnp: self.enable_upnp,
            enable_natpmp: self.enable_natpmp,
            alert_queue_size: self.alert_queue_size,
        }
    }
}

/// A running libtorrent session.
pub struct Session {
    inner: UniquePtr<ffi::Session>,
}

// libtorrent's session is internally synchronised, so moving one to another
// thread is sound. It is deliberately not Sync: the daemon will poll alerts on
// a dedicated thread and reach the session through a lock, the same shape the
// Python AlertManager uses today.
unsafe impl Send for Session {}

impl Session {
    pub fn new(settings: &SessionSettings) -> Result<Self, Error> {
        ffi::new_session(&settings.to_ffi())
            .map(|inner| Self { inner })
            .map_err(|err| Error::SessionStart(err.what().to_owned()))
    }

    /// Adds a magnet URI and returns its hex v1 infohash.
    ///
    /// Returns as soon as libtorrent has accepted the torrent. Metadata arrives
    /// later, as an [`crate::AlertKind::MetadataReceived`] alert.
    pub fn add_magnet(&mut self, uri: &str, save_path: &str) -> Result<String, Error> {
        self.inner
            .pin_mut()
            .add_magnet(uri, save_path)
            .map_err(Error::from_cxx)
    }

    /// Adds a torrent from previously saved resume data.
    ///
    /// This is the restart path. The resume data names the torrent and its
    /// trackers, so no magnet URI is needed. Pass an empty `save_path` to keep
    /// the location recorded in the resume data, or a path to relocate it.
    pub fn add_torrent_from_resume(
        &mut self,
        resume_data: &[u8],
        save_path: &str,
    ) -> Result<String, Error> {
        self.inner
            .pin_mut()
            .add_torrent_from_resume(resume_data, save_path)
            .map_err(Error::from_cxx)
    }

    /// Asks libtorrent to build resume data for a torrent.
    ///
    /// Asynchronous: the bytes arrive later on a
    /// [`crate::AlertKind::SaveResumeData`] alert, readable with
    /// [`crate::Alert::resume_data`]. Set `flush_disk_cache` only when shutting
    /// down; it is expensive and pointless otherwise.
    pub fn save_resume_data(
        &mut self,
        info_hash: &str,
        flush_disk_cache: bool,
    ) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .save_resume_data(info_hash, flush_disk_cache)
            .map_err(Error::from_cxx)
    }

    /// Whether anything changed since this torrent's last resume data.
    ///
    /// A periodic save should consult this first, or a large session spends its
    /// time rewriting files nothing has touched.
    pub fn needs_resume_save(&self, info_hash: &str) -> Result<bool, Error> {
        self.inner
            .needs_resume_save(info_hash)
            .map_err(Error::from_cxx)
    }

    /// Blocks until at least one alert is queued, or the timeout expires.
    ///
    /// Returns whether an alert is waiting. This is the call the daemon's alert
    /// thread parks on.
    pub fn wait_for_alert(&mut self, timeout: Duration) -> bool {
        let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
        self.inner.pin_mut().wait_for_alert(millis)
    }

    /// Drains the alert queue. Never blocks.
    pub fn pop_alerts(&mut self) -> Vec<Alert> {
        self.inner
            .pin_mut()
            .pop_alerts()
            .into_iter()
            .map(Alert::from_flat)
            .collect()
    }

    pub fn torrent_status(&self, info_hash: &str) -> Result<TorrentStatus, Error> {
        self.inner
            .torrent_status(info_hash)
            .map(TorrentStatus::from)
            .map_err(Error::from_cxx)
    }

    /// Hex infohashes of every torrent in the session.
    pub fn torrent_hashes(&self) -> Vec<String> {
        self.inner.torrent_hashes()
    }

    pub fn pause_torrent(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .pause_torrent(info_hash)
            .map_err(Error::from_cxx)
    }

    pub fn resume_torrent(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .resume_torrent(info_hash)
            .map_err(Error::from_cxx)
    }

    /// Removes a torrent, optionally deleting what it downloaded.
    pub fn remove_torrent(&mut self, info_hash: &str, with_data: bool) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .remove_torrent(info_hash, with_data)
            .map_err(Error::from_cxx)
    }
}

// ------------------------------------------------------------------- settings

impl Session {
    /// Applies session settings by name.
    ///
    /// All of them in one call, because libtorrent applies a pack atomically
    /// and applying them one at a time lets the session run briefly in a state
    /// nobody asked for.
    pub fn apply_settings(&mut self, settings: &[Setting]) -> Result<(), Error> {
        let raw: Vec<ffi::SettingValue> = settings.iter().map(Setting::to_ffi).collect();
        self.inner
            .pin_mut()
            .apply_settings(&raw)
            .map_err(Error::from_cxx)
    }

    /// Applies one setting. A shorthand over [`Session::apply_settings`].
    pub fn apply_setting(&mut self, setting: Setting) -> Result<(), Error> {
        self.apply_settings(&[setting])
    }

    pub fn setting_int(&self, name: &str) -> Result<i64, Error> {
        self.inner.setting_int(name).map_err(Error::from_cxx)
    }

    pub fn setting_bool(&self, name: &str) -> Result<bool, Error> {
        self.inner.setting_bool(name).map_err(Error::from_cxx)
    }

    pub fn setting_str(&self, name: &str) -> Result<String, Error> {
        self.inner.setting_str(name).map_err(Error::from_cxx)
    }

    /// Asks libtorrent to post a `session_stats_alert`.
    ///
    /// Counters arrive on the alert rather than from a getter, so this is a
    /// request and the answer comes through the alert loop.
    pub fn post_session_stats(&mut self) {
        self.inner.pin_mut().post_session_stats();
    }

    /// The names of the counters a `session_stats_alert` carries, in order.
    pub fn stat_names() -> Vec<String> {
        ffi::session_stat_names()
    }

    pub fn is_listening(&self) -> bool {
        self.inner.is_listening()
    }

    /// The port peers connect to, or 0 when not listening.
    pub fn listen_port(&self) -> u16 {
        self.inner.listen_port()
    }

    /// Replaces the peer IP filter with these rules, applied in order.
    ///
    /// Rules are not merged: a later one wins where it overlaps an earlier one,
    /// which is how an allowed range punches a hole in a blocked one. Passing
    /// an empty slice clears the filter and lets everything through again.
    pub fn set_ip_filter(&mut self, rules: &[IpRange]) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .set_ip_filter(rules)
            .map_err(Error::from_cxx)
    }

    /// Clears the filter. A shorthand over [`Session::set_ip_filter`].
    pub fn clear_ip_filter(&mut self) -> Result<(), Error> {
        self.set_ip_filter(&[])
    }

    /// How many ranges the filter holds, counting both address families.
    ///
    /// A filter that blocks nothing still holds one range per family, covering
    /// the whole address space and allowing it, so the floor is two.
    pub fn ip_filter_ranges(&self) -> i64 {
        self.inner.ip_filter_ranges()
    }
}

// ------------------------------------------------------------------- torrents

impl Session {
    /// Adds a torrent from a file, a magnet or resume data.
    pub fn add_torrent(&mut self, request: &AddTorrent) -> Result<String, Error> {
        self.inner
            .pin_mut()
            .add_torrent(&request.to_ffi())
            .map_err(Error::from_cxx)
    }

    /// The infohash of a `.torrent` file, without adding it.
    ///
    /// The daemon needs this to reject a duplicate before it has a handle.
    pub fn torrent_file_info_hash(torrent_file: &[u8]) -> Result<String, Error> {
        ffi::torrent_file_info_hash(torrent_file).map_err(Error::from_cxx)
    }

    /// Builds a `.torrent` from a file or directory on disk.
    ///
    /// Hashes every byte, so this blocks for as long as the content takes to
    /// read. Call it from somewhere that is allowed to block.
    pub fn create_torrent(
        path: &str,
        piece_length: i32,
        comment: &str,
        creator: &str,
        private: bool,
        trackers: &[String],
        web_seeds: &[String],
    ) -> Result<Vec<u8>, Error> {
        ffi::create_torrent(
            path,
            piece_length,
            comment,
            creator,
            private,
            trackers,
            web_seeds,
        )
        .map(|bytes| bytes.into_iter().collect())
        .map_err(Error::from_cxx)
    }

    /// Status for every torrent, in one crossing.
    ///
    /// The daemon polls all of them on a timer. One call per torrent is the
    /// difference between a cheap poll and an expensive one.
    pub fn all_torrent_status(&self) -> Vec<TorrentStatus> {
        self.inner
            .all_torrent_status()
            .into_iter()
            .map(TorrentStatus::from)
            .collect()
    }

    /// Whether this infohash names a torrent the session still holds.
    pub fn is_valid(&self, info_hash: &str) -> bool {
        self.inner.is_valid(info_hash)
    }

    pub fn force_recheck(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .force_recheck(info_hash)
            .map_err(Error::from_cxx)
    }

    /// Announces to the trackers again, after `seconds`.
    pub fn force_reannounce(&mut self, info_hash: &str, seconds: i32) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .force_reannounce(info_hash, seconds)
            .map_err(Error::from_cxx)
    }

    pub fn scrape_tracker(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .scrape_tracker(info_hash)
            .map_err(Error::from_cxx)
    }

    /// Clears a torrent's error so it can be retried.
    pub fn clear_error(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .clear_error(info_hash)
            .map_err(Error::from_cxx)
    }

    /// Moves a torrent's files. Completion arrives as a `StorageMoved` alert.
    pub fn move_storage(&mut self, info_hash: &str, destination: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .move_storage(info_hash, destination)
            .map_err(Error::from_cxx)
    }

    /// Turns flags on and off in one call. See [`FlagChange`].
    pub fn set_flags(&mut self, info_hash: &str, change: FlagChange) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .set_flags(info_hash, change.set, change.unset)
            .map_err(Error::from_cxx)
    }

    /// Connections for this torrent. -1 for no limit.
    pub fn set_max_connections(&mut self, info_hash: &str, limit: i32) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .set_max_connections(info_hash, limit)
            .map_err(Error::from_cxx)
    }

    /// Upload slots for this torrent. -1 for no limit.
    pub fn set_max_uploads(&mut self, info_hash: &str, limit: i32) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .set_max_uploads(info_hash, limit)
            .map_err(Error::from_cxx)
    }

    /// Bytes per second. -1 for no limit.
    pub fn set_download_limit(&mut self, info_hash: &str, limit: i32) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .set_download_limit(info_hash, limit)
            .map_err(Error::from_cxx)
    }

    /// Bytes per second. -1 for no limit.
    pub fn set_upload_limit(&mut self, info_hash: &str, limit: i32) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .set_upload_limit(info_hash, limit)
            .map_err(Error::from_cxx)
    }

    pub fn queue_position(&self, info_hash: &str) -> Result<i32, Error> {
        self.inner
            .queue_position(info_hash)
            .map_err(Error::from_cxx)
    }

    pub fn queue_top(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .queue_top(info_hash)
            .map_err(Error::from_cxx)
    }

    pub fn queue_up(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .queue_up(info_hash)
            .map_err(Error::from_cxx)
    }

    pub fn queue_down(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .queue_down(info_hash)
            .map_err(Error::from_cxx)
    }

    pub fn queue_bottom(&mut self, info_hash: &str) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .queue_bottom(info_hash)
            .map_err(Error::from_cxx)
    }
}

// ---------------------------------------------------------------------- files

impl Session {
    /// The files in a torrent. Empty until metadata arrives.
    pub fn files(&self, info_hash: &str) -> Result<Vec<FileEntry>, Error> {
        self.inner
            .files(info_hash)
            .map(|files| files.into_iter().map(FileEntry::from).collect())
            .map_err(Error::from_cxx)
    }

    /// Bytes downloaded per file, in file order.
    pub fn file_progress(&self, info_hash: &str) -> Result<Vec<i64>, Error> {
        self.inner
            .file_progress(info_hash)
            .map(|progress| progress.into_iter().collect())
            .map_err(Error::from_cxx)
    }

    /// Priority per file, 0 to 7. 0 means do not download.
    pub fn file_priorities(&self, info_hash: &str) -> Result<Vec<u8>, Error> {
        self.inner
            .file_priorities(info_hash)
            .map(|priorities| priorities.into_iter().collect())
            .map_err(Error::from_cxx)
    }

    /// Sets the priority of every file, 0 to 7.
    ///
    /// Applied on libtorrent's own thread, so reading the priorities back
    /// immediately still returns the old ones. Treat the call as accepted
    /// rather than done.
    pub fn prioritize_files(&mut self, info_hash: &str, priorities: &[u8]) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .prioritize_files(info_hash, priorities)
            .map_err(Error::from_cxx)
    }

    /// Renames one file. Completion arrives as a `FileRenamed` alert.
    pub fn rename_file(
        &mut self,
        info_hash: &str,
        index: i32,
        new_name: &str,
    ) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .rename_file(info_hash, index, new_name)
            .map_err(Error::from_cxx)
    }

    pub fn piece_priorities(&self, info_hash: &str) -> Result<Vec<u8>, Error> {
        self.inner
            .piece_priorities(info_hash)
            .map(|priorities| priorities.into_iter().collect())
            .map_err(Error::from_cxx)
    }

    /// Sets the priority of every piece. Asynchronous, like file priorities.
    pub fn prioritize_pieces(&mut self, info_hash: &str, priorities: &[u8]) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .prioritize_pieces(info_hash, priorities)
            .map_err(Error::from_cxx)
    }

    /// How many connected peers have each piece.
    pub fn piece_availability(&self, info_hash: &str) -> Result<Vec<i32>, Error> {
        self.inner
            .piece_availability(info_hash)
            .map(|availability| availability.into_iter().collect())
            .map_err(Error::from_cxx)
    }
}

// ----------------------------------------------------------- trackers, peers

impl Session {
    pub fn trackers(&self, info_hash: &str) -> Result<Vec<TrackerEntry>, Error> {
        self.inner
            .trackers(info_hash)
            .map(|entries| entries.into_iter().map(TrackerEntry::from).collect())
            .map_err(Error::from_cxx)
    }

    /// Replaces the whole announce list.
    ///
    /// Pass an empty `tiers` to put every tracker in tier 0, or one tier per
    /// tracker. A mismatched length is an error rather than a guess.
    pub fn replace_trackers(
        &mut self,
        info_hash: &str,
        urls: &[String],
        tiers: &[u8],
    ) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .replace_trackers(info_hash, urls, tiers)
            .map_err(Error::from_cxx)
    }

    pub fn add_tracker(&mut self, info_hash: &str, url: &str, tier: u8) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .add_tracker(info_hash, url, tier)
            .map_err(Error::from_cxx)
    }

    pub fn peers(&self, info_hash: &str) -> Result<Vec<PeerInfo>, Error> {
        self.inner
            .peers(info_hash)
            .map(|peers| peers.into_iter().map(PeerInfo::from).collect())
            .map_err(Error::from_cxx)
    }

    /// Adds a peer by hand, which is what the "connect peer" action does.
    pub fn connect_peer(&mut self, info_hash: &str, ip: &str, port: u16) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .connect_peer(info_hash, ip, port)
            .map_err(Error::from_cxx)
    }

    /// The bencoded `.torrent`, rebuilt from the parsed metadata.
    ///
    /// Equivalent to the original, not identical: libtorrent keeps the parsed
    /// form, so a torrent carrying unusual extra keys does not round-trip byte
    /// for byte.
    pub fn torrent_file(&self, info_hash: &str) -> Result<Vec<u8>, Error> {
        self.inner
            .torrent_file(info_hash)
            .map(|bytes| bytes.into_iter().collect())
            .map_err(Error::from_cxx)
    }

    /// Attaches a certificate to a torrent that requires one.
    pub fn set_ssl_certificate(
        &mut self,
        info_hash: &str,
        certificate: &[u8],
        private_key: &[u8],
        dh_params: &[u8],
        passphrase: &str,
    ) -> Result<(), Error> {
        self.inner
            .pin_mut()
            .set_ssl_certificate(info_hash, certificate, private_key, dh_params, passphrase)
            .map_err(Error::from_cxx)
    }
}
