// SPDX-License-Identifier: GPL-3.0-or-later
//! The four message shapes DelugeRPC carries.
//!
//! Transcribed from `deluge/core/rpcserver.py` and `deluge/ui/client.py`. The
//! numbering is part of the wire contract.

use redeluge_rencode::Value;

/// Message type identifiers, as they appear in the first element.
pub const RPC_RESPONSE: i64 = 1;
pub const RPC_ERROR: i64 = 2;
pub const RPC_EVENT: i64 = 3;

/// A call, from client to daemon.
///
/// Requests travel in a list, so several can share one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Chosen by the client; the daemon echoes it so replies can be matched to
    /// calls that may complete out of order.
    pub id: i64,
    pub method: String,
    pub args: Vec<Value>,
    pub kwargs: Vec<(Value, Value)>,
}

impl Request {
    pub fn new(id: i64, method: impl Into<String>) -> Self {
        Self {
            id,
            method: method.into(),
            args: Vec::new(),
            kwargs: Vec::new(),
        }
    }

    pub fn arg(mut self, value: impl Into<Value>) -> Self {
        self.args.push(value.into());
        self
    }

    pub fn kwarg(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.kwargs
            .push((Value::Str(name.to_owned()), value.into()));
        self
    }

    fn to_value(&self) -> Value {
        Value::List(vec![
            Value::Int(self.id),
            Value::Str(self.method.clone()),
            Value::List(self.args.clone()),
            Value::Dict(self.kwargs.clone()),
        ])
    }
}

/// One frame's worth of requests.
pub fn requests_to_value(requests: &[Request]) -> Value {
    Value::List(requests.iter().map(Request::to_value).collect())
}

/// Anything the daemon sends back.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A successful reply to the request with this id.
    Response { id: i64, result: Value },
    /// A failed reply. The daemon also sends a traceback, which is deliberately
    /// not kept: see [`Failure`].
    Error { id: i64, failure: Failure },
    /// An unsolicited event.
    Event { name: String, args: Vec<Value> },
}

/// A remote failure.
///
/// The Python daemon puts a full traceback in the sixth field, and sends it to
/// unauthenticated clients on a failed login, leaking paths and versions. It is
/// parsed here so the field is accounted for, and kept out of [`Display`] so it
/// cannot reach a user by accident.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    /// The Python exception class, e.g. `BadLoginError`.
    pub exception: String,
    pub args: Vec<Value>,
    pub kwargs: Vec<(Value, Value)>,
    /// The remote traceback, if the daemon sent one. Never displayed.
    pub traceback: Option<String>,
}

impl Failure {
    /// The first argument as text, which is where the daemon puts its message.
    pub fn message(&self) -> Option<&str> {
        self.args.first().and_then(Value::as_str)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.message() {
            Some(message) => write!(f, "{}: {message}", self.exception),
            None => write!(f, "{}", self.exception),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("malformed message from the daemon: {reason}")]
pub struct MalformedMessage {
    pub reason: String,
}

fn malformed(reason: impl Into<String>) -> MalformedMessage {
    MalformedMessage {
        reason: reason.into(),
    }
}

impl Incoming {
    /// Reads one decoded message.
    pub fn from_value(value: &Value) -> Result<Self, MalformedMessage> {
        let items = value
            .as_list()
            .ok_or_else(|| malformed(format!("expected a list, got {}", value.type_name())))?;

        let kind = items
            .first()
            .and_then(Value::as_i64)
            .ok_or_else(|| malformed("no message type"))?;

        match kind {
            RPC_RESPONSE => {
                let id = items
                    .get(1)
                    .and_then(Value::as_i64)
                    .ok_or_else(|| malformed("response has no request id"))?;
                // The daemon wraps the return value in a one-element list.
                let result = match items.get(2) {
                    Some(Value::List(values)) if values.len() == 1 => values[0].clone(),
                    Some(other) => other.clone(),
                    None => Value::None,
                };
                Ok(Self::Response { id, result })
            }
            RPC_ERROR => {
                let id = items
                    .get(1)
                    .and_then(Value::as_i64)
                    .ok_or_else(|| malformed("error has no request id"))?;
                let exception = items
                    .get(2)
                    .and_then(Value::as_str)
                    .unwrap_or("UnknownError")
                    .to_owned();
                let args = items
                    .get(3)
                    .and_then(Value::as_list)
                    .map(<[Value]>::to_vec)
                    .unwrap_or_default();
                let kwargs = match items.get(4) {
                    Some(Value::Dict(entries)) => entries.clone(),
                    _ => Vec::new(),
                };
                let traceback = items
                    .get(5)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .filter(|text| !text.is_empty());

                Ok(Self::Error {
                    id,
                    failure: Failure {
                        exception,
                        args,
                        kwargs,
                        traceback,
                    },
                })
            }
            RPC_EVENT => {
                let name = items
                    .get(1)
                    .and_then(Value::as_str)
                    .ok_or_else(|| malformed("event has no name"))?
                    .to_owned();
                let args = items
                    .get(2)
                    .and_then(Value::as_list)
                    .map(<[Value]>::to_vec)
                    .unwrap_or_default();
                Ok(Self::Event { name, args })
            }
            other => Err(malformed(format!("unknown message type {other}"))),
        }
    }
}
