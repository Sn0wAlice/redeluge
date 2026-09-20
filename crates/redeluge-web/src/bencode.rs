// SPDX-License-Identifier: GPL-3.0-or-later
//! Just enough bencode to read a `.torrent` file.
//!
//! The daemon needs no bencode at all: resume data crosses the FFI boundary as
//! an opaque blob and libtorrent reads it. The Web UI server is the one place
//! that has to look inside a torrent, because the add dialog shows the name and
//! the file tree before anything is added.
//!
//! Only decoding, and only what a torrent file contains. The one subtlety is
//! that an infohash is the SHA-1 of the *raw bytes* of the `info` dictionary,
//! not of a re-encoding of it: re-encoding normalises, and a torrent whose
//! producer ordered its keys unusually would come out with a different hash and
//! fail to match the swarm. So every value remembers where it came from.

use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("unexpected end of input at byte {0}")]
    Truncated(usize),
    #[error("not bencode at byte {0}")]
    Malformed(usize),
    #[error("nested more than {0} deep")]
    TooDeep(usize),
    #[error("a length that does not fit at byte {0}")]
    BadLength(usize),
}

pub type Result<T> = std::result::Result<T, Error>;

/// How deep a torrent may nest. A real one is three or four.
const MAX_DEPTH: usize = 32;

/// One bencode value, with the byte range it occupied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    /// Keys are byte strings in the file; they are kept as bytes rather than
    /// text because nothing guarantees UTF-8, and sorted because bencode
    /// requires it.
    Dict(BTreeMap<Vec<u8>, Value>),
}

impl Value {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) => Some(value),
            _ => None,
        }
    }

    /// A byte string as text, replacing anything that is not UTF-8.
    ///
    /// A torrent name is written by whoever made it and is not required to be
    /// UTF-8. Dropping the field would hide the torrent the user is looking at.
    pub fn as_text(&self) -> Option<String> {
        self.as_bytes()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Dict(entries) => entries.get(key.as_bytes()),
            _ => None,
        }
    }
}

/// Decodes one value from the front of `input`.
pub fn decode(input: &[u8]) -> Result<Value> {
    let mut parser = Parser {
        input,
        at: 0,
        depth: 0,
    };
    parser.value()
}

