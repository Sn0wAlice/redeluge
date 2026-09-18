// SPDX-License-Identifier: GPL-3.0-or-later
//! The FFI boundary.
//!
//! Everything crossing between Rust and C++ is a plain struct of scalars and
//! strings. libtorrent's alerts are a C++ class hierarchy read with
//! `alert_cast`, which does not translate into Rust; the shim flattens each one
//! into [`FlatAlert`] on the C++ side so the boundary stays trivial. That
//! choice is what keeps this crate maintainable across libtorrent releases.

// `create_torrent` carries every field of a torrent file, which is more than
// the lint likes. Grouping them into a struct would put a second type on the
// boundary for no gain.
#[allow(clippy::too_many_arguments)]
#[cxx::bridge(namespace = "redeluge")]
pub mod ffi {

    /// One libtorrent alert, flattened into scalars.
    ///
    /// `kind` is a discriminant from `contract/alerts.json`, in the order the
    /// contract lists them. The numeric and string payload slots carry the
    /// per-alert fields the daemon actually reads; `message` always holds
    /// libtorrent's own rendering, so nothing is lost when a slot is unused.
    #[derive(Debug, Clone, PartialEq)]
    struct FlatAlert {
        kind: u16,
        /// libtorrent's short name for the alert, e.g. `torrent_finished`.
        what: String,
        message: String,
        /// Hex v1 infohash, empty when the alert is not torrent-scoped.
        info_hash: String,
        num_a: i64,
        num_b: i64,
        str_a: String,
        str_b: String,
        /// Opaque payload, currently only the bencoded resume data carried by
        /// `save_resume_data_alert`. Empty for every other alert.
        blob: Vec<u8>,
        /// Session counters, carried only by `session_stats_alert`. Their names
        /// are `session_stat_names()`, in the same order.
        counters: Vec<i64>,
    }

    /// Session settings the spike needs. The full settings pack, all 77 keys of
    /// it, arrives with the daemon in phase 3.
    #[derive(Debug, Clone)]
    struct SessionConfig {
        /// libtorrent `listen_interfaces` syntax, e.g. `0.0.0.0:6881`.
        listen_interfaces: String,
        user_agent: String,
        enable_dht: bool,
        enable_lsd: bool,
        enable_upnp: bool,
        enable_natpmp: bool,
        alert_queue_size: i32,
    }

    /// One libtorrent session setting, tagged with its type.
    ///
    /// libtorrent looks settings up by name and encodes the type in the index,
    /// so the bridge takes a name and a tagged value rather than one function
    /// per setting. That is 29 settings today and whatever Deluge adds later,
    /// with no change here.
    #[derive(Debug, Clone, PartialEq)]
    struct SettingValue {
        name: String,
        /// 0 int, 1 bool, 2 string.
        kind: u8,
        int_value: i64,
        bool_value: bool,
        str_value: String,
    }

    /// Everything needed to add a torrent.
    ///
    /// One struct rather than a function per source, because the daemon adds
    /// torrents from a file, a magnet or resume data with the same options
    /// attached, and three near-identical entry points would drift.
    #[derive(Debug, Clone, Default)]
    struct AddTorrentRequest {
        /// Bencoded `.torrent` contents. Empty when adding from a magnet.
        torrent_file: Vec<u8>,
        /// Magnet URI. Empty when adding from a file.
        magnet_uri: String,
        /// Previously saved resume data. Empty when there is none.
        resume_data: Vec<u8>,
        save_path: String,
        /// Overrides the name from the metadata. Empty to keep it.
        name: String,
        /// One priority per file, 0 to 7. Empty to leave them at the default.
        file_priorities: Vec<u8>,
        /// Replaces the trackers in the metadata. Empty to keep them.
        trackers: Vec<String>,
        /// libtorrent `torrent_flags` to turn on and off.
        flags_set: u64,
        flags_unset: u64,
        /// `storage_mode_sparse` unless this is set.
        pre_allocate: bool,
    }

