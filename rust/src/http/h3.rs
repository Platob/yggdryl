//! HTTP/3 under the client: one QUIC connection per origin, its streams
//! shared by every thread sending there.
//!
//! An origin is reached over QUIC when HTTP/3 is asked for by name, or when
//! it advertised HTTP/3 in `Alt-Svc` ([`super::alt_svc`]) - on the port it
//! named, for as long as it said. QUIC is UDP, which networks drop more
//! readily than TCP: a connection that cannot be made within the connect
//! timeout, or whose handshake fails, marks the origin broken for five
//! minutes, and its requests go to HTTP/2 or HTTP/1.1 meanwhile, in the same
//! attempt - a blocked port costs one timeout, not one per request.
//!
//! One endpoint per address family sends every connection of a client, and
//! the connection's control streams are driven by a task on the private
//! runtime; a request is opened, its body written and its answer read on the
//! calling thread, as over HTTP/2. The receive windows are wide - 4 MiB a
//! stream, 16 MiB the connection - and a keep-alive holds a pooled
//! connection's path open while it is idle.

use std::io::{self, Read};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bytes::{Buf, Bytes};
use h3::client::{RequestStream, SendRequest};
use h3::error::StreamError;

use super::HttpVersion;
use super::alt_svc::Advice;
use super::client::{Answer, Payload, Wire};
use super::h2::{Origin, Remembered, Slot, Slots, headers_of, request_of, status_of};
use super::runtime;
use super::tls::ClientConfigs;

/// How long an origin whose QUIC connection failed is reached another way.
const BROKEN_FOR: Duration = Duration::from_secs(300);
/// The receive window of one stream, the client's and the server's alike.
///
/// The window is what bounds the gaps a lossy path can leave in a stream:
/// quinn reassembles one out of at most [`QUIC_STREAM_CHUNKS`] pieces that
/// do not touch and closes the whole connection past that ("too many gaps
/// in stream buffer"), and a window in which every other datagram was lost
/// holds one piece per two datagrams. 4 MiB held some 1900 of them, so a
/// starved receiver dropping datagrams of a large body lost the connection;
/// 1 MiB, close to quinn's own default, holds under 500.
pub(crate) const STREAM_WINDOW: u32 = 1 << 20;
/// The receive window of the whole connection.
pub(crate) const CONNECTION_WINDOW: u32 = 16 << 20;
/// The pieces that do not touch quinn reassembles one stream from
/// (`quinn-proto`'s `MAX_CHUNKS`) before it closes the connection.
const QUIC_STREAM_CHUNKS: u32 = 1024;
/// The least stream data a full datagram carries: QUIC's 1200-byte minimum,
/// less its packet and frame headers.
const QUIC_DATAGRAM_DATA: u32 = 1100;
// Twice over: a path whose datagrams run smaller still stays under the bound.
const _: () = assert!(STREAM_WINDOW / (2 * QUIC_DATAGRAM_DATA) < QUIC_STREAM_CHUNKS / 2);
/// How often an idle connection is kept alive.
const KEEP_ALIVE: Duration = Duration::from_secs(10);
/// How long a connection lives with nothing on it.
const IDLE: Duration = Duration::from_secs(60);
/// The most a request body is cut into per write.
const SEND_CHUNK: usize = 64 << 10;

type Sender = SendRequest<h3_quinn::OpenStreams, Bytes>;
type Stream = RequestStream<h3_quinn::BidiStream<Bytes>, Bytes>;

/// What one origin's slot holds: the request handle and the QUIC connection
/// whose liveness it is read by.
type Held = (Sender, quinn::Connection);

/// What one client knows of HTTP/3: its endpoints, its connections, the
/// alternatives origins advertised and the origins QUIC failed for.
pub(crate) struct Pool {
    config: quinn::ClientConfig,
    connect_timeout: Duration,
    /// One endpoint per address family, bound on first use.
    endpoints: Mutex<(Option<quinn::Endpoint>, Option<quinn::Endpoint>)>,
    slots: Slots<Held>,
    /// The HTTP/3 port an origin advertised, and until when it stands.
    alternatives: Remembered<(u16, Instant)>,
    /// Until when an origin is reached another way.
    broken: Remembered<Instant>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pool")
            .field("connect_timeout", &self.connect_timeout)
            .finish_non_exhaustive()
    }
}