/// Decodes a value and reports where one of its top-level members lay.
///
/// Used for `info`, whose raw bytes are what the infohash is taken over.
pub fn decode_with_span(input: &[u8], key: &str) -> Result<(Value, Option<Range<usize>>)> {
    let mut parser = Parser {
        input,
        at: 0,
        depth: 0,
    };
    let mut span = None;
    let value = parser.value_noting(key.as_bytes(), &mut span)?;
    Ok((value, span))
}

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Result<u8> {
        self.input
            .get(self.at)
            .copied()
            .ok_or(Error::Truncated(self.at))
    }

    fn value(&mut self) -> Result<Value> {
        let mut ignored = None;
        self.value_noting(b"\0none", &mut ignored)
    }

    /// Parses a value; if it is a dictionary, records where `wanted` lay.
    ///
    /// Only the outermost dictionary is considered, which is what an infohash
    /// needs and keeps this from tracking a span per nested key.
    fn value_noting(&mut self, wanted: &[u8], span: &mut Option<Range<usize>>) -> Result<Value> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::TooDeep(MAX_DEPTH));
        }
        let outermost = self.depth == 1;

        let value = match self.peek()? {
            b'i' => self.integer()?,
            b'l' => {
                self.at += 1;
                let mut items = Vec::new();
                while self.peek()? != b'e' {
                    items.push(self.value()?);
                }
                self.at += 1;
                Value::List(items)
            }
            b'd' => {
                self.at += 1;
                let mut entries = BTreeMap::new();
                while self.peek()? != b'e' {
                    let key = match self.value()? {
                        Value::Bytes(key) => key,
                        _ => return Err(Error::Malformed(self.at)),
                    };
                    let start = self.at;
                    let item = self.value()?;
                    if outermost && key == wanted {
                        *span = Some(start..self.at);
                    }
                    entries.insert(key, item);
                }
                self.at += 1;
                Value::Dict(entries)
            }
            b'0'..=b'9' => self.bytes()?,
            _ => return Err(Error::Malformed(self.at)),
        };

        self.depth -= 1;
        Ok(value)
    }

    fn integer(&mut self) -> Result<Value> {
        let start = self.at + 1;
        let end = self.find(b'e', start)?;
        let text =
            std::str::from_utf8(&self.input[start..end]).map_err(|_| Error::Malformed(start))?;
        let value: i64 = text.parse().map_err(|_| Error::Malformed(start))?;
        self.at = end + 1;
        Ok(Value::Int(value))
    }

    fn bytes(&mut self) -> Result<Value> {
        let colon = self.find(b':', self.at)?;
        let text = std::str::from_utf8(&self.input[self.at..colon])
            .map_err(|_| Error::Malformed(self.at))?;
        let length: usize = text.parse().map_err(|_| Error::BadLength(self.at))?;

        let start = colon + 1;
        let end = start.checked_add(length).ok_or(Error::BadLength(self.at))?;
        if end > self.input.len() {
            return Err(Error::Truncated(self.at));
        }
        self.at = end;
        Ok(Value::Bytes(self.input[start..end].to_vec()))
    }

    fn find(&self, byte: u8, from: usize) -> Result<usize> {
        self.input[from..]
            .iter()
            .position(|candidate| *candidate == byte)
            .map(|offset| from + offset)
            .ok_or(Error::Truncated(from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_strings_lists_and_dictionaries_decode() {
        assert_eq!(decode(b"i42e").unwrap(), Value::Int(42));
        assert_eq!(decode(b"i-7e").unwrap(), Value::Int(-7));
        assert_eq!(decode(b"4:spam").unwrap(), Value::Bytes(b"spam".to_vec()));
        assert_eq!(decode(b"0:").unwrap(), Value::Bytes(Vec::new()));
        assert_eq!(
            decode(b"li1ei2ee").unwrap(),
            Value::List(vec![Value::Int(1), Value::Int(2)])
        );

        let dict = decode(b"d3:cow3:moo4:spam4:eggse").unwrap();
        assert_eq!(dict.get("cow").unwrap().as_text().unwrap(), "moo");
        assert_eq!(dict.get("spam").unwrap().as_text().unwrap(), "eggs");
    }

    #[test]
    fn a_name_that_is_not_utf8_still_reads() {
        // A torrent name is written by whoever made it. Dropping the field
        // would hide the torrent the user is looking at.
        let value = Value::Bytes(vec![0xff, 0xfe, b'o', b'k']);
        assert!(value.as_text().unwrap().ends_with("ok"));
    }

    #[test]
    fn truncated_input_is_an_error_not_a_panic() {
        for input in [
            &b"i42"[..],
            b"4:spa",
            b"li1e",
            b"d3:cow",
            b"d3:cow3:moo",
            b"",
            b"5:",
        ] {
            assert!(decode(input).is_err(), "{input:?} should not decode");
        }
    }

    #[test]
    fn rubbish_is_an_error() {
        assert!(decode(b"x").is_err());
        assert!(decode(b"ie").is_err());
        assert!(decode(b"i1x2e").is_err());
        assert!(decode(b"d3:cowi1ei2ee").is_err(), "a non-string key");
    }

    #[test]
    fn a_length_that_would_overflow_is_refused() {
        // The length field is attacker-controlled in a file someone uploads.
        let input = b"99999999999999999999:x";
        assert!(matches!(decode(input), Err(Error::BadLength(_))));
    }

    #[test]
    fn a_length_longer_than_the_input_is_refused_rather_than_read() {
        assert!(matches!(decode(b"100:short"), Err(Error::Truncated(_))));
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = "l".repeat(MAX_DEPTH + 5) + &"e".repeat(MAX_DEPTH + 5);
        assert!(matches!(
            decode(deep.as_bytes()),
            Err(Error::TooDeep(MAX_DEPTH))
        ));
    }

    #[test]
    fn the_span_of_a_top_level_member_is_reported() {
        // This is what makes an infohash right: the raw bytes, not a
        // re-encoding, which would normalise key order and change the hash.
        let input = b"d4:infod4:name4:testee";
        let (value, span) = decode_with_span(input, "info").unwrap();
        let span = span.expect("info is there");

        assert_eq!(&input[span.clone()], b"d4:name4:teste");
        assert_eq!(
            value
                .get("info")
                .unwrap()
                .get("name")
                .unwrap()
                .as_text()
                .unwrap(),
            "test"
        );
    }

    #[test]
    fn a_missing_member_has_no_span() {
        let (_, span) = decode_with_span(b"d3:cow3:mooe", "info").unwrap();
        assert!(span.is_none());
    }

    #[test]
    fn only_the_outermost_dictionary_is_searched_for_the_span() {
        // A nested key called `info` must not be mistaken for the real one.
        let input = b"d5:outerd4:info3:note4:infod4:name2:okee";
        let (_, span) = decode_with_span(input, "info").unwrap();
        let span = span.expect("the outer info");
        assert_eq!(&input[span], b"d4:name2:oke");
    }
}

#[cfg(test)]
mod hostile_tests {
    //! The decoder reads bytes somebody else wrote.
    //!
    //! A `.torrent` is uploaded by a person who has logged in, so this is not
    //! the daemon's pre-authentication surface — `rencode` is, and it has a
    //! suite of its own. It is still a parser fed from outside, and the cost
    //! of finding out it panics is a thread dying under a request.
    //!
    //! The noise here is deterministic rather than a fuzzer's: a failure
    //! reproduces on the next run and on somebody else's machine, and it runs
    //! in the gate on every push instead of on a schedule nobody watches.

    use super::*;

    /// Deterministic noise, so a failure is reproducible without a fuzzer.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn byte(&mut self) -> u8 {
            (self.next() >> 24) as u8
        }
    }

    /// A handful of well-formed inputs to corrupt.
    fn corpus() -> Vec<Vec<u8>> {
        vec![
            b"i42e".to_vec(),
            b"4:spam".to_vec(),
            b"li1ei2ei3ee".to_vec(),
            b"d3:cow3:moo4:spam4:eggse".to_vec(),
            b"d4:infod6:lengthi1024e4:name4:test12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee".to_vec(),
            b"d8:announce30:udp://tracker.invalid:6969/ann4:infod6:lengthi1eee".to_vec(),
        ]
    }

    #[test]
    fn every_truncation_of_valid_input_is_handled() {
        // The commonest malformed input there is: a file that stopped early.
        for input in corpus() {
            for cut in 0..input.len() {
                let _ = decode(&input[..cut]);
                let _ = decode_with_span(&input[..cut], "info");
            }
        }
    }

    #[test]
    fn every_single_byte_corruption_is_handled() {
        for input in corpus() {
            for at in 0..input.len() {
                for byte in [0u8, b'0', b'9', b'e', b'i', b'l', b'd', b':', 0xff] {
                    let mut broken = input.clone();
                    broken[at] = byte;
                    let _ = decode(&broken);
                    let _ = decode_with_span(&broken, "info");
                }
            }
        }
    }

    #[test]
    fn random_bytes_never_panic() {
        let mut rng = Rng(0x5eed_1312);
        for _ in 0..2000 {
            let length = (rng.next() % 64) as usize;
            let bytes: Vec<u8> = (0..length).map(|_| rng.byte()).collect();
            let _ = decode(&bytes);
            let _ = decode_with_span(&bytes, "info");
        }
    }

    #[test]
    fn random_bytes_that_start_like_a_torrent_never_panic() {
        // Noise alone is rejected by the first byte most of the time, which
        // tests very little. This keeps the shape and corrupts the inside.
        let mut rng = Rng(0xfeed_face);
        let template = b"d4:infod6:lengthi1024e4:name4:test12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";
        for _ in 0..2000 {
            let mut bytes = template.to_vec();
            let hits = 1 + (rng.next() % 4) as usize;
            for _ in 0..hits {
                let at = (rng.next() as usize) % bytes.len();
                bytes[at] = rng.byte();
            }
            let _ = decode(&bytes);
            let _ = decode_with_span(&bytes, "info");
        }
    }

    #[test]
    fn deep_nesting_is_refused_rather_than_overflowing_the_stack() {
        // A thousand opening brackets is four bytes of typing and a stack
        // overflow is not an error you can catch.
        let deep: Vec<u8> = b"l".repeat(10_000);
        assert!(decode(&deep).is_err());

        let dicts: Vec<u8> = b"d1:a".repeat(10_000);
        assert!(decode(&dicts).is_err());
    }

    #[test]
    fn nesting_just_within_the_limit_still_works() {
        let mut input = b"l".repeat(MAX_DEPTH - 1);
        input.extend_from_slice(b"i1e");
        input.extend(b"e".repeat(MAX_DEPTH - 1));
        assert!(decode(&input).is_ok(), "a legitimate torrent was refused");
    }

    #[test]
    fn an_enormous_declared_string_length_is_refused_without_allocating() {
        // The length is a number in the file. Believing it is how a few bytes
        // become a memory limit.
        assert!(decode(b"99999999999999999999:abc").is_err());
        assert!(decode(b"4294967296:abc").is_err());
        assert!(decode(b"-1:abc").is_err());
    }

    #[test]
    fn an_integer_that_is_not_one_is_refused() {
        for input in [
            &b"ie"[..],
            b"i-e",
            b"i--1e",
            b"i1",
            b"i99999999999999999999999e",
            b"i 1e",
        ] {
            assert!(decode(input).is_err(), "{:?} was accepted", input);
        }
    }

    #[test]
    fn what_follows_a_value_is_ignored_rather_than_refused() {
        // Deliberate, and worth pinning: `decode` reads one value from the
        // front of the input. Torrent files with something appended exist —
        // a tracker's comment, a tool's padding — and refusing them would
        // refuse files every other client opens. Nothing is lost by the
        // leniency: the infohash is taken over the `info` span, which is
        // bounded by the dictionary rather than by the end of the file.
        assert_eq!(decode(b"i42ei43e").unwrap(), Value::Int(42));
        assert_eq!(decode(b"4:spamX").unwrap(), Value::Bytes(b"spam".to_vec()));

        // And the span is still the info dictionary alone.
        let file = b"d4:infod6:lengthi1eee-------- appended --------";
        let (_, span) = decode_with_span(file, "info").expect("a torrent with padding");
        let span = span.expect("the info span");
        assert_eq!(&file[span], b"d6:lengthi1ee");
    }
}
