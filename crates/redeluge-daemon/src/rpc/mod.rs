// SPDX-License-Identifier: GPL-3.0-or-later
//! The DelugeRPC listener.
//!
//! One task per connection. A connection is unauthenticated until
//! `daemon.login` succeeds, and until then only the two calls the Python daemon
//! also answers before authentication are reachable.
//!
//! # What changes from the Python implementation
//!
//! `deluge/core/rpcserver.py` sends the full Python traceback to the client on
//! any failure, including a failed login from an unauthenticated peer, which
//! leaks paths and versions. Here a failure carries an exception name and a
//! message, and the detail goes to the daemon's own log.
//!
//! It also answers `core.get_auth_levels_mappings` before authentication. That
//! is raised to read-only here: an unauthenticated method on this port should
//! be a decision rather than an accident.

pub mod dispatch;
pub mod server;
pub mod tls;

pub use dispatch::{CallContext, Rpc, RpcError};
pub use server::{Server, ServerConfig};
