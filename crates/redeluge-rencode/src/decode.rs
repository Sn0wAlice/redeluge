// SPDX-License-Identifier: GPL-3.0-or-later
//! Decoding, written defensively.
//!
//! Everything here runs on bytes an unauthenticated peer chose. The decoder
//! never indexes without checking, never allocates a container to a length it
//! has not yet seen bytes for, and bounds its own recursion.

use crate::error::{Error, Result, StringMode};
use crate::format::*;
use crate::value::Value;

/// Deepest nesting accepted. Deluge's own traffic is a handful of levels; this
/// is generous while still stopping a message of nothing but list markers from
/// overflowing the stack.
pub const MAX_DEPTH: usize = 64;

/// Longest string or largest container the decoder will accept.
///
/// The reference implementation has no such limit, so a four-byte length field
/// can make it reserve gigabytes. The cap is far above any real Deluge message.
pub const MAX_LENGTH: u64 = 64 * 1024 * 1024;

/// Decodes one complete value, rejecting anything left over.
pub fn from_slice(input: &[u8], mode: StringMode) -> Result<Value> {
    let mut decoder = Decoder {
        input,
        offset: 0,
        mode,
    };
    let value = decoder.value(0)?;
    if decoder.offset != input.len() {
        return Err(Error::TrailingBytes {
            extra: input.len() - decoder.offset,
        });
    }
    Ok(value)
}

/// Decodes one value and reports how many bytes it used.
///
/// Useful when a value is embedded in a larger buffer; [`from_slice`] is the
/// one to reach for otherwise.
pub fn from_slice_prefix(input: &[u8], mode: StringMode) -> Result<(Value, usize)> {
    let mut decoder = Decoder {
        input,
        offset: 0,
        mode,
    };
    let value = decoder.value(0)?;
    Ok((value, decoder.offset))
}

struct Decoder<'a> {
    input: &'a [u8],
    offset: usize,
    mode: StringMode,
}

impl<'a> Decoder<'a> {
    fn need(&self, count: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(count).ok_or(Error::Truncated {
            offset: self.offset,
            needed: count,
        })?;
        self.input.get(self.offset..end).ok_or(Error::Truncated {
            offset: self.offset,
            needed: end - self.input.len().min(end),
        })
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let slice = self.need(count)?;
        self.offset += count;
        Ok(slice)
    }

    fn peek(&self) -> Result<u8> {
        self.input
            .get(self.offset)
            .copied()
            .ok_or(Error::Truncated {
                offset: self.offset,
                needed: 1,
            })
    }

    fn check_length(&self, length: u64) -> Result<usize> {
        if length > MAX_LENGTH {
            return Err(Error::LengthLimit {
                offset: self.offset,
                length,
                limit: MAX_LENGTH,
            });
        }
        Ok(length as usize)
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(Error::TooDeep {
                offset: self.offset,
                limit: MAX_DEPTH,
            });
        }

        let code = self.peek()?;

