// SPDX-License-Identifier: GPL-3.0-or-later
//! A client for the Deluge daemon.
//!
//! One task owns the socket; calls go in over a channel and replies come back
//! on a oneshot, so several callers can share a connection without taking turns.
//! Events arrive unsolicited and are broadcast.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use redeluge_rencode::Value;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_rustls::TlsConnector;

use crate::message::{requests_to_value, Failure, Incoming, Request};
use crate::tls::TlsMode;
use crate::transfer::{encode_frame, FrameReader, Limits};

/// Where the daemon listens when nothing says otherwise.
pub const DEFAULT_DAEMON_PORT: u16 = 58846;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not reach the daemon at {address}: {source}")]
    Connect {
        address: String,
        source: std::io::Error,
    },

    #[error("TLS handshake with the daemon failed: {0}")]
    Tls(String),

    #[error("the connection to the daemon was lost")]
    Disconnected,

    #[error("the daemon did not answer within {0:?}")]
    Timeout(Duration),

    #[error("the daemon refused the call: {0}")]
    Remote(Failure),

    #[error(transparent)]
    Protocol(#[from] crate::transfer::Error),

    #[error("the daemon sent something unexpected: {0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// An event the daemon pushed without being asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub name: String,
    pub args: Vec<Value>,
}

/// How the client behaves once connected.
#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub tls: TlsMode,
    pub limits: Limits,
    /// How long a single call may take before it is abandoned.
    pub call_timeout: Duration,
    /// Version reported to the daemon. It refuses a login without one.
    pub client_version: String,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self {
            tls: TlsMode::Insecure,
            limits: Limits::default(),
            call_timeout: Duration::from_secs(30),
            client_version: concat!("redeluge ", env!("CARGO_PKG_VERSION")).to_owned(),
        }
    }
}

struct Pending {
    request: Request,
    reply: oneshot::Sender<Result<Value>>,
}

/// A connection to the daemon.
///
/// Cloning shares the connection rather than opening another.
#[derive(Clone)]
pub struct Client {
    outgoing: mpsc::Sender<Pending>,
    events: broadcast::Sender<Event>,
    next_id: Arc<AtomicI64>,
    call_timeout: Duration,
    client_version: String,
}

impl Client {
    /// Opens a connection and starts the task that owns it.
    ///
    /// The connection is not authenticated yet; call [`Client::login`] next.
    pub async fn connect(host: &str, port: u16, settings: ClientSettings) -> Result<Self> {
        let address = format!("{host}:{port}");
        let stream = TcpStream::connect(&address)
            .await
            .map_err(|source| Error::Connect {
                address: address.clone(),
                source,
            })?;
        // Deluge's traffic is many small calls; waiting to coalesce them adds
        // latency to every one.
        let _ = stream.set_nodelay(true);

        let connector = TlsConnector::from(Arc::new(settings.tls.client_config()));
        // The daemon's certificate names nothing useful, and verification is
        // handled by TlsMode, so the name here only has to be well-formed.
        let server_name = ServerName::try_from(host.to_owned())
            .unwrap_or_else(|_| ServerName::try_from("deluge").expect("valid literal"));

        let stream = connector
            .connect(server_name, stream)
            .await
            .map_err(|err| Error::Tls(err.to_string()))?;

        let (outgoing_tx, outgoing_rx) = mpsc::channel(64);
        let (events_tx, _) = broadcast::channel(256);

        tokio::spawn(drive(
            stream,
            outgoing_rx,
            events_tx.clone(),
            settings.limits,
        ));

        Ok(Self {
            outgoing: outgoing_tx,
            events: events_tx,
            next_id: Arc::new(AtomicI64::new(0)),
            call_timeout: settings.call_timeout,
            client_version: settings.client_version,
        })
    }

    /// Subscribes to the daemon's events. Late subscribers miss earlier ones.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Calls a method and waits for its reply.
    pub async fn call(&self, method: &str, args: Vec<Value>) -> Result<Value> {
        self.call_with(Request {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            method: method.to_owned(),
            args,
            kwargs: Vec::new(),
        })
        .await
    }

    /// Calls a method that takes keyword arguments.
    pub async fn call_with(&self, mut request: Request) -> Result<Value> {
        request.id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let (reply_tx, reply_rx) = oneshot::channel();
        self.outgoing
            .send(Pending {
                request,
                reply: reply_tx,
            })
            .await
            .map_err(|_| Error::Disconnected)?;

        match tokio::time::timeout(self.call_timeout, reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(Error::Disconnected),
            Err(_) => Err(Error::Timeout(self.call_timeout)),
        }
    }

