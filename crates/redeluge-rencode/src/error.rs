// SPDX-License-Identifier: GPL-3.0-or-later
//! Decoding failures.
//!
//! Every one of these is reachable from the network, before authentication, so
//! none of them may panic and none may allocate on the attacker's say-so.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The input ran out mid-value.
    #[error("truncated input: needed {needed} more bytes at offset {offset}")]
    Truncated { offset: usize, needed: usize },

    /// A type code that means nothing in this format.
    #[error("unknown type code {code} at offset {offset}")]
    UnknownTypeCode { code: u8, offset: usize },

    /// A decimal integer or string length that is not a number.
    #[error("malformed number at offset {offset}: {reason}")]
    MalformedNumber { offset: usize, reason: String },

    /// A length that would not fit in memory, or a decimal integer longer than
    /// the reference implementation accepts.
    #[error("length {length} at offset {offset} exceeds the limit of {limit}")]
    LengthLimit {
        offset: usize,
        length: u64,
        limit: u64,
    },

    /// Nesting deeper than [`crate::MAX_DEPTH`].
    ///
    /// Without this a short message of nothing but list markers overflows the
    /// stack, which is a remote crash on an unauthenticated port.
    #[error("nesting deeper than {limit} at offset {offset}")]
    TooDeep { offset: usize, limit: usize },

    /// A `CHR_LIST` or `CHR_DICT` that never ends.
    #[error("unterminated {container} started at offset {offset}")]
    Unterminated {
        offset: usize,
        container: &'static str,
    },

    /// A dictionary whose last key has no value.
    #[error("dictionary key without a value at offset {offset}")]
    DanglingKey { offset: usize },

    /// A complete value, followed by bytes that are not part of it.
    #[error("{extra} trailing bytes after a complete value")]
    TrailingBytes { extra: usize },
}

/// Whether the failure means "give me more bytes" rather than "this is broken".
impl Error {
    pub fn is_truncated(&self) -> bool {
        matches!(self, Self::Truncated { .. })
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// How a string was decoded, mirroring the reference implementation's
/// `decode_utf8` switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StringMode {
    /// Leave every string as raw bytes.
    Raw,
    /// Decode strings as UTF-8, keeping the raw bytes when that fails.
    ///
    /// This is the mode Deluge uses. The reference implementation raises on
    /// invalid UTF-8; a peer controls torrent names, so that is reachable from
    /// the network. Falling back keeps the message readable instead of
    /// discarding it.
    #[default]
    Utf8Lossy,
}

impl fmt::Display for StringMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw => write!(f, "raw"),
            Self::Utf8Lossy => write!(f, "utf8-lossy"),
        }
    }
}