    /// One peer connected to a torrent.
    #[derive(Debug, Clone, PartialEq)]
    struct PeerInfo {
        ip: String,
        port: u16,
        client: String,
        /// Hex peer id.
        peer_id: String,
        down_speed: i32,
        up_speed: i32,
        progress: f32,
        /// True when the peer has every piece.
        seed: bool,
        /// Two-letter country code, empty when unknown.
        country: String,
        /// True when the connection is over uTP rather than TCP.
        utp: bool,
        /// True when the connection is encrypted, either scheme.
        encrypted: bool,
        /// How many pieces this peer has that we do not.
        ///
        /// The one number that says whether a peer is worth having: a seed we
        /// are already ahead of is nothing to us, and a peer at 3% can be the
        /// only one with the piece we are waiting on.
        useful_pieces: i32,
        /// Bytes sent to and received from this peer on this connection.
        ///
        /// Per connection, not for all time: both reset to zero when a peer
        /// disconnects and comes back. Anything that wants a running total has
        /// to notice that and add the difference itself.
        total_upload: i64,
        total_download: i64,
    }

    /// One tracker in a torrent's announce list.
    #[derive(Debug, Clone, PartialEq)]
    struct TrackerEntry {
        url: String,
        tier: u8,
        /// Empty when the last announce succeeded or none has happened.
        message: String,
        verified: bool,
        updating: bool,
        fails: i32,
    }

    /// One file inside a torrent.
    #[derive(Debug, Clone, PartialEq)]
    struct FileEntry {
        index: i32,
        path: String,
        size: i64,
        offset: i64,
    }

    /// What the daemon reports about a torrent.
    ///
    /// Every field here is one the Python `Torrent.get_status` reads. Fields
    /// libtorrent offers and Deluge ignores are deliberately absent: crossing
    /// them would cost on every status poll, and status is polled constantly.
    #[derive(Debug, Clone, PartialEq)]
    struct TorrentStatus {
        info_hash: String,
        name: String,
        save_path: String,
        /// libtorrent `torrent_status::state_t`.
        state: u8,
        progress: f32,
        /// Current `torrent_flags`, for the nine Deluge cares about.
        flags: u64,

        download_rate: i32,
        upload_rate: i32,
        download_payload_rate: i32,
        upload_payload_rate: i32,

        num_peers: i32,
        num_seeds: i32,
        num_complete: i32,
        num_incomplete: i32,
        connect_candidates: i32,

        total_done: i64,
        total_wanted: i64,
        total_wanted_done: i64,
        total_payload_download: i64,
        total_payload_upload: i64,
        all_time_download: i64,
        all_time_upload: i64,

        /// Seconds.
        active_time: i64,
        seeding_time: i64,
        time_since_download: i64,
        time_since_upload: i64,
        /// Unix timestamps; 0 when it has not happened.
        added_time: i64,
        completed_time: i64,
        finished_time: i64,
        last_seen_complete: i64,
        /// Seconds until the next announce.
        next_announce: i64,

        distributed_copies: f32,
        queue_position: i32,
        seed_rank: i32,
        storage_mode: u8,

        is_finished: bool,
        is_seeding: bool,
        is_paused: bool,
        has_metadata: bool,
        moving_storage: bool,

        current_tracker: String,
        /// Empty when the torrent is not in an error state.
        error: String,
        /// The file the error is about, when the error names one.
        error_file: String,

        /// One bool per piece, empty when there is no metadata yet.
        pieces: Vec<u8>,
        num_pieces: i32,
        piece_length: i32,
        total_size: i64,
        num_files: i32,
    }

    /// One rule in the peer IP filter: an inclusive address range, blocked or
    /// allowed.
    ///
    /// Addresses are text because the filter is built from a downloaded list of
    /// text ranges, and parsing them once on this side avoids a second address
    /// type crossing the boundary.
    #[derive(Debug, Clone)]
    struct IpRange {
        first: String,
        last: String,
        blocked: bool,
    }

    extern "Rust" {
        /// Where hashing progress goes. Opaque to C++, which only calls the
        /// one method on it.
        type HashProgress;
        fn note_piece(self: &mut HashProgress, piece: i32, total: i32);
    }

