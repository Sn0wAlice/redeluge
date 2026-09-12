// SPDX-License-Identifier: GPL-3.0-or-later
//! Safe Rust bindings to libtorrent-rasterbar, shaped for redeluge.
//!
//! This crate is the phase 0 spike of the migration: it proves that a `cxx`
//! bridge to libtorrent 2.0 can be built, linked and driven from Rust, and it
//! fixes the pattern the rest of the port will follow.
//!
//! The pattern is that nothing from libtorrent's type system crosses the
//! boundary. Alerts, which are a C++ class hierarchy read through `alert_cast`,
//! are flattened into a plain struct on the C++ side and reassembled into
//! [`Alert`] here. Torrents are addressed by their hex infohash rather than by
//! a handle, so Rust never holds a C++ object it could outlive.
//!
//! ```no_run
//! use redeluge_libtorrent::{Session, SessionSettings};
//!
//! let mut session = Session::new(&SessionSettings::default())?;
//! let info_hash = session.add_magnet("magnet:?xt=urn:btih:...", "/downloads")?;
//!
//! while session.wait_for_alert(std::time::Duration::from_secs(1)) {
//!     for alert in session.pop_alerts() {
//!         println!("{}: {}", alert.kind.handler_key(), alert.message);
//!     }
//! }
//! # Ok::<(), redeluge_libtorrent::Error>(())
//! ```

mod alert;
mod bridge;
mod session;
pub mod settings;
mod torrent;

pub use alert::{Alert, AlertKind};
pub use bridge::ffi::IpRange;
pub use bridge::HashProgress;
pub use session::{Session, SessionSettings};
pub use settings::Setting;
pub use torrent::{
    flags, AddTorrent, FileEntry, FlagChange, PeerInfo, TorrentState, TorrentStatus, TrackerEntry,
};

/// Anything the C++ side can refuse to do.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// libtorrent rejected the call. The message is libtorrent's own.
    #[error("libtorrent: {0}")]
    Libtorrent(String),

    /// The session could not be created at all, usually a bad listen interface.
    #[error("could not start the libtorrent session: {0}")]
    SessionStart(String),

    /// No torrent is registered under that infohash.
    #[error("unknown torrent: {0}")]
    UnknownTorrent(String),
}

impl Error {
    fn from_cxx(source: cxx::Exception) -> Self {
        let message = source.what().to_owned();
        match message.strip_prefix("no such torrent: ") {
            Some(hash) => Self::UnknownTorrent(hash.to_owned()),
            None => Self::Libtorrent(message),
        }
    }
}

/// Version of the libtorrent this was linked against, e.g. `2.0.11.0`.
pub fn libtorrent_version() -> String {
    bridge::ffi::libtorrent_version()
}
