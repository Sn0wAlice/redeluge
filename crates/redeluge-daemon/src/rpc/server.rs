// SPDX-License-Identifier: GPL-3.0-or-later
//! The listener itself.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use redeluge_rencode::Value;
use redeluge_rpc::message::{RPC_ERROR, RPC_EVENT, RPC_RESPONSE};
use redeluge_rpc::transfer::{encode_frame, FrameReader, Limits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_rustls::TlsAcceptor;

use crate::auth::AuthLevel;
use crate::events::Event;
use crate::rpc::dispatch::{CallContext, Rpc, RpcError};

/// How the listener is set up.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub address: SocketAddr,
    pub limits: Limits,
    /// Events buffered per connection before the slowest subscriber starts
    /// missing them. A client that stops reading must not stall the daemon.
    pub event_buffer: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1:58846".parse().expect("valid literal"),
            limits: Limits::default(),
            event_buffer: 1024,
        }
    }
}

/// The running listener.
pub struct Server {
    listener: TcpListener,
    acceptor: TlsAcceptor,
    config: ServerConfig,
    events: broadcast::Sender<(Option<i64>, Event)>,
    next_session: AtomicI64,
}

impl Server {
    pub async fn bind(config: ServerConfig, acceptor: TlsAcceptor) -> std::io::Result<Arc<Self>> {
        let listener = TcpListener::bind(config.address).await?;
        let bound = listener.local_addr()?;
        let (events, _) = broadcast::channel(config.event_buffer);

        tracing::info!(%bound, "DelugeRPC listening");
        Ok(Arc::new(Self {
            listener,
            acceptor,
            config,
            events,
            next_session: AtomicI64::new(1),
        }))
    }

    /// The address actually bound, which matters when the port was 0.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Broadcasts an event to every interested session.
    pub fn broadcast(&self, event: Event) {
        // No subscribers is not an error; the daemon runs fine with no clients.
        let _ = self.events.send((None, event));
    }

    /// Sends an event to one session only.
    pub fn send_to(&self, session_id: i64, event: Event) {
        let _ = self.events.send((Some(session_id), event));
    }