/// How an attempt at HTTP/3 ended short of an answer.
pub(crate) enum Declined {
    /// QUIC is not available to this origin now: reach it another way.
    Fallback,
    /// The request failed, in the retry rules' vocabulary.
    Failed(ureq::Error),
}

impl Pool {
    /// A pool verifying servers as `configs` do.
    ///
    /// # Errors
    ///
    /// When the TLS configuration cannot carry QUIC.
    pub(crate) fn new(configs: &ClientConfigs, connect_timeout: Duration) -> crate::Result<Self> {
        let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(Arc::clone(&configs.h3))
            .map_err(|error| {
                crate::Error::Io(io::Error::other(format!(
                    "the TLS configuration cannot carry QUIC: {error}"
                )))
            })?;
        let mut config = quinn::ClientConfig::new(Arc::new(crypto));
        config.transport_config(Arc::new(transport()));
        Ok(Self {
            config,
            connect_timeout,
            endpoints: Mutex::new((None, None)),
            slots: Slots::default(),
            alternatives: Remembered::default(),
            broken: Remembered::default(),
        })
    }

    /// The port `origin` answers HTTP/3 on: its own when `asked`, else the
    /// one it advertised while that stands; `None` when HTTP/3 is not for it
    /// now.
    pub(crate) fn port_for(&self, origin: &Origin, asked: bool) -> Option<u16> {
        if !origin.secure {
            return None;
        }
        let now = Instant::now();
        if self.broken.get(origin).is_some_and(|until| until > now) {
            return None;
        }
        match self.alternatives.get(origin) {
            Some((port, until)) if until > now => Some(port),
            Some(_) => {
                self.alternatives.remove(origin);
                asked.then_some(origin.port)
            }
            None => asked.then_some(origin.port),
        }
    }

    /// Take up what an answer's `Alt-Svc` said of `origin`.
    pub(crate) fn advise(&self, origin: &Origin, advice: Advice) {
        match advice {
            Advice::Http3 { port, max_age } => {
                if let Some(until) = Instant::now().checked_add(max_age) {
                    self.alternatives.insert(origin.clone(), (port, until));
                }
            }
            Advice::Clear => self.alternatives.remove(origin),
        }
    }

    /// Send `wire` over HTTP/3 to `origin` on `port`, and read the answer's
    /// head.
    pub(crate) fn exchange(
        &self,
        origin: &Origin,
        port: u16,
        wire: &Wire<'_>,
        payload: Option<Payload<'_>>,
    ) -> std::result::Result<Answer, Declined> {
        let slot = self.slots.of(origin);
        let request =
            request_of(origin, wire, ureq::http::Version::HTTP_3).map_err(Declined::Failed)?;
        let timeout = wire.timeout;
        let connect_timeout = wire.connect_timeout.unwrap_or(self.connect_timeout);
        // The one bound on the whole attempt, the body's reads included.
        let until = wire
            .deadline
            .and_then(|deadline| Instant::now().checked_add(deadline));
        let exchange = async {
            let (sender, connection) =
                match self.connection(origin, port, &slot, connect_timeout).await {
                    Some(held) => held,
                    None => return Err(Declined::Fallback),
                };
            match send(sender, request, payload, timeout).await {
                Ok(answered) => Ok(answered),
                Err(error) => {
                    if is_connection_error(&error) {
                        let id = connection.stable_id();
                        slot.forget(|(_, held)| held.stable_id() == id);
                    }
                    Err(Declined::Failed(failure(error)))
                }
            }
        };
        let outcome = runtime::wait(runtime::before(until, exchange))
            .map_err(|error| Declined::Failed(ureq::Error::Io(error)))?
            .unwrap_or(Err(Declined::Failed(ureq::Error::Timeout(
                ureq::Timeout::Global,
            ))));
        match outcome {
            Ok((response, stream)) => Ok(Answer {
                status: status_of(response.status()).map_err(Declined::Failed)?,
                version: HttpVersion::Http3,
                headers: headers_of(response.headers()).map_err(Declined::Failed)?,
                body: Box::new(Body {
                    stream,
                    chunk: Bytes::new(),
                    timeout,
                    until,
                    done: false,
                }),
                attempts: 1,
            }),
            Err(Declined::Fallback) => {
                if let Some(until) = Instant::now().checked_add(BROKEN_FOR) {
                    self.broken.insert(origin.clone(), until);
                }
                Err(Declined::Fallback)
            }
            Err(failed) => Err(failed),
        }
    }

