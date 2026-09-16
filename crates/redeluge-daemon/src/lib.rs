// SPDX-License-Identifier: GPL-3.0-or-later
//! The redeluge daemon.
//!
//! It answers DelugeRPC on port 58846, drives libtorrent through the bridge,
//! and keeps the state a BitTorrent client has that libtorrent does not: which
//! torrents exist across restarts, what the user called them, which are
//! queued, and what each one is for.
//!
//! The wire contract is `contract/rpc-api.json`, extracted from the Python
//! daemon. Every method here is one of those, with the same name, arity and
//! authorisation level.

pub mod activity;
pub mod auth;
pub mod config;
pub mod core;
pub mod events;
pub mod features;
pub mod geoip;
pub mod manager;
pub mod peers;
pub mod prefs;
pub mod rpc;
pub mod state;
pub mod torrent;
