// SPDX-License-Identifier: GPL-3.0-or-later
//! Encoding.
//!
//! Every value has exactly one encoding: the shortest form that fits. The
//! reference implementation works the same way, so a round trip through Python
//! and back is byte-identical, which the conformance tests rely on.

use crate::format::*;
use crate::value::Value;

/// Encodes one value.
pub fn to_vec(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(value, &mut out);
    out
}

fn write(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::None => out.push(CHR_NONE),
        Value::Bool(true) => out.push(CHR_TRUE),
        Value::Bool(false) => out.push(CHR_FALSE),
        Value::Int(number) => write_int(*number, out),
        Value::BigInt(text) => {
            out.push(CHR_INT);
            out.extend_from_slice(text.as_bytes());
            out.push(CHR_TERM);
        }
        Value::Float32(number) => {
            out.push(CHR_FLOAT32);
            out.extend_from_slice(&number.to_be_bytes());
        }
        Value::Float64(number) => {
            out.push(CHR_FLOAT64);
            out.extend_from_slice(&number.to_be_bytes());
        }
        Value::Bytes(raw) => write_string(raw, out),
        Value::Str(text) => write_string(text.as_bytes(), out),
        Value::List(items) => {
            if items.len() < usize::from(LIST_FIXED_COUNT) {
                out.push(LIST_FIXED_START + items.len() as u8);
                for item in items {
                    write(item, out);
                }
            } else {
                out.push(CHR_LIST);
                for item in items {
                    write(item, out);
                }
                out.push(CHR_TERM);
            }
        }
        Value::Dict(entries) => {
            if entries.len() < usize::from(DICT_FIXED_COUNT) {
                out.push(DICT_FIXED_START + entries.len() as u8);
                for (key, value) in entries {
                    write(key, out);
                    write(value, out);
                }
            } else {
                out.push(CHR_DICT);
                for (key, value) in entries {
                    write(key, out);
                    write(value, out);
                }
                out.push(CHR_TERM);
            }
        }
    }
}

fn write_int(number: i64, out: &mut Vec<u8>) {
    if (0..i64::from(INT_POS_FIXED_COUNT)).contains(&number) {
        out.push(INT_POS_FIXED_START + number as u8);
    } else if (-i64::from(INT_NEG_FIXED_COUNT)..0).contains(&number) {
        out.push(INT_NEG_FIXED_START + (-1 - number) as u8);
    } else if let Ok(narrow) = i8::try_from(number) {
        out.push(CHR_INT1);
        out.extend_from_slice(&narrow.to_be_bytes());
    } else if let Ok(narrow) = i16::try_from(number) {
        out.push(CHR_INT2);
        out.extend_from_slice(&narrow.to_be_bytes());
    } else if let Ok(narrow) = i32::try_from(number) {
        out.push(CHR_INT4);
        out.extend_from_slice(&narrow.to_be_bytes());
    } else {
        out.push(CHR_INT8);
        out.extend_from_slice(&number.to_be_bytes());
    }
}

fn write_string(raw: &[u8], out: &mut Vec<u8>) {
    if raw.len() < usize::from(STR_FIXED_COUNT) {
        out.push(STR_FIXED_START + raw.len() as u8);
        out.extend_from_slice(raw);
    } else {
        out.extend_from_slice(raw.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(raw);
    }
}
