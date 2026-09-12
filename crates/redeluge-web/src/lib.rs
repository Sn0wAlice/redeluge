// SPDX-License-Identifier: GPL-3.0-or-later
//! The redeluge Web UI server.
//!
//! It answers the same `POST /json` the Python `deluge-web` answers, and serves
//! the same ExtJS assets, so the browser cannot tell the difference. Calls that
//! belong to the daemon are forwarded over DelugeRPC; the rest are answered
//! here, because they are about the web session rather than about torrents.
//!
//! For now the daemon on the other end is still the Python one. That is
//! deliberate: it makes the protocol code testable against a reference
//! implementation before anything else is replaced.

pub mod assets;
pub mod auth;
pub mod bencode;
pub mod bootstrap;
pub mod config;
pub mod convert;
pub mod hostlist;
pub mod index;
pub mod json_api;
pub mod minify;
pub mod persist;
pub mod routes;
pub mod state;
pub mod template;
pub mod throttle;
pub mod torrentfile;
pub mod upload;
