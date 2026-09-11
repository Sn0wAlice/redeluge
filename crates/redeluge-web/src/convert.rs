// SPDX-License-Identifier: GPL-3.0-or-later
//! Between the browser's JSON and the daemon's rencode.
//!
//! The web server sits between two type systems that do not line up. JSON has
//! one number type; rencode has fixed-width integers and two float widths. JSON
//! object keys are strings; rencode dictionary keys can be anything. Both
//! directions are lossy somewhere, so the rules are written down here rather
//! than being rediscovered at each call site.

use redeluge_rencode::Value;
use serde_json::{Map, Number, Value as Json};

/// Browser to daemon.
///
/// Integers become `Int`, everything else with a fractional part becomes
/// `Float64`. The Python daemon receives a Python float either way, so the
/// width only affects how many bytes cross the wire.
pub fn json_to_rencode(json: &Json) -> Value {
    match json {
        Json::Null => Value::None,
        Json::Bool(value) => Value::Bool(*value),
        Json::Number(number) => number_to_rencode(number),
        Json::String(text) => Value::Str(text.clone()),
        Json::Array(items) => Value::List(items.iter().map(json_to_rencode).collect()),
        Json::Object(entries) => Value::Dict(
            entries
                .iter()
                .map(|(key, value)| (Value::Str(key.clone()), json_to_rencode(value)))
                .collect(),
        ),
    }
}

fn number_to_rencode(number: &Number) -> Value {
    if let Some(integer) = number.as_i64() {
        return Value::Int(integer);
    }
    // A u64 above i64::MAX has no fixed-width rencode form, so it goes as text,
    // which is what rencode does for large integers anyway.
    if let Some(unsigned) = number.as_u64() {
        return Value::BigInt(unsigned.to_string());
    }
    Value::Float64(number.as_f64().unwrap_or(0.0))
}

/// Daemon to browser.
///
/// Dictionary keys that are not strings are rendered as text, because JSON has
/// no other option. Deluge does not send such keys today; doing it this way
/// means a future one becomes a readable key rather than a dropped entry.
pub fn rencode_to_json(value: &Value) -> Json {
    match value {
        Value::None => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::Int(number) => Json::Number(Number::from(*number)),
        // Too large for JSON's number type in practice, so it stays text rather
        // than silently losing precision.
        Value::BigInt(text) => Json::String(text.clone()),
        Value::Float32(number) => float_to_json(f64::from(*number)),
        Value::Float64(number) => float_to_json(*number),
        Value::Str(text) => Json::String(text.clone()),
        // Bytes reach here only when they are not valid UTF-8, which for Deluge
        // means a torrent or file name from a peer. Lossy is the right call:
        // the alternative is dropping the field the user wants to see.
        Value::Bytes(raw) => Json::String(String::from_utf8_lossy(raw).into_owned()),
        Value::List(items) => Json::Array(items.iter().rencode_each()),
        Value::Dict(entries) => {
            let mut map = Map::with_capacity(entries.len());
            for (key, value) in entries {
                map.insert(key_to_string(key), rencode_to_json(value));
            }
            Json::Object(map)
        }
    }
}

/// JSON has no NaN or infinity, and a bare `null` in a numeric field breaks the
/// Web UI's arithmetic. Those become 0, which is what the field means when the
/// daemon has nothing to report.
fn float_to_json(number: f64) -> Json {
    Number::from_f64(number)
        .map(Json::Number)
        .unwrap_or_else(|| Json::Number(Number::from(0)))
}

fn key_to_string(key: &Value) -> String {
    match key {
        Value::Str(text) => text.clone(),
        Value::Bytes(raw) => String::from_utf8_lossy(raw).into_owned(),
        other => other.to_string(),
    }
}

/// Small helper so the list arm above reads in one line.
trait RencodeEach {
    fn rencode_each(self) -> Vec<Json>;
}

impl<'a, I> RencodeEach for I
where
    I: Iterator<Item = &'a Value>,
{
    fn rencode_each(self) -> Vec<Json> {
        self.map(rencode_to_json).collect()
    }
}