    /// Accepts connections until the future is dropped.
    pub async fn serve<H: Rpc>(self: Arc<Self>, handler: Arc<H>) {
        loop {
            let (stream, peer) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                Err(err) => {
                    // A per-connection failure must not take the listener down.
                    tracing::warn!(error = %err, "accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            };
            let _ = stream.set_nodelay(true);

            let session_id = self.next_session.fetch_add(1, Ordering::Relaxed);
            let server = Arc::clone(&self);
            let handler = Arc::clone(&handler);

            tokio::spawn(async move {
                if let Err(err) = server
                    .handle_connection(stream, peer, session_id, handler.as_ref())
                    .await
                {
                    tracing::debug!(session_id, %peer, error = %err, "connection ended");
                }
                handler.disconnected(session_id).await;
                server.broadcast(Event::ClientDisconnected { session_id });
            });
        }
    }

    async fn handle_connection<H: Rpc>(
        &self,
        stream: tokio::net::TcpStream,
        peer: SocketAddr,
        session_id: i64,
        handler: &H,
    ) -> std::io::Result<()> {
        let stream = self.acceptor.accept(stream).await?;
        tracing::debug!(session_id, %peer, "client connected");

        let (mut reader, mut writer) = tokio::io::split(stream);
        let mut frames = FrameReader::new(self.config.limits);
        let mut chunk = vec![0u8; 16 * 1024];

        let mut context = CallContext {
            session_id,
            username: String::new(),
            level: AuthLevel::None,
            peer: peer.to_string(),
        };
        let mut events = self.events.subscribe();
        let mut interest: Option<Vec<String>> = None;

        loop {
            tokio::select! {
                read = reader.read(&mut chunk) => {
                    let count = match read {
                        Ok(0) => return Ok(()),
                        Ok(count) => count,
                        Err(err) => return Err(err),
                    };
                    frames.feed(&chunk[..count]);

                    loop {
                        let message = match frames.next_message() {
                            Ok(None) => break,
                            Ok(Some(message)) => message,
                            Err(err) => {
                                // Framing is broken. The stream cannot be
                                // resynchronised, so the connection ends.
                                tracing::warn!(session_id, %peer, error = %err,
                                    "dropping a client that is not speaking DelugeRPC");
                                return Ok(());
                            }
                        };

                        for reply in self
                            .handle_message(&message, &mut context, &mut interest, handler)
                            .await
                        {
                            writer.write_all(&reply).await?;
                        }
                    }
                }

                event = events.recv() => {
                    match event {
                        Ok((target, event)) => {
                            // Events only reach an authenticated session that
                            // asked for them.
                            if context.level == AuthLevel::None {
                                continue;
                            }
                            if let Some(only) = target {
                                if only != session_id {
                                    continue;
                                }
                            }
                            if !wants(&interest, event.name()) {
                                continue;
                            }

                            let frame = encode_frame(&Value::List(vec![
                                Value::Int(RPC_EVENT),
                                Value::Str(event.name().to_owned()),
                                Value::List(event.args()),
                            ]));
                            if let Ok(frame) = frame {
                                writer.write_all(&frame).await?;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(missed)) => {
                            // The client is not keeping up. Telling it is not
                            // possible, so this is recorded and the daemon
                            // carries on rather than stalling for one slow peer.
                            tracing::warn!(session_id, missed, "client fell behind on events");
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        }
    }

    /// Answers one frame, which may hold several calls.
    async fn handle_message<H: Rpc>(
        &self,
        message: &Value,
        context: &mut CallContext,
        interest: &mut Option<Vec<String>>,
        handler: &H,
    ) -> Vec<Vec<u8>> {
        let Some(requests) = message.as_list() else {
            tracing::debug!("ignoring a message that is not a list of requests");
            return Vec::new();
        };

        let mut replies = Vec::new();
        for request in requests {
            let Some(fields) = request.as_list() else {
                continue;
            };
            let Some(id) = fields.first().and_then(Value::as_i64) else {
                continue;
            };
            let Some(method) = fields.get(1).and_then(Value::as_str) else {
                continue;
            };
            let args = fields
                .get(2)
                .and_then(Value::as_list)
                .map(<[Value]>::to_vec)
                .unwrap_or_default();
            let kwargs = match fields.get(3) {
                Some(Value::Dict(entries)) => entries.clone(),
                _ => Vec::new(),
            };

            let outcome = self
                .run(method, args, kwargs, context, interest, handler)
                .await;

            let frame = match outcome {
                Ok(value) => encode_frame(&Value::List(vec![
                    Value::Int(RPC_RESPONSE),
                    Value::Int(id),
                    // The daemon wraps the return value in a one-element list.
                    Value::List(vec![value]),
                ])),
                Err(error) => {
                    let mut args = vec![Value::Str(error.message.clone())];
                    args.extend(error.args.clone());
                    encode_frame(&Value::List(vec![
                        Value::Int(RPC_ERROR),
                        Value::Int(id),
                        Value::Str(error.exception.clone()),
                        Value::List(args),
                        Value::Dict(Vec::new()),
                        // Where the Python daemon puts a traceback. Leaving it
                        // empty is deliberate: it named paths and versions, and
                        // it went to unauthenticated clients.
                        Value::Str(String::new()),
                    ]))
                }
            };

            match frame {
                Ok(frame) => replies.push(frame),
                Err(err) => tracing::error!(error = %err, "could not encode a reply"),
            }
        }
        replies
    }

    async fn run<H: Rpc>(
        &self,
        method: &str,
        args: Vec<Value>,
        kwargs: Vec<(Value, Value)>,
        context: &mut CallContext,
        interest: &mut Option<Vec<String>>,
        handler: &H,
    ) -> Result<Value, RpcError> {
        // Two calls answer before authentication, as they do in the Python
        // daemon: the version, so a client knows whether it can speak to this
        // daemon, and the login itself.
        match method {
            "daemon.info" => return Ok(Value::Str(handler.version())),
            "daemon.login" => {
                let username = args
                    .first()
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::bad_login("a username is required"))?;
                let password = args
                    .get(1)
                    .and_then(Value::as_str)
                    .ok_or_else(|| RpcError::bad_login("a password is required"))?;

                // The Python daemon refuses a login with no client_version, and
                // clients depend on that refusal to detect an old daemon.
                let has_version = kwargs
                    .iter()
                    .any(|(key, _)| key.as_str() == Some("client_version"));
                if !has_version {
                    return Err(RpcError::new(
                        "IncompatibleClient",
                        format!(
                            "Your deluge client is not compatible with the daemon. \
                                 Please upgrade your client to {}",
                            handler.version()
                        ),
                    ));
                }

                let level = handler.authenticate(username, password).await?;
                context.username = username.to_owned();
                context.level = level;
                tracing::info!(
                    session_id = context.session_id,
                    username,
                    level = level.as_i64(),
                    "client authenticated"
                );
                return Ok(Value::Int(level.as_i64()));
            }
            _ => {}
        }

        let required = handler
            .auth_level(method)
            .ok_or_else(|| RpcError::unknown_method(method))?;

        if context.level < required {
            if context.level == AuthLevel::None {
                tracing::debug!(
                    peer = %context.peer,
                    method,
                    "call from an unauthenticated client"
                );
            }
            return Err(RpcError::not_authorized(required, context.level));
        }

        // Two more the listener owns, because they are about the connection
        // rather than about torrents.
        match method {
            "daemon.get_method_list" => {
                return Ok(Value::List(
                    handler.method_list().into_iter().map(Value::Str).collect(),
                ))
            }
            "daemon.set_event_interest" => {
                let events: Vec<String> = args
                    .first()
                    .and_then(Value::as_list)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                *interest = Some(events.clone());
                handler.set_event_interest(context.session_id, events).await;
                return Ok(Value::Bool(true));
            }
            _ => {}
        }

        handler.call(context, method, args, kwargs).await
    }
}

/// Whether a session asked for this event.
///
/// No call to `set_event_interest` means nothing has been asked for, which is
/// how the Python daemon behaves: a client gets what it subscribes to. An
/// empty list means everything, which is what the Web UI sends.
fn wants(interest: &Option<Vec<String>>, name: &str) -> bool {
    match interest {
        None => false,
        Some(names) if names.is_empty() => true,
        Some(names) => names.iter().any(|wanted| wanted == name),
    }
}
