// SPDX-License-Identifier: GPL-3.0-or-later
//! The value model rencode can carry.

use std::fmt;

/// Anything that can cross the Deluge RPC wire.
///
/// This mirrors what the Python side can encode, which is why it has both
/// [`Value::Bytes`] and [`Value::Str`], and why dictionaries keep insertion
/// order and allow any value as a key. Python dictionaries are ordered and
/// rencode preserves that, so a map type here would quietly reorder traffic.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    /// An integer that fits in 64 bits, which covers everything Deluge sends.
    Int(i64),
    /// An integer too large for 64 bits, kept as its decimal text.
    ///
    /// rencode writes these as a decimal string, and Python has no width limit.
    /// Keeping the text avoids pulling in a bignum crate for values no part of
    /// Deluge produces but any peer could send.
    BigInt(String),
    Float32(f32),
    Float64(f64),
    /// A string whose bytes are not valid UTF-8, or one decoded in raw mode.
    Bytes(Vec<u8>),
    Str(String),
    List(Vec<Value>),
    /// Ordered key/value pairs. Keys may be any value, as rencode allows.
    Dict(Vec<(Value, Value)>),
}

impl Value {
    /// Looks up a string key in a dictionary, matching `Str` and `Bytes` alike.
    ///
    /// The daemon is inconsistent about which it sends, so callers should not
    /// have to be.
    pub fn get(&self, key: &str) -> Option<&Value> {
        let Value::Dict(entries) = self else {
            return None;
        };
        entries
            .iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(_, v)| v)
    }

    /// The text of a `Str`, or of `Bytes` that happen to be valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text),
            Self::Bytes(raw) => std::str::from_utf8(raw).ok(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// A short name for the variant, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::BigInt(_) => "bigint",
            Self::Float32(_) => "float32",
            Self::Float64(_) => "float64",
            Self::Bytes(_) => "bytes",
            Self::Str(_) => "str",
            Self::List(_) => "list",
            Self::Dict(_) => "dict",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Int(value) => write!(f, "{value}"),
            Self::BigInt(value) => write!(f, "{value}"),
            Self::Float32(value) => write!(f, "{value}"),
            Self::Float64(value) => write!(f, "{value}"),
            Self::Bytes(raw) => write!(f, "<{} bytes>", raw.len()),
            Self::Str(text) => write!(f, "{text:?}"),
            Self::List(items) => {
                write!(f, "[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Self::Dict(entries) => {
                write!(f, "{{")?;
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{key}: {value}")?;
                }
                write!(f, "}}")
            }
        }
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Self::List(value)
    }
}
