// SPDX-License-Identifier: GPL-3.0-or-later
//! The DelugeRPC frame: a five-byte header, then a zlib-compressed rencode body.
//!
//! ```text
//!  ubyte    uint32          bytes
//! |.version.|..body size..|.....body.....|
//! ```
//!
//! # Two things the Python implementation gets wrong
//!
//! Both are reachable before a client has authenticated, on the daemon's TLS
//! port, and both are fixed here rather than reproduced.
//!
//! The size field is a 32-bit integer that `deluge/transfer.py` accepts as
//! given, buffering up to four gigabytes for a message nobody asked for. Here
//! it is checked against [`Limits::max_frame`] before a single byte is kept.
//!
//! The body is `zlib.decompress`ed with no bound on the output, so a few
//! kilobytes of zeroes expand into as much memory as the peer likes. Here
//! decompression stops at [`Limits::max_body`] and reports a failure.

use std::io::Read;

use flate2::bufread::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use redeluge_rencode::{from_slice, to_vec, StringMode, Value};

/// The only protocol version Deluge has ever spoken.
pub const PROTOCOL_VERSION: u8 = 1;

/// Bytes of header before the body starts.
pub const HEADER_SIZE: usize = 5;

/// What the framing layer refuses to do on a peer's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Largest compressed body accepted from the wire.
    ///
    /// A status response for a very large session is the biggest thing Deluge
    /// legitimately sends, and it compresses well; 16 MiB is far above that and
    /// far below what an unauthenticated peer should be able to make us hold.
    pub max_frame: usize,
    /// Largest body accepted after decompression.
    ///
    /// This is the bound that makes a compression bomb harmless.
    pub max_body: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame: 16 * 1024 * 1024,
            max_body: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported protocol version {found}, expected {PROTOCOL_VERSION}")]
    UnsupportedVersion { found: u8 },

    #[error("frame of {size} bytes exceeds the {limit} byte limit")]
    FrameTooLarge { size: usize, limit: usize },

    #[error("body expanded past the {limit} byte limit, refusing to continue")]
    BodyTooLarge { limit: usize },

    #[error("could not decompress the body: {0}")]
    Decompress(String),

    #[error("could not decode the body: {0}")]
    Decode(#[from] redeluge_rencode::Error),

    #[error("could not compress the body: {0}")]
    Compress(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Frames one value for the wire.
pub fn encode_frame(value: &Value) -> Result<Vec<u8>> {
    use std::io::Write;

    let body = to_vec(value);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&body)
        .map_err(|err| Error::Compress(err.to_string()))?;
    let compressed = encoder
        .finish()
        .map_err(|err| Error::Compress(err.to_string()))?;

    let mut frame = Vec::with_capacity(HEADER_SIZE + compressed.len());
    frame.push(PROTOCOL_VERSION);
    frame.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    frame.extend_from_slice(&compressed);
    Ok(frame)
}

/// Decompresses and decodes one body, refusing to expand past the limit.
pub fn decode_body(compressed: &[u8], limits: Limits) -> Result<Value> {
    // take() is what makes this safe: the decoder is never asked for more than
    // the limit, so a bomb stops at a known size instead of at the machine's.
    let mut decoder = ZlibDecoder::new(compressed).take(limits.max_body as u64 + 1);
    let mut body = Vec::new();
    decoder
        .read_to_end(&mut body)
        .map_err(|err| Error::Decompress(err.to_string()))?;

    if body.len() > limits.max_body {
        return Err(Error::BodyTooLarge {
            limit: limits.max_body,
        });
    }

    Ok(from_slice(&body, StringMode::Utf8Lossy)?)
}

/// Reassembles frames from a byte stream.
///
/// Feed it whatever arrives from the socket and take whole messages out. It
/// holds at most one frame plus whatever has been fed beyond it.
#[derive(Debug)]
pub struct FrameReader {
    buffer: Vec<u8>,
    limits: Limits,
    /// Size of the body currently being collected, once the header is in.
    expected: Option<usize>,
}

impl FrameReader {
    pub fn new(limits: Limits) -> Self {
        Self {
            buffer: Vec::new(),
            limits,
            expected: None,
        }
    }

    /// Bytes currently held, waiting for the rest of a frame.
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    /// Takes the next complete message, if one has arrived.
    ///
    /// Returns `Ok(None)` when more bytes are needed. An error means the peer is
    /// not speaking this protocol, and the connection should be dropped: the
    /// reader cannot resynchronise a stream whose framing it no longer trusts.
    pub fn next_message(&mut self) -> Result<Option<Value>> {
        if self.expected.is_none() {
            if self.buffer.len() < HEADER_SIZE {
                return Ok(None);
            }

            let version = self.buffer[0];
            if version != PROTOCOL_VERSION {
                return Err(Error::UnsupportedVersion { found: version });
            }

            let size = u32::from_be_bytes([
                self.buffer[1],
                self.buffer[2],
                self.buffer[3],
                self.buffer[4],
            ]) as usize;

            // Checked before anything is reserved, which is the whole point.
            if size > self.limits.max_frame {
                return Err(Error::FrameTooLarge {
                    size,
                    limit: self.limits.max_frame,
                });
            }

            self.buffer.drain(..HEADER_SIZE);
            self.expected = Some(size);
        }

        let size = self.expected.expect("set just above");
        if self.buffer.len() < size {
            return Ok(None);
        }

        let body: Vec<u8> = self.buffer.drain(..size).collect();
        self.expected = None;
        decode_body(&body, self.limits).map(Some)
    }
}

impl Default for FrameReader {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}
