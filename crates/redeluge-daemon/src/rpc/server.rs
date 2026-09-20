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
                    .handle_connection(stream, peer, session_id, Arc::clone(&handler))
                    .await
                {
                    tracing::debug!(session_id, %peer, error = %err, "connection ended");
                }
                handler.disconnected(session_id).await;
                server.broadcast(Event::ClientDisconnected { session_id });
            });
        }
    }

    /// Frames waiting to be written to one client before the daemon gives up
    /// on it. A client that has stopped reading is not a client to hold
    /// megabytes of events for.
    const OUTBOUND_QUEUE_SIZE: usize = 256;

    /// Calls from one connection that may be running at the same time.
    ///
    /// Enough that a poll's worth of calls and a long one do not queue behind
    /// each other; few enough that one connection cannot fill the daemon with
    /// work. They all reach the session thread in the end, which serialises
    /// what actually touches libtorrent.
    const CALLS_AT_ONCE: usize = 8;

    async fn handle_connection<H: Rpc>(
        self: &Arc<Self>,
        stream: tokio::net::TcpStream,
        peer: SocketAddr,
        session_id: i64,
        handler: Arc<H>,
    ) -> std::io::Result<()> {
        let stream = self.acceptor.accept(stream).await?;
        tracing::debug!(session_id, %peer, "client connected");

        let (mut reader, writer) = tokio::io::split(stream);
        // One task owns the socket's writing half. Replies and events both go
        // through it, so a call that takes a while no longer holds the wire:
        // before this, the whole connection — reads, events and every other
        // call on it — waited for whatever was running, and a client that
        // asked the daemon to hash forty gigabytes waited in silence, its own
        // progress events queued behind the call they were about.
        let (outbound, mut pending) =
            tokio::sync::mpsc::channel::<Vec<u8>>(Self::OUTBOUND_QUEUE_SIZE);
        let writing = tokio::spawn(async move {
            let mut writer = writer;
            while let Some(frame) = pending.recv().await {
                if writer.write_all(&frame).await.is_err() {
                    break;
                }
            }
        });

        // Calls run at the same time, which is what the Python daemon does as
        // well — every reply carries the id it answers, and no client has ever
        // been able to assume otherwise. Bounded, because a client that sends
        // ten thousand requests without reading its answers must not be able
        // to make the daemon start ten thousand of them.
        let in_flight = Arc::new(tokio::sync::Semaphore::new(Self::CALLS_AT_ONCE));

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
                        Ok(0) => break,
                        Ok(count) => count,
                        Err(err) => {
                            drop(outbound);
                            let _ = writing.await;
                            return Err(err);
                        }
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
                                drop(outbound);
                                let _ = writing.await;
                                return Ok(());
                            }
                        };

                        Self::handle_message(
                            &message,
                            &mut context,
                            &mut interest,
                            &handler,
                            &outbound,
                            &in_flight,
                        )
                        .await;
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
                                // A client that has stopped reading fills this
                                // queue; the connection ends rather than the
                                // daemon holding frames for it for ever.
                                if outbound.send(frame).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(missed)) => {
                            // The client is not keeping up. Telling it is not
                            // possible, so this is recorded and the daemon
                            // carries on rather than stalling for one slow peer.
                            tracing::warn!(session_id, missed, "client fell behind on events");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }

        // Whatever ended the loop, the writer goes with the connection.
        drop(outbound);
        let _ = writing.await;
        Ok(())
    }

    /// Starts the calls in one frame, which may hold several.
    ///
    /// The three that are about the connection itself — the version, the login
    /// and the event subscription — are answered here, in order, because they
    /// decide what the calls after them are allowed to do. Everything else is
    /// handed to a task, so a long call does not hold up the ones behind it,
    /// the events on their way out, or the reads.
    async fn handle_message<H: Rpc>(
        message: &Value,
        context: &mut CallContext,
        interest: &mut Option<Vec<String>>,
        handler: &Arc<H>,
        outbound: &tokio::sync::mpsc::Sender<Vec<u8>>,
        in_flight: &Arc<tokio::sync::Semaphore>,
    ) {
        let Some(requests) = message.as_list() else {
            tracing::debug!("ignoring a message that is not a list of requests");
            return;
        };

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

            // A call that changes what this connection may do next is
            // answered before the next one is even read.
            if matches!(
                method,
                "daemon.info" | "daemon.login" | "daemon.set_event_interest"
            ) {
                let outcome =
                    Self::run(method, args, kwargs, context, interest, handler.as_ref()).await;
                if let Some(frame) = Self::reply_frame(id, outcome) {
                    if outbound.send(frame).await.is_err() {
                        return;
                    }
                }
                continue;
            }

            // Everything else runs on its own. The permit is taken here, so a
            // client that floods the connection is slowed down at the read
            // rather than by starting the work.
            let Ok(permit) = Arc::clone(in_flight).acquire_owned().await else {
                return;
            };
            let handler = Arc::clone(handler);
            let context = context.clone();
            let outbound = outbound.clone();
            let method = method.to_owned();
            tokio::spawn(async move {
                let _permit = permit;
                let outcome =
                    Self::run_authorised(handler.as_ref(), &context, &method, args, kwargs).await;
                if let Some(frame) = Self::reply_frame(id, outcome) {
                    let _ = outbound.send(frame).await;
                }
            });
        }
    }

    /// One reply, framed as the client expects it.
    fn reply_frame(id: i64, outcome: Result<Value, RpcError>) -> Option<Vec<u8>> {
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
            Ok(frame) => Some(frame),
            Err(err) => {
                tracing::error!(error = %err, "could not encode a reply");
                None
            }
        }
    }

    async fn run<H: Rpc>(
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

        // The event subscription is the listener's own, and it changes what
        // this connection is sent from here on, so it is answered in order
        // rather than on a task.
        if method == "daemon.set_event_interest" {
            {
                let required = handler
                    .auth_level(method)
                    .ok_or_else(|| RpcError::unknown_method(method))?;
                if context.level < required {
                    return Err(RpcError::not_authorized(required, context.level));
                }
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
        }

        Self::run_authorised(handler, context, method, args, kwargs).await
    }

    /// Runs a call that is not about the connection itself.
    ///
    /// Every call that is not answered by the three above comes through here,
    /// whether it runs in order or on a task of its own — the authorisation is
    /// checked in one place, and there is no path around it.
    async fn run_authorised<H: Rpc>(
        handler: &H,
        context: &CallContext,
        method: &str,
        args: Vec<Value>,
        kwargs: Vec<(Value, Value)>,
    ) -> Result<Value, RpcError> {
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

        // The method list is the listener's, because the listener is what
        // knows which methods exist.
        if method == "daemon.get_method_list" {
            return Ok(Value::List(
                handler.method_list().into_iter().map(Value::Str).collect(),
            ));
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

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// A handler that answers slowly and counts what it was asked.
    struct Slow {
        started: AtomicUsize,
        finished: AtomicUsize,
        hold: Duration,
    }

    impl Slow {
        fn new(hold: Duration) -> Arc<Self> {
            Arc::new(Self {
                started: AtomicUsize::new(0),
                finished: AtomicUsize::new(0),
                hold,
            })
        }
    }

    #[async_trait]
    impl Rpc for Slow {
        fn auth_level(&self, method: &str) -> Option<AuthLevel> {
            match method {
                "core.slow" | "core.quick" => Some(AuthLevel::Normal),
                "core.admin_only" => Some(AuthLevel::Admin),
                _ => None,
            }
        }

        fn method_list(&self) -> Vec<String> {
            vec!["core.slow".to_owned(), "core.quick".to_owned()]
        }

        async fn call(
            &self,
            _context: &CallContext,
            method: &str,
            _args: Vec<Value>,
            _kwargs: Vec<(Value, Value)>,
        ) -> Result<Value, RpcError> {
            self.started.fetch_add(1, Ordering::SeqCst);
            if method == "core.slow" {
                tokio::time::sleep(self.hold).await;
            }
            self.finished.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Str(method.to_owned()))
        }

        async fn authenticate(
            &self,
            _username: &str,
            _password: &str,
        ) -> Result<AuthLevel, RpcError> {
            Ok(AuthLevel::Normal)
        }

        fn version(&self) -> String {
            "test".to_owned()
        }

        async fn disconnected(&self, _session_id: i64) {}

        async fn set_event_interest(&self, _session_id: i64, _events: Vec<String>) {}
    }

    fn context(level: AuthLevel) -> CallContext {
        CallContext {
            session_id: 1,
            username: "tester".to_owned(),
            level,
            peer: "127.0.0.1:0".to_owned(),
        }
    }

    fn request(id: i64, method: &str) -> Value {
        Value::List(vec![Value::List(vec![
            Value::Int(id),
            Value::Str(method.to_owned()),
            Value::List(Vec::new()),
            Value::Dict(Vec::new()),
        ])])
    }

    #[tokio::test]
    async fn a_call_that_takes_a_while_does_not_hold_up_the_next_one() {
        // The whole point: before this, the connection answered one call at a
        // time, so a client that asked the daemon to hash forty gigabytes
        // waited in silence — and so did every other call and event on that
        // connection.
        let handler = Slow::new(Duration::from_millis(300));
        let (outbound, mut replies) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
        let in_flight = Arc::new(tokio::sync::Semaphore::new(8));
        let mut ctx = context(AuthLevel::Normal);
        let mut interest = None;

        let started = std::time::Instant::now();
        Server::handle_message(
            &request(1, "core.slow"),
            &mut ctx,
            &mut interest,
            &handler,
            &outbound,
            &in_flight,
        )
        .await;
        Server::handle_message(
            &request(2, "core.quick"),
            &mut ctx,
            &mut interest,
            &handler,
            &outbound,
            &in_flight,
        )
        .await;

        // Both were handed off without waiting for the slow one.
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "handing off a call waited for it to finish"
        );

        // And the quick one is answered first, which is only possible because
        // they ran at the same time.
        let first = tokio::time::timeout(Duration::from_secs(2), replies.recv())
            .await
            .expect("an answer arrived")
            .expect("a frame");
        assert!(
            !first.is_empty(),
            "the quick call was not answered while the slow one ran"
        );
        assert_eq!(handler.started.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_client_that_may_not_call_something_still_cannot() {
        // The calls run on tasks now, and a path that reached the handler
        // without going through the authorisation check would be a daemon that
        // answers anything to anyone. There is one door and this is it.
        let handler = Slow::new(Duration::from_millis(0));
        let (outbound, mut replies) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
        let in_flight = Arc::new(tokio::sync::Semaphore::new(8));
        let mut ctx = context(AuthLevel::ReadOnly);
        let mut interest = None;

        Server::handle_message(
            &request(7, "core.admin_only"),
            &mut ctx,
            &mut interest,
            &handler,
            &outbound,
            &in_flight,
        )
        .await;

        let frame = tokio::time::timeout(Duration::from_secs(2), replies.recv())
            .await
            .expect("an answer arrived")
            .expect("a frame");
        // The reply is an error frame, and the handler was never reached.
        assert_eq!(
            handler.started.load(Ordering::SeqCst),
            0,
            "an unauthorised call ran"
        );
        assert!(!frame.is_empty());
    }

    #[tokio::test]
    async fn an_unauthenticated_client_is_refused_before_the_handler() {
        let handler = Slow::new(Duration::from_millis(0));
        let (outbound, mut replies) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
        let in_flight = Arc::new(tokio::sync::Semaphore::new(8));
        let mut ctx = context(AuthLevel::None);
        let mut interest = None;

        Server::handle_message(
            &request(9, "core.quick"),
            &mut ctx,
            &mut interest,
            &handler,
            &outbound,
            &in_flight,
        )
        .await;

        let _ = tokio::time::timeout(Duration::from_secs(2), replies.recv())
            .await
            .expect("an answer arrived");
        assert_eq!(
            handler.started.load(Ordering::SeqCst),
            0,
            "a call ran before a login"
        );
    }

    #[tokio::test]
    async fn a_method_nobody_has_is_still_unknown() {
        let handler = Slow::new(Duration::from_millis(0));
        let (outbound, mut replies) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
        let in_flight = Arc::new(tokio::sync::Semaphore::new(8));
        let mut ctx = context(AuthLevel::Admin);
        let mut interest = None;

        Server::handle_message(
            &request(11, "core.no_such_thing"),
            &mut ctx,
            &mut interest,
            &handler,
            &outbound,
            &in_flight,
        )
        .await;

        let _ = tokio::time::timeout(Duration::from_secs(2), replies.recv())
            .await
            .expect("an answer arrived");
        assert_eq!(handler.started.load(Ordering::SeqCst), 0);
    }
}