    /// The daemon's version. Answerable before logging in.
    pub async fn info(&self) -> Result<String> {
        let value = self.call("daemon.info", vec![]).await?;
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Malformed("daemon.info did not return a string".to_owned()))
    }

    /// Authenticates, returning the granted authorisation level.
    ///
    /// The daemon rejects a login without `client_version`, which is how it
    /// keeps very old clients out.
    pub async fn login(&self, username: &str, password: &str) -> Result<i64> {
        let request = Request::new(0, "daemon.login")
            .arg(username)
            .arg(password)
            .kwarg("client_version", self.client_version.as_str());

        let value = self.call_with(request).await?;
        value
            .as_i64()
            .ok_or_else(|| Error::Malformed("daemon.login did not return a level".to_owned()))
    }

    /// Every method this daemon exposes to the current session.
    pub async fn method_list(&self) -> Result<Vec<String>> {
        let value = self.call("daemon.get_method_list", vec![]).await?;
        let items = value
            .as_list()
            .ok_or_else(|| Error::Malformed("method list is not a list".to_owned()))?;
        Ok(items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect())
    }

    /// Asks the daemon to start sending the named events.
    pub async fn set_event_interest(&self, events: &[&str]) -> Result<()> {
        let names = Value::List(
            events
                .iter()
                .map(|name| Value::Str((*name).into()))
                .collect(),
        );
        self.call("daemon.set_event_interest", vec![names]).await?;
        Ok(())
    }
}

/// Owns the socket: writes requests, reads replies, routes both.
async fn drive<S>(
    stream: S,
    mut outgoing: mpsc::Receiver<Pending>,
    events: broadcast::Sender<Event>,
    limits: Limits,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut reader_half, mut writer_half) = tokio::io::split(stream);
    let mut frames = FrameReader::new(limits);
    let mut waiting: HashMap<i64, oneshot::Sender<Result<Value>>> = HashMap::new();
    let mut chunk = vec![0u8; 16 * 1024];

    loop {
        tokio::select! {
            request = outgoing.recv() => {
                let Some(pending) = request else {
                    break; // Every Client was dropped.
                };

                let one = std::slice::from_ref(&pending.request);
                let frame = match encode_frame(&requests_to_value(one)) {
                    Ok(frame) => frame,
                    Err(err) => {
                        let _ = pending.reply.send(Err(Error::Protocol(err)));
                        continue;
                    }
                };

                if writer_half.write_all(&frame).await.is_err() {
                    let _ = pending.reply.send(Err(Error::Disconnected));
                    break;
                }
                waiting.insert(pending.request.id, pending.reply);
            }

            read = reader_half.read(&mut chunk) => {
                let count = match read {
                    Ok(0) | Err(_) => break,
                    Ok(count) => count,
                };
                frames.feed(&chunk[..count]);

                loop {
                    match frames.next_message() {
                        Ok(None) => break,
                        Ok(Some(value)) => route(&value, &mut waiting, &events),
                        Err(err) => {
                            // Framing is broken; the stream cannot be trusted to
                            // resynchronise, so everyone waiting is told.
                            tracing::warn!(error = %err, "dropping the daemon connection");
                            fail_all(&mut waiting, || Error::Disconnected);
                            return;
                        }
                    }
                }
            }
        }
    }

    fail_all(&mut waiting, || Error::Disconnected);
}

fn route(
    value: &Value,
    waiting: &mut HashMap<i64, oneshot::Sender<Result<Value>>>,
    events: &broadcast::Sender<Event>,
) {
    match Incoming::from_value(value) {
        Ok(Incoming::Response { id, result }) => {
            if let Some(reply) = waiting.remove(&id) {
                let _ = reply.send(Ok(result));
            }
        }
        Ok(Incoming::Error { id, failure }) => {
            if let Some(reply) = waiting.remove(&id) {
                let _ = reply.send(Err(Error::Remote(failure)));
            }
        }
        Ok(Incoming::Event { name, args }) => {
            // Nobody subscribed is not an error.
            let _ = events.send(Event { name, args });
        }
        Err(err) => tracing::warn!(error = %err, "ignoring an unparseable message"),
    }
}

fn fail_all<F>(waiting: &mut HashMap<i64, oneshot::Sender<Result<Value>>>, error: F)
where
    F: Fn() -> Error,
{
    for (_, reply) in waiting.drain() {
        let _ = reply.send(Err(error()));
    }
}