    unsafe extern "C++" {
        include!("shim.h");

        /// Owns the `lt::session` and the handle table.
        type Session;

        fn new_session(config: &SessionConfig) -> Result<UniquePtr<Session>>;

        /// Version string of the libtorrent this was linked against.
        fn libtorrent_version() -> String;

        /// libtorrent's own encoding of a client fingerprint, `-XX1234-`.
        fn generate_fingerprint(
            name: &str,
            major: i32,
            minor: i32,
            revision: i32,
            tag: i32,
        ) -> String;

        /// Applies session settings by name.
        ///
        /// Unknown names and type mismatches are reported rather than ignored:
        /// a setting that silently does nothing is how a rate limit stops
        /// applying and nobody notices for a month.
        fn apply_settings(self: Pin<&mut Session>, settings: &[SettingValue]) -> Result<()>;

        /// Reads one session setting back.
        fn setting_int(&self, name: &str) -> Result<i64>;
        fn setting_bool(&self, name: &str) -> Result<bool>;
        fn setting_str(&self, name: &str) -> Result<String>;

        /// The names of the counters a `session_stats_alert` carries, in order.
        ///
        /// Counters arrive on the alert rather than from a getter, which is why
        /// there is no function to read them here.
        fn session_stat_names() -> Vec<String>;

        /// Where libtorrent says one metric's value sits.
        ///
        /// The authority for what `session_stat_names` has to line up with.
        /// -1 when there is no such metric.
        fn session_stat_index(name: &str) -> i32;
        /// Asks libtorrent to post a `session_stats_alert`.
        fn post_session_stats(self: Pin<&mut Session>);

        /// Whether the session is listening for peers.
        fn is_listening(&self) -> bool;
        fn listen_port(&self) -> u16;

        /// Replaces the peer IP filter with these rules, applied in order.
        ///
        /// Order is the whole semantics: a later rule wins over an earlier one
        /// where they overlap, which is how a whitelist entry punches a hole in
        /// a blocked range. An empty slice clears the filter.
        fn set_ip_filter(self: Pin<&mut Session>, rules: &[IpRange]) -> Result<()>;

        /// How many ranges the filter holds, counting both address families.
        ///
        /// A filter that blocks nothing still holds one range per family, which
        /// covers the whole space and allows it.
        fn ip_filter_ranges(&self) -> i64;

        /// Adds a torrent, returning its hex v1 infohash.
        fn add_torrent(self: Pin<&mut Session>, request: &AddTorrentRequest) -> Result<String>;

        /// Adds a magnet URI with no options. A shorthand over `add_torrent`.
        fn add_magnet(self: Pin<&mut Session>, uri: &str, save_path: &str) -> Result<String>;

        /// Adds a torrent from resume data alone, the restart path.
        fn add_torrent_from_resume(
            self: Pin<&mut Session>,
            resume_data: &[u8],
            save_path: &str,
        ) -> Result<String>;

        /// Builds a `.torrent` from a file or directory on disk.
        ///
        /// Returns the bencoded file. Hashing every piece is slow and blocking,
        /// so this belongs on a thread that is allowed to block.
        fn create_torrent(
            path: &str,
            piece_length: i32,
            comment: &str,
            creator: &str,
            // Not `private`: cxx copies parameter names into C++, where that
            // is a keyword.
            private_torrent: bool,
            trackers: &[String],
            web_seeds: &[String],
            // Called once per piece as the hashing runs, so a client watching
            // a progress dialog sees it move.
            progress: &mut HashProgress,
        ) -> Result<Vec<u8>>;

        /// Reads a `.torrent` file's infohash without adding it.
        ///
        /// The daemon needs this to reject a duplicate before it has a handle.
        fn torrent_file_info_hash(torrent_file: &[u8]) -> Result<String>;

        /// Blocks up to `millis` for at least one alert to be queued.
        fn wait_for_alert(self: Pin<&mut Session>, millis: i32) -> bool;

        /// Drains the alert queue. Never blocks.
        fn pop_alerts(self: Pin<&mut Session>) -> Vec<FlatAlert>;

        // ------------------------------------------------------------ torrents

        fn torrent_status(&self, info_hash: &str) -> Result<TorrentStatus>;
        /// Status for every torrent, in one crossing.
        ///
        /// The daemon polls every torrent on a timer, and one call per torrent
        /// is the difference between a cheap poll and an expensive one.
        fn all_torrent_status(&self) -> Vec<TorrentStatus>;
        fn torrent_hashes(&self) -> Vec<String>;
        fn is_valid(&self, info_hash: &str) -> bool;

        fn pause_torrent(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;
        fn resume_torrent(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;
        fn remove_torrent(self: Pin<&mut Session>, info_hash: &str, with_data: bool) -> Result<()>;
        fn force_recheck(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;
        fn force_reannounce(self: Pin<&mut Session>, info_hash: &str, seconds: i32) -> Result<()>;
        fn scrape_tracker(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;
        fn clear_error(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;
        fn move_storage(self: Pin<&mut Session>, info_hash: &str, destination: &str) -> Result<()>;

        /// Turns `torrent_flags` on and off in one call.
        ///
        /// Two calls would leave a window where the torrent has neither state,
        /// which for the paused flag means it briefly starts.
        fn set_flags(self: Pin<&mut Session>, info_hash: &str, set: u64, unset: u64) -> Result<()>;

        fn set_max_connections(self: Pin<&mut Session>, info_hash: &str, limit: i32) -> Result<()>;
        fn set_max_uploads(self: Pin<&mut Session>, info_hash: &str, limit: i32) -> Result<()>;
        fn set_download_limit(
            self: Pin<&mut Session>,
            info_hash: &str,
            bytes_per_second: i32,
        ) -> Result<()>;
        fn set_upload_limit(
            self: Pin<&mut Session>,
            info_hash: &str,
            bytes_per_second: i32,
        ) -> Result<()>;

        fn queue_position(&self, info_hash: &str) -> Result<i32>;
        fn queue_top(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;
        fn queue_up(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;
        fn queue_down(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;
        fn queue_bottom(self: Pin<&mut Session>, info_hash: &str) -> Result<()>;

        // --------------------------------------------------------------- files

        fn files(&self, info_hash: &str) -> Result<Vec<FileEntry>>;
        fn file_progress(&self, info_hash: &str) -> Result<Vec<i64>>;
        fn file_priorities(&self, info_hash: &str) -> Result<Vec<u8>>;
        fn prioritize_files(
            self: Pin<&mut Session>,
            info_hash: &str,
            priorities: &[u8],
        ) -> Result<()>;
        fn rename_file(
            self: Pin<&mut Session>,
            info_hash: &str,
            index: i32,
            new_name: &str,
        ) -> Result<()>;

        fn piece_priorities(&self, info_hash: &str) -> Result<Vec<u8>>;
        fn prioritize_pieces(
            self: Pin<&mut Session>,
            info_hash: &str,
            priorities: &[u8],
        ) -> Result<()>;
        fn piece_availability(&self, info_hash: &str) -> Result<Vec<i32>>;

        // ------------------------------------------------------ trackers, peers

        fn trackers(&self, info_hash: &str) -> Result<Vec<TrackerEntry>>;
        fn replace_trackers(
            self: Pin<&mut Session>,
            info_hash: &str,
            urls: &[String],
            tiers: &[u8],
        ) -> Result<()>;
        fn add_tracker(self: Pin<&mut Session>, info_hash: &str, url: &str, tier: u8)
            -> Result<()>;

        fn peers(&self, info_hash: &str) -> Result<Vec<PeerInfo>>;
        fn connect_peer(
            self: Pin<&mut Session>,
            info_hash: &str,
            ip: &str,
            port: u16,
        ) -> Result<()>;

        // ------------------------------------------------------- resume, metadata

        fn save_resume_data(
            self: Pin<&mut Session>,
            info_hash: &str,
            flush_disk_cache: bool,
        ) -> Result<()>;
        fn needs_resume_save(&self, info_hash: &str) -> Result<bool>;

        /// The bencoded `.torrent`, once metadata has arrived.
        fn torrent_file(&self, info_hash: &str) -> Result<Vec<u8>>;

        /// Attaches an SSL certificate, for torrents that require one.
        fn set_ssl_certificate(
            self: Pin<&mut Session>,
            info_hash: &str,
            certificate: &[u8],
            private_key: &[u8],
            dh_params: &[u8],
            passphrase: &str,
        ) -> Result<()>;
    }
}

/// A sink for hashing progress, borrowed by C++ for the length of one call.
///
/// Creating a torrent reads every byte of its content, which can take minutes,
/// and a client watching a progress dialog needs to see it move. libtorrent
/// reports each piece through a callback, so something has to cross back into
/// Rust; this is the only place anything does.
pub struct HashProgress {
    sink: Box<dyn FnMut(i32, i32) + Send>,
}

impl HashProgress {
    /// `sink` is called with the piece just hashed and the total.
    pub fn new(sink: Box<dyn FnMut(i32, i32) + Send>) -> Self {
        Self { sink }
    }

    /// Progress that goes nowhere, for a caller that does not want it.
    pub fn ignored() -> Self {
        Self::new(Box::new(|_, _| {}))
    }

    fn note_piece(&mut self, piece: i32, total: i32) {
        (self.sink)(piece, total);
    }
}