    /// The live connection to `origin`, opened within `connect_timeout` when
    /// there is none; `None` when QUIC cannot reach it.
    async fn connection(
        &self,
        origin: &Origin,
        port: u16,
        slot: &Slot<Held>,
        connect_timeout: Duration,
    ) -> Option<Held> {
        if let Some(held) = live(slot) {
            return Some(held);
        }
        let _opening = slot.opening.lock().await;
        // The thread that held the lock before this one may have opened it.
        if let Some(held) = live(slot) {
            return Some(held);
        }
        let held = runtime::within(connect_timeout, self.open(origin, port)).await??;
        slot.set(held.clone());
        Some(held)
    }

    /// Open a QUIC connection to `origin` on `port` and its HTTP/3 session.
    async fn open(&self, origin: &Origin, port: u16) -> Option<(Sender, quinn::Connection)> {
        let addresses: Vec<SocketAddr> = tokio::net::lookup_host((origin.host.as_str(), port))
            .await
            .ok()?
            .collect();
        for address in addresses {
            let Some(endpoint) = self.endpoint(address) else {
                continue;
            };
            let Ok(connecting) = endpoint.connect_with(self.config.clone(), address, &origin.host)
            else {
                continue;
            };
            let Ok(connection) = connecting.await else {
                continue;
            };
            let Ok((mut driver, sender)) =
                h3::client::new(h3_quinn::Connection::new(connection.clone())).await
            else {
                continue;
            };
            tokio::spawn(async move {
                // The control streams end with the connection.
                let _ = std::future::poll_fn(|context| driver.poll_close(context)).await;
            });
            return Some((sender, connection));
        }
        None
    }

    /// The endpoint that sends to `address`'s family, bound on first use.
    fn endpoint(&self, address: SocketAddr) -> Option<quinn::Endpoint> {
        let mut endpoints = self
            .endpoints
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (bind, held) = if address.is_ipv4() {
            (SocketAddr::from(([0, 0, 0, 0], 0)), &mut endpoints.0)
        } else {
            (SocketAddr::from(([0_u16; 8], 0)), &mut endpoints.1)
        };
        if held.is_none() {
            *held = quinn::Endpoint::client(bind).ok();
        }
        held.clone()
    }
}

/// The connection `slot` holds while QUIC keeps it open; one closed since -
/// idle, reset, gone away - is dropped, to be opened again.
fn live(slot: &Slot<Held>) -> Option<Held> {
    let held = slot.get()?;
    if held.1.close_reason().is_none() {
        return Some(held);
    }
    let id = held.1.stable_id();
    slot.forget(|(_, current)| current.stable_id() == id);
    None
}

/// The transport every client connection runs under.
fn transport() -> quinn::TransportConfig {
    let mut transport = quinn::TransportConfig::default();
    transport
        .stream_receive_window(STREAM_WINDOW.into())
        .receive_window(CONNECTION_WINDOW.into())
        .keep_alive_interval(Some(KEEP_ALIVE))
        .max_idle_timeout(quinn::IdleTimeout::try_from(IDLE).ok());
    transport
}

/// `future` within `timeout`, a phase named `phase` when it runs out.
async fn within<T>(
    timeout: Duration,
    phase: ureq::Timeout,
    future: impl std::future::Future<Output = std::result::Result<T, StreamError>>,
) -> std::result::Result<T, Unsent> {
    match runtime::within(timeout, future).await {
        Some(done) => Ok(done?),
        None => Err(Unsent::TimedOut(phase)),
    }
}

