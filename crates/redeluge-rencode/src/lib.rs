// SPDX-License-Identifier: GPL-3.0-or-later
//! The rencode wire format, as used by the Deluge RPC protocol.
//!
//! rencode is a variant of bencode that packs small values into their own type
//! code. It has no written specification: the Python module is the
//! specification, so this crate is checked against a corpus generated from that
//! module rather than against prose. See `contract/rencode-corpus.json` and
//! `tests/conformance.rs`.
//!
//! ```
//! use redeluge_rencode::{from_slice, to_vec, StringMode, Value};
//!
//! let request = Value::List(vec![
//!     Value::Int(0),
//!     Value::Str("daemon.info".into()),
//!     Value::List(vec![]),
//!     Value::Dict(vec![]),
//! ]);
//!
//! let encoded = to_vec(&request);
//! assert_eq!(from_slice(&encoded, StringMode::Utf8Lossy).unwrap(), request);
//! ```
//!
//! # Decoding is hostile-input code
//!
//! Everything decoded here arrives from an unauthenticated peer. The decoder
//! bounds its recursion ([`MAX_DEPTH`]), refuses lengths it could not hold
//! ([`MAX_LENGTH`]), and never allocates a container before it has the bytes.
//! The reference implementation does none of that.

mod decode;
mod encode;
mod error;
mod format;
mod value;

pub use decode::{from_slice, from_slice_prefix, MAX_DEPTH, MAX_LENGTH};
pub use encode::to_vec;
pub use error::{Error, Result, StringMode};
pub use value::Value;

/// The type codes, for anything that needs to reason about the wire form.
pub mod codes {
    pub use crate::format::*;
}
