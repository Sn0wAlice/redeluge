// SPDX-License-Identifier: GPL-3.0-or-later
//! The DelugeRPC wire protocol.
//!
//! A client speaks this to the daemon over TLS: a five-byte header, a
//! zlib-compressed rencode body, and four message shapes on top. It is what the
//! web server uses to reach the daemon, and for now that daemon is still the
//! Python one, which is exactly the point. Replacing `deluge-web` first means
//! this codec is checked against a running reference implementation rather than
//! against itself.

pub mod client;
pub mod message;
pub mod tls;
pub mod transfer;

pub use client::{Client, ClientSettings, Event, DEFAULT_DAEMON_PORT};
pub use message::{requests_to_value, Failure, Incoming, Request};
pub use tls::TlsMode;
pub use transfer::{decode_body, encode_frame, FrameReader, Limits, PROTOCOL_VERSION};