        // Ranges first, exact codes after. The ranges are disjoint and the
        // exact codes all fall in the gaps between them.
        if code < INT_POS_FIXED_COUNT {
            self.offset += 1;
            return Ok(Value::Int(i64::from(code - INT_POS_FIXED_START)));
        }
        if (INT_NEG_FIXED_START..INT_NEG_FIXED_START + INT_NEG_FIXED_COUNT).contains(&code) {
            self.offset += 1;
            return Ok(Value::Int(-1 - i64::from(code - INT_NEG_FIXED_START)));
        }
        if (STR_FIXED_START..STR_FIXED_START + STR_FIXED_COUNT).contains(&code) {
            self.offset += 1;
            let length = usize::from(code - STR_FIXED_START);
            let raw = self.take(length)?;
            return Ok(self.string(raw));
        }
        if (LIST_FIXED_START..).contains(&code) && code >= LIST_FIXED_START {
            self.offset += 1;
            let count = usize::from(code - LIST_FIXED_START);
            let mut items = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                items.push(self.value(depth + 1)?);
            }
            return Ok(Value::List(items));
        }
        if (DICT_FIXED_START..DICT_FIXED_START + DICT_FIXED_COUNT).contains(&code) {
            self.offset += 1;
            let count = usize::from(code - DICT_FIXED_START);
            let mut entries = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let key = self.value(depth + 1)?;
                let value = self.value(depth + 1)?;
                entries.push((key, value));
            }
            return Ok(Value::Dict(entries));
        }
        if (DIGIT_START..=DIGIT_END).contains(&code) {
            return self.length_prefixed_string();
        }

        match code {
            CHR_NONE => {
                self.offset += 1;
                Ok(Value::None)
            }
            CHR_TRUE => {
                self.offset += 1;
                Ok(Value::Bool(true))
            }
            CHR_FALSE => {
                self.offset += 1;
                Ok(Value::Bool(false))
            }
            CHR_INT1 => {
                self.offset += 1;
                let raw = self.take(1)?;
                Ok(Value::Int(i64::from(raw[0] as i8)))
            }
            CHR_INT2 => {
                self.offset += 1;
                let raw = self.take(2)?;
                Ok(Value::Int(i64::from(i16::from_be_bytes([raw[0], raw[1]]))))
            }
            CHR_INT4 => {
                self.offset += 1;
                let raw = self.take(4)?;
                Ok(Value::Int(i64::from(i32::from_be_bytes([
                    raw[0], raw[1], raw[2], raw[3],
                ]))))
            }
            CHR_INT8 => {
                self.offset += 1;
                let raw = self.take(8)?;
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(raw);
                Ok(Value::Int(i64::from_be_bytes(bytes)))
            }
            CHR_FLOAT32 => {
                self.offset += 1;
                let raw = self.take(4)?;
                let mut bytes = [0u8; 4];
                bytes.copy_from_slice(raw);
                Ok(Value::Float32(f32::from_be_bytes(bytes)))
            }
            CHR_FLOAT64 => {
                self.offset += 1;
                let raw = self.take(8)?;
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(raw);
                Ok(Value::Float64(f64::from_be_bytes(bytes)))
            }
            CHR_INT => self.decimal_int(),
            CHR_LIST => {
                let start = self.offset;
                self.offset += 1;
                let mut items = Vec::new();
                loop {
                    match self.peek() {
                        Ok(CHR_TERM) => {
                            self.offset += 1;
                            return Ok(Value::List(items));
                        }
                        Ok(_) => items.push(self.value(depth + 1)?),
                        Err(_) => {
                            return Err(Error::Unterminated {
                                offset: start,
                                container: "list",
                            })
                        }
                    }
                }
            }
            CHR_DICT => {
                let start = self.offset;
                self.offset += 1;
                let mut entries = Vec::new();
                loop {
                    match self.peek() {
                        Ok(CHR_TERM) => {
                            self.offset += 1;
                            return Ok(Value::Dict(entries));
                        }
                        Ok(_) => {
                            let key = self.value(depth + 1)?;
                            // Distinguish "this key has no value" from "the
                            // message stopped here", because they point at
                            // different bugs on the other end.
                            match self.peek() {
                                Ok(CHR_TERM) => {
                                    return Err(Error::DanglingKey {
                                        offset: self.offset,
                                    })
                                }
                                Ok(_) => {}
                                Err(_) => {
                                    return Err(Error::Unterminated {
                                        offset: start,
                                        container: "dict",
                                    })
                                }
                            }
                            let value = self.value(depth + 1)?;
                            entries.push((key, value));
                        }
                        Err(_) => {
                            return Err(Error::Unterminated {
                                offset: start,
                                container: "dict",
                            })
                        }
                    }
                }
            }
            other => Err(Error::UnknownTypeCode {
                code: other,
                offset: self.offset,
            }),
        }
    }

    /// `CHR_INT` followed by a decimal string terminated by `CHR_TERM`.
    fn decimal_int(&mut self) -> Result<Value> {
        let start = self.offset;
        self.offset += 1;

        let body_start = self.offset;
        let end = self.input[body_start..]
            .iter()
            .position(|byte| *byte == CHR_TERM)
            .ok_or(Error::Unterminated {
                offset: start,
                container: "int",
            })?;

        if end >= MAX_INT_LENGTH {
            return Err(Error::LengthLimit {
                offset: start,
                length: end as u64,
                limit: MAX_INT_LENGTH as u64,
            });
        }

        let digits = &self.input[body_start..body_start + end];
        let text = std::str::from_utf8(digits).map_err(|_| Error::MalformedNumber {
            offset: body_start,
            reason: "not ASCII".to_owned(),
        })?;

        // The reference implementation rejects "-0" and leading zeros, so a
        // value has exactly one encoding.
        let rejected = text.is_empty()
            || text == "-"
            || text.starts_with("-0")
            || (text.starts_with('0') && text.len() > 1);
        if rejected {
            return Err(Error::MalformedNumber {
                offset: body_start,
                reason: format!("not a canonical integer: {text:?}"),
            });
        }
        if let Some(rest) = text.strip_prefix('-') {
            if !rest.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Error::MalformedNumber {
                    offset: body_start,
                    reason: format!("not a number: {text:?}"),
                });
            }
        } else if !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::MalformedNumber {
                offset: body_start,
                reason: format!("not a number: {text:?}"),
            });
        }

        self.offset = body_start + end + 1;

        // Anything that fits becomes an Int; the rest keeps its text, because
        // Python has no width limit and a peer can send more than 64 bits.
        Ok(match text.parse::<i64>() {
            Ok(value) => Value::Int(value),
            Err(_) => Value::BigInt(text.to_owned()),
        })
    }

    /// `<decimal length>:<bytes>`.
    fn length_prefixed_string(&mut self) -> Result<Value> {
        let start = self.offset;
        let colon = self.input[start..]
            .iter()
            .position(|byte| *byte == b':')
            .ok_or(Error::MalformedNumber {
                offset: start,
                reason: "string length has no terminating colon".to_owned(),
            })?;

        let digits = &self.input[start..start + colon];
        if digits.len() > 20 || !digits.iter().all(u8::is_ascii_digit) {
            return Err(Error::MalformedNumber {
                offset: start,
                reason: "string length is not a plain number".to_owned(),
            });
        }
        let text = std::str::from_utf8(digits).expect("digits are ASCII");
        let length: u64 = text.parse().map_err(|_| Error::MalformedNumber {
            offset: start,
            reason: format!("string length out of range: {text}"),
        })?;

        self.offset = start + colon + 1;
        let length = self.check_length(length)?;
        let raw = self.take(length)?;
        Ok(self.string(raw))
    }

    fn string(&self, raw: &[u8]) -> Value {
        match self.mode {
            StringMode::Raw => Value::Bytes(raw.to_vec()),
            StringMode::Utf8Lossy => match std::str::from_utf8(raw) {
                Ok(text) => Value::Str(text.to_owned()),
                Err(_) => Value::Bytes(raw.to_vec()),
            },
        }
    }
}
