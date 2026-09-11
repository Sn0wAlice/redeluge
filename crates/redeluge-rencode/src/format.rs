// SPDX-License-Identifier: GPL-3.0-or-later
//! Type codes, transcribed from the reference implementation.
//!
//! rencode has no written specification; `rencode_orig.py` is the
//! specification. These constants and ranges carve up the byte 0..=255 with no
//! gaps that matter and no overlaps, which `tests/format.rs` asserts.

/// Positive integers 0..=43, value stored in the type code itself.
pub const INT_POS_FIXED_START: u8 = 0;
pub const INT_POS_FIXED_COUNT: u8 = 44;

pub const CHR_FLOAT64: u8 = 44;
/// ASCII digits, which introduce a `<length>:<bytes>` string.
pub const DIGIT_START: u8 = b'0';
pub const DIGIT_END: u8 = b'9';

pub const CHR_LIST: u8 = 59;
pub const CHR_DICT: u8 = 60;
pub const CHR_INT: u8 = 61;
pub const CHR_INT1: u8 = 62;
pub const CHR_INT2: u8 = 63;
pub const CHR_INT4: u8 = 64;
pub const CHR_INT8: u8 = 65;
pub const CHR_FLOAT32: u8 = 66;
pub const CHR_TRUE: u8 = 67;
pub const CHR_FALSE: u8 = 68;
pub const CHR_NONE: u8 = 69;

/// Negative integers -1..=-32, value stored in the type code.
pub const INT_NEG_FIXED_START: u8 = 70;
pub const INT_NEG_FIXED_COUNT: u8 = 32;

/// Dictionaries of 0..=24 pairs, length stored in the type code.
pub const DICT_FIXED_START: u8 = 102;
pub const DICT_FIXED_COUNT: u8 = 25;

/// Ends a `CHR_LIST`, `CHR_DICT` or `CHR_INT`.
pub const CHR_TERM: u8 = 127;

/// Strings of 0..=63 bytes, length stored in the type code.
pub const STR_FIXED_START: u8 = 128;
pub const STR_FIXED_COUNT: u8 = 64;

/// Lists of 0..=63 items, length stored in the type code.
pub const LIST_FIXED_START: u8 = 192;
pub const LIST_FIXED_COUNT: u8 = 64;

/// Longest decimal integer the reference implementation will read or write.
///
/// It exists so a peer cannot make the decoder chew through a megabyte of
/// digits, and the same limit applies here.
pub const MAX_INT_LENGTH: usize = 64;
