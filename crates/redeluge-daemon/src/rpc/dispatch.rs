// SPDX-License-Identifier: GPL-3.0-or-later
//! What a connection is allowed to call, and what answers it.

use async_trait::async_trait;
use redeluge_rencode::Value;

use crate::auth::AuthLevel;

/// Who is calling, and over which connection.
#[derive(Debug, Clone)]
pub struct CallContext {
    /// Identifies the connection. Clients see it in `ClientDisconnectedEvent`.
    pub session_id: i64,
    pub username: String,
    pub level: AuthLevel,
    /// The peer's address, for logging a failed login.
    pub peer: String,
}

/// A failure the client should see.
#[derive(Debug, Clone)]
pub struct RpcError {
    /// The Python exception name clients match on, such as `BadLoginError`.
    pub exception: String,
    pub message: String,
    /// Extra positional arguments the Python exception carried. Several client
    /// errors read them, so they are part of the contract.
    pub args: Vec<Value>,
}

impl RpcError {
    pub fn new(exception: &str, message: impl Into<String>) -> Self {
        Self {
            exception: exception.to_owned(),
            message: message.into(),
            args: Vec::new(),
        }
    }

    pub fn with_args(mut self, args: Vec<Value>) -> Self {
        self.args = args;
        self
    }

    /// The method does not exist. Named as the Python daemon names it.
    pub fn unknown_method(method: &str) -> Self {
        Self::new(
            "WrappedException",
            format!("RPC call on invalid function: {method}"),
        )
    }

    pub fn not_authorized(required: AuthLevel, actual: AuthLevel) -> Self {
        Self::new(
            "NotAuthorizedError",
            format!(
                "Auth level too low: {} < {}",
                actual.as_i64(),
                required.as_i64()
            ),
        )
        .with_args(vec![
            Value::Int(actual.as_i64()),
            Value::Int(required.as_i64()),
        ])
    }

    pub fn bad_login(message: &str) -> Self {
        Self::new("BadLoginError", message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new("WrappedException", message)
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.exception, self.message)
    }
}

/// What the listener calls into.
///
/// The listener knows nothing about torrents; it frames, authenticates and
/// routes. Everything else is behind this.
#[async_trait]
pub trait Rpc: Send + Sync + 'static {
    /// The level a method needs, or None when there is no such method.
    fn auth_level(&self, method: &str) -> Option<AuthLevel>;

    /// Every method this daemon exposes, for `daemon.get_method_list`.
    fn method_list(&self) -> Vec<String>;

    /// Runs a call that has already been authorised.
    async fn call(
        &self,
        context: &CallContext,
        method: &str,
        args: Vec<Value>,
        kwargs: Vec<(Value, Value)>,
    ) -> Result<Value, RpcError>;

    /// Checks a username and password, returning the level it grants.
    async fn authenticate(&self, username: &str, password: &str) -> Result<AuthLevel, RpcError>;

    /// The daemon's version string, answered before authentication.
    fn version(&self) -> String;

    /// Told when a connection goes away, so per-session state can be dropped.
    async fn disconnected(&self, session_id: i64);

    /// Told which events a session wants. An empty list means all of them.
    async fn set_event_interest(&self, session_id: i64, events: Vec<String>);
}