/// Open the stream, write the body, and wait for the answer's head, each
/// write and the wait for the head bounded by `timeout` apiece, as HTTP/1.1
/// bounds its phases apart.
async fn send(
    mut sender: Sender,
    request: ureq::http::Request<()>,
    payload: Option<Payload<'_>>,
    timeout: Duration,
) -> std::result::Result<(ureq::http::Response<()>, Stream), Unsent> {
    // A stream that could not be opened carried nothing.
    let mut stream = match runtime::within(timeout, sender.send_request(request)).await {
        Some(Ok(stream)) => stream,
        Some(Err(error)) => return Err(Unsent::Unopened(error)),
        None => return Err(Unsent::TimedOut(ureq::Timeout::Connect)),
    };
    match payload {
        Some(Payload::Bytes(bytes)) => {
            for chunk in bytes.chunks(SEND_CHUNK) {
                within(
                    timeout,
                    ureq::Timeout::SendBody,
                    stream.send_data(Bytes::copy_from_slice(chunk)),
                )
                .await?;
            }
        }
        Some(Payload::Reader { body, length }) => {
            let mut left = length;
            let mut buffer =
                vec![0_u8; SEND_CHUNK.min(usize::try_from(length).unwrap_or(SEND_CHUNK))];
            while left > 0 {
                let want = usize::try_from(left)
                    .unwrap_or(usize::MAX)
                    .min(buffer.len());
                let read = body.read(&mut buffer[..want]).map_err(Unsent::Body)?;
                if read == 0 {
                    return Err(Unsent::Body(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!("the request body ended {left} bytes short of its length"),
                    )));
                }
                left -= read as u64;
                within(
                    timeout,
                    ureq::Timeout::SendBody,
                    stream.send_data(Bytes::copy_from_slice(&buffer[..read])),
                )
                .await?;
            }
        }
        None => {}
    }
    within(timeout, ureq::Timeout::SendBody, stream.finish()).await?;
    let response = within(timeout, ureq::Timeout::RecvResponse, stream.recv_response()).await?;
    Ok((response, stream))
}

/// Why a request got no answer: its stream, a stream that never opened, the
/// caller's body reader, or a phase that ran out of time.
enum Unsent {
    Stream(StreamError),
    Unopened(StreamError),
    Body(io::Error),
    TimedOut(ureq::Timeout),
}

impl From<StreamError> for Unsent {
    fn from(error: StreamError) -> Self {
        Self::Stream(error)
    }
}

/// Whether `error` ended the connection, not only the stream.
fn is_connection_error(error: &Unsent) -> bool {
    matches!(
        error,
        Unsent::Unopened(_)
            | Unsent::Stream(
                StreamError::ConnectionError { .. } | StreamError::RemoteClosing { .. }
            )
    )
}

/// An HTTP/3 failure as the retry rules read it.
fn failure(error: Unsent) -> ureq::Error {
    let error = match error {
        Unsent::Body(error) => return ureq::Error::Io(error),
        Unsent::TimedOut(phase) => return ureq::Error::Timeout(phase),
        // Nothing went out on a stream that never opened.
        Unsent::Unopened(error) => {
            return ureq::Error::Io(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("http/3: {error}"),
            ));
        }
        Unsent::Stream(error) => error,
    };
    let kind = match &error {
        // The server never ran it: resending cannot act twice.
        StreamError::RemoteClosing { .. } => io::ErrorKind::ConnectionRefused,
        StreamError::RemoteTerminate { code, .. }
            if *code == h3::error::Code::H3_REQUEST_REJECTED =>
        {
            io::ErrorKind::ConnectionRefused
        }
        _ => io::ErrorKind::ConnectionReset,
    };
    ureq::Error::Io(io::Error::new(kind, format!("http/3: {error}")))
}

/// An HTTP/3 answer's body, read on the caller's thread a frame at a time.
struct Body {
    stream: Stream,
    chunk: Bytes,
    /// How long one read waits for the next frame.
    timeout: Duration,
    /// The deadline of the attempt the body answers, past which no read
    /// waits.
    until: Option<Instant>,
    done: bool,
}

impl Read for Body {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if !self.chunk.is_empty() || buffer.is_empty() {
                let taken = self.chunk.len().min(buffer.len());
                self.chunk.copy_to_slice(&mut buffer[..taken]);
                return Ok(taken);
            }
            if self.done {
                return Ok(0);
            }
            let bound = runtime::capped(self.timeout, self.until);
            let received = runtime::wait_for(bound, async {
                self.stream
                    .recv_data()
                    .await
                    .map(|data| data.map(|mut data| data.copy_to_bytes(data.remaining())))
            })?;
            match received {
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "no HTTP/3 frame arrived within the timeout",
                    ));
                }
                Some(Ok(None)) => self.done = true,
                Some(Ok(Some(chunk))) => self.chunk = chunk,
                Some(Err(error)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        format!("http/3: {error}"),
                    ));
                }
            }
        }
    }
}
