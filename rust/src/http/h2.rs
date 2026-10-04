//! HTTP/2 under the client: one multiplexed connection per origin, its
//! streams shared by every thread sending there.
//!
//! An `https` origin is asked for `h2` by ALPN in the TLS handshake and
//! answers `h2` or `http/1.1`; a plain one is spoken to with prior knowledge
//! (`h2c`) only when HTTP/2 is asked for by name. An origin that picks
//! HTTP/1.1, or a plain one that does not read the HTTP/2 preface, is
//! remembered, and its requests go to the HTTP/1.1 transport from then on -
//! the refusal is learned once, not per request.
//!
//! The connection's frames are driven by a task on the private runtime
//! ([`super::runtime`]); a request is opened, its body written and its
//! answer's head awaited on the calling thread, and the body is read there
//! too, one `DATA` frame at a time, each frame's bytes handed back to the
//! stream's window as they are taken. The windows are wide - 4 MiB a stream,
//! 16 MiB the connection - so a transfer is not throttled by its own flow
//! control on a link with any latency, and a connection that dies is
//! dropped from the pool and opened again by the next request.
//!
//! Failures are spelled in the vocabulary the retry rules already read: a
//! stream the server refused, or one past its `GOAWAY`, never ran there and
//! is an unsent request; a stream reset mid-body is a severed transfer a
//! [`super::Stream`] resumes.

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bytes::{Buf, Bytes};
use h2::client::SendRequest;
use tokio::net::TcpStream;
use ureq::http::{HeaderMap, Uri};

use super::client::{Answer, Payload, Wire};
use super::runtime;
use super::tls::ClientConfigs;
use super::{Headers, HttpVersion, Status};
use crate::Url;

/// The receive window of one stream.
const STREAM_WINDOW: u32 = 4 << 20;
/// The receive window of the whole connection.
const CONNECTION_WINDOW: u32 = 16 << 20;
/// The largest frame the peer may send.
const MAX_FRAME: u32 = 64 << 10;
/// The most a request body is cut into per `DATA` frame asked for.
const SEND_CHUNK: usize = 64 << 10;
/// The most origins a pool remembers the version of.
pub(crate) const REMEMBERED: usize = 4096;

/// Where a connection goes: scheme, host and port.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct Origin {
    pub(crate) secure: bool,
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl Origin {
    /// The origin of `url`, when it is an `http` or `https` URL with a host.
    pub(crate) fn of(url: &Url) -> Option<Self> {
        let scheme = url.scheme().as_str();
        let secure = if scheme.eq_ignore_ascii_case("https") {
            true
        } else if scheme.eq_ignore_ascii_case("http") {
            false
        } else {
            return None;
        };
        let host = url
            .hostname()?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        let port = url
            .authority()
            .port()
            .unwrap_or(if secure { 443 } else { 80 });
        Some(Self { secure, host, port })
    }

    /// The host as a URI authority spells it: an IPv6 address bracketed.
    pub(crate) fn authority_host(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }
}

/// A set of origins with what was learned of each, bounded: past
/// [`REMEMBERED`] entries it forgets everything and learns again, which
/// costs one refused attempt per origin still in use.
#[derive(Debug)]
pub(crate) struct Remembered<T> {
    entries: Mutex<HashMap<Origin, T>>,
}

impl<T> Default for Remembered<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: Clone> Remembered<T> {
    pub(crate) fn get(&self, origin: &Origin) -> Option<T> {
        self.lock().get(origin).cloned()
    }

    pub(crate) fn insert(&self, origin: Origin, value: T) {
        let mut entries = self.lock();
        if entries.len() >= REMEMBERED && !entries.contains_key(&origin) {
            entries.clear();
        }
        entries.insert(origin, value);
    }

    #[cfg(feature = "http3")]
    pub(crate) fn remove(&self, origin: &Origin) {
        self.lock().remove(origin);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Origin, T>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The slot of every origin a pool reached, each shared by the requests to
/// it; past [`REMEMBERED`] origins the slots no request holds are dropped.
pub(crate) struct Slots<C> {
    slots: Mutex<HashMap<Origin, Arc<Slot<C>>>>,
}

impl<C> Default for Slots<C> {
    fn default() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
        }
    }
}

impl<C> Slots<C> {
    /// The slot of `origin`, made when it has none.
    pub(crate) fn of(&self, origin: &Origin) -> Arc<Slot<C>> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = slots.get(origin) {
            return Arc::clone(slot);
        }
        if slots.len() >= REMEMBERED {
            // A slot held only here has no request in flight; its
            // connection, when it has one, ends with its last stream.
            slots.retain(|_, slot| Arc::strong_count(slot) > 1);
        }
        Arc::clone(slots.entry(origin.clone()).or_default())
    }
}

/// The slot one origin's connection lives in: the connection, read and
/// replaced under a brief lock that no wait is made under, and the lock a
/// thread opening one holds, so threads racing to a new origin open one.
pub(crate) struct Slot<C> {
    held: Mutex<Option<C>>,
    pub(crate) opening: tokio::sync::Mutex<()>,
}

impl<C> Default for Slot<C> {
    fn default() -> Self {
        Self {
            held: Mutex::new(None),
            opening: tokio::sync::Mutex::const_new(()),
        }
    }
}

impl<C: Clone> Slot<C> {
    /// The connection held, when there is one.
    pub(crate) fn get(&self) -> Option<C> {
        self.lock().clone()
    }

    /// Hold `connection` from now on.
    pub(crate) fn set(&self, connection: C) {
        *self.lock() = Some(connection);
    }

    /// Drop the connection held when `is_it` says it is the one a request
    /// found dead, and not one another thread opened since.
    pub(crate) fn forget(&self, is_it: impl FnOnce(&C) -> bool) {
        let mut held = self.lock();
        if held.as_ref().is_some_and(is_it) {
            *held = None;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<C>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One open connection: the handle streams are opened on, and whether it
/// ever answered, which tells a server that does not speak `h2c` from one
/// that failed a request.
#[derive(Clone)]
struct Connection {
    send: SendRequest<Bytes>,
    answered: Arc<AtomicBool>,
}

/// What one client knows of HTTP/2: its connections and the origins that
/// answered HTTP/1.1.
pub(crate) struct Pool {
    tls: Arc<rustls::ClientConfig>,
    connect_timeout: Duration,
    slots: Slots<Connection>,
    /// Origins that speak HTTP/1.1 only.
    http1: Remembered<()>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pool")
            .field("connect_timeout", &self.connect_timeout)
            .finish_non_exhaustive()
    }
}

/// How an attempt at HTTP/2 ended short of an answer.
pub(crate) enum Declined {
    /// The origin speaks HTTP/1.1: send the request there.
    Http1,
    /// The transport failed, in the retry rules' vocabulary.
    Failed(ureq::Error),
}

impl From<ureq::Error> for Declined {
    fn from(error: ureq::Error) -> Self {
        Self::Failed(error)
    }
}

impl Pool {
    pub(crate) fn new(configs: &ClientConfigs, connect_timeout: Duration) -> Self {
        Self {
            tls: Arc::clone(&configs.h2),
            connect_timeout,
            slots: Slots::default(),
            http1: Remembered::default(),
        }
    }

    /// Whether `origin` is known to speak HTTP/1.1 only.
    pub(crate) fn speaks_http1(&self, origin: &Origin) -> bool {
        self.http1.get(origin).is_some()
    }

    /// Send `wire` over HTTP/2 to `origin` - `prior_knowledge` opening a
    /// plain origin without asking - and read the answer's head.
    pub(crate) fn exchange(
        &self,
        origin: &Origin,
        prior_knowledge: bool,
        wire: &Wire<'_>,
        payload: Option<Payload<'_>>,
    ) -> std::result::Result<Answer, Declined> {
        if self.speaks_http1(origin) || (!origin.secure && !prior_knowledge) {
            return Err(Declined::Http1);
        }
        let slot = self.slots.of(origin);
        let request = request_of(origin, wire, ureq::http::Version::HTTP_2)?;
        let timeout = wire.timeout;
        let connect_timeout = wire.connect_timeout.unwrap_or(self.connect_timeout);
        // The one bound on the whole attempt, the body's reads included.
        let until = wire
            .deadline
            .and_then(|deadline| Instant::now().checked_add(deadline));
        // A body read from the caller's reader cannot be read again, so a
        // refusal found after it went out cannot fall back.
        let streamed = matches!(payload, Some(Payload::Reader { length, .. }) if length > 0);
        let exchange = async {
            // Opening a stream - connecting, or waiting for the peer to allow
            // one more - is bounded like any phase; nothing went out yet.
            // Every handle on a connection is counted under the lock all its
            // streams share, so the request takes the one it was handed.
            let Connection {
                send: sender,
                answered,
            } = match runtime::within(timeout, self.connection(origin, &slot, connect_timeout))
                .await
            {
                None => {
                    return Err(Declined::Failed(ureq::Error::Timeout(
                        ureq::Timeout::Connect,
                    )));
                }
                Some(Ok(Some(connection))) => connection,
                Some(Ok(None)) => return Err(Declined::Http1),
                Some(Err(error)) => return Err(Declined::Failed(error)),
            };
            match send(sender, request, payload, timeout).await {
                Ok(response) => {
                    answered.store(true, Ordering::Relaxed);
                    Ok(response)
                }
                Err(Unsent::Body(error)) => Err(Declined::Failed(ureq::Error::Io(error))),
                Err(Unsent::TimedOut(phase)) => Err(Declined::Failed(ureq::Error::Timeout(phase))),
                Err(Unsent::Frames(error)) => {
                    // A plain origin that never answered a frame is not
                    // speaking HTTP/2: what it read was not a request.
                    let refused =
                        !origin.secure && !answered.load(Ordering::Relaxed) && !error.is_remote();
                    // One stream reset leaves the connection serving the rest.
                    if !error.is_reset() {
                        slot.forget(|held| Arc::ptr_eq(&held.answered, &answered));
                    }
                    match (refused, streamed) {
                        (true, false) => Err(Declined::Http1),
                        (true, true) => {
                            self.http1.insert(origin.clone(), ());
                            Err(Declined::Failed(ureq::Error::Io(io::Error::new(
                                io::ErrorKind::ConnectionRefused,
                                format!(
                                    "{}:{} does not speak HTTP/2 by prior knowledge; the \
                                     request's body was spent on it",
                                    origin.host, origin.port
                                ),
                            ))))
                        }
                        (false, _) => Err(Declined::Failed(failure(&error))),
                    }
                }
            }
        };
        let outcome = runtime::wait(runtime::before(until, exchange))
            .map_err(|error| Declined::Failed(ureq::Error::Io(error)))?
            .unwrap_or(Err(Declined::Failed(ureq::Error::Timeout(
                ureq::Timeout::Global,
            ))));
        match outcome {
            Ok(response) => Ok(answer(response, timeout, until)?),
            Err(Declined::Http1) => {
                self.http1.insert(origin.clone(), ());
                Err(Declined::Http1)
            }
            Err(failed) => Err(failed),
        }
    }

    /// The live connection to `origin`, opened within `connect_timeout` when
    /// there is none; `None` when the origin answered TLS with HTTP/1.1.
    async fn connection(
        &self,
        origin: &Origin,
        slot: &Slot<Connection>,
        connect_timeout: Duration,
    ) -> std::result::Result<Option<Connection>, ureq::Error> {
        if let Some(connection) = ready(slot).await {
            return Ok(Some(connection));
        }
        let _opening = slot.opening.lock().await;
        // The thread that held the lock before this one may have opened it.
        if let Some(connection) = ready(slot).await {
            return Ok(Some(connection));
        }
        let opened = runtime::within(connect_timeout, self.open(origin)).await;
        let Some(send) = opened.ok_or(ureq::Error::Timeout(ureq::Timeout::Connect))?? else {
            return Ok(None);
        };
        let send = match send.ready().await {
            Ok(send) => send,
            // A plain origin that hung up on the preface does not speak
            // HTTP/2; nothing was sent to act on.
            Err(error) if !origin.secure && !error.is_remote() => return Ok(None),
            Err(error) => return Err(failure(&error)),
        };
        let connection = Connection {
            send,
            answered: Arc::new(AtomicBool::new(false)),
        };
        slot.set(connection.clone());
        Ok(Some(connection))
    }

    /// Open a connection to `origin`: TCP, then TLS offering `h2` for a
    /// secure one; `None` when TLS settled on HTTP/1.1.
    async fn open(
        &self,
        origin: &Origin,
    ) -> std::result::Result<Option<SendRequest<Bytes>>, ureq::Error> {
        let tcp = connect_tcp(origin).await?;
        if !origin.secure {
            return handshake(tcp).await.map(Some);
        }
        let name = rustls::pki_types::ServerName::try_from(origin.host.clone())
            .map_err(|error| ureq::Error::BadUri(format!("{}: {error}", origin.host)))?;
        let tls = tokio_rustls::TlsConnector::from(Arc::clone(&self.tls))
            .connect(name, tcp)
            .await
            .map_err(tls_failure)?;
        if tls.get_ref().1.alpn_protocol() != Some(b"h2".as_slice()) {
            return Ok(None);
        }
        handshake(tls).await.map(Some)
    }
}

/// The connection `slot` holds once it may open one more stream; one that
/// closed since is dropped.
async fn ready(slot: &Slot<Connection>) -> Option<Connection> {
    let Connection { send, answered } = slot.get()?;
    match send.ready().await {
        Ok(send) => Some(Connection { send, answered }),
        Err(_) => {
            slot.forget(|current| Arc::ptr_eq(&current.answered, &answered));
            None
        }
    }
}

/// A TCP connection to the first address of `origin` that takes one, with
/// Nagle off: a request's head and body are separate writes.
pub(crate) async fn connect_tcp(origin: &Origin) -> std::result::Result<TcpStream, ureq::Error> {
    let addresses: Vec<_> = tokio::net::lookup_host((origin.host.as_str(), origin.port))
        .await
        .map_err(|_| ureq::Error::HostNotFound)?
        .collect();
    if addresses.is_empty() {
        return Err(ureq::Error::HostNotFound);
    }
    let mut last = None;
    for address in addresses {
        match TcpStream::connect(address).await {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(error) => last = Some(error),
        }
    }
    Err(ureq::Error::Io(last.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "no address took the connection",
        )
    })))
}

/// The HTTP/2 handshake over `io`, its connection driven by a runtime task.
async fn handshake<T>(io: T) -> std::result::Result<SendRequest<Bytes>, ureq::Error>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (send, connection) = h2::client::Builder::new()
        .initial_window_size(STREAM_WINDOW)
        .initial_connection_window_size(CONNECTION_WINDOW)
        .max_frame_size(MAX_FRAME)
        .enable_push(false)
        .handshake::<_, Bytes>(io)
        .await
        .map_err(|error| failure(&error))?;
    tokio::spawn(async move {
        // The connection ends when every handle and stream is dropped, or
        // the peer goes away; either way its streams have their answer.
        let _ = connection.await;
    });
    Ok(send)
}

/// A TLS failure as the retry rules read it: a verdict, not retried.
pub(crate) fn tls_failure(error: io::Error) -> ureq::Error {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(tls) => ureq::Error::Rustls(tls.clone()),
        None => ureq::Error::Io(error),
    }
}

/// An HTTP/2 failure as the retry rules read it.
fn failure(error: &h2::Error) -> ureq::Error {
    // Only the peer can say a stream never ran there: a `REFUSED_STREAM` it
    // sent, or its `GOAWAY` naming a last stream below this one. A `GOAWAY`
    // this side raised over a broken connection proves nothing of the kind.
    let refused = error.is_remote()
        && (error.reason() == Some(h2::Reason::REFUSED_STREAM) || error.is_go_away());
    let kind = if refused {
        // The server never ran it: resending cannot act twice.
        io::ErrorKind::ConnectionRefused
    } else if error.is_io() {
        io::ErrorKind::ConnectionReset
    } else {
        io::ErrorKind::Other
    };
    ureq::Error::Io(io::Error::new(kind, format!("http/2: {error}")))
}

/// The request `wire` states, as HTTP/2 carries it: the target as
/// `:scheme`, `:authority` and `:path`, and no field that belongs to one
/// HTTP/1.1 connection.
pub(crate) fn request_of(
    origin: &Origin,
    wire: &Wire<'_>,
    version: ureq::http::Version,
) -> std::result::Result<ureq::http::Request<()>, ureq::Error> {
    let mut target = format!(
        "{}://{}:{}",
        if origin.secure { "https" } else { "http" },
        origin.authority_host(),
        origin.port
    );
    let path = wire
        .url
        .path_text(false)
        .map_err(|error| ureq::Error::BadUri(error.to_string()))?;
    target.push_str(if path.is_empty() { "/" } else { &path });
    if let Some(query) = wire
        .url
        .query(false)
        .map_err(|error| ureq::Error::BadUri(error.to_string()))?
    {
        target.push('?');
        target.push_str(&query);
    }
    let uri: Uri = target
        .parse()
        .map_err(|error| ureq::Error::BadUri(format!("{target}: {error}")))?;
    let mut builder = ureq::http::Request::builder()
        .method(wire.method.as_str())
        .uri(uri)
        .version(version);
    for (name, value) in wire.headers {
        if !is_connection_field(name, value) {
            builder = builder.header(name, value);
        }
    }
    builder.body(()).map_err(ureq::Error::Http)
}

/// Whether `name` is a field HTTP/2 and HTTP/3 forbid, since it speaks of one
/// HTTP/1.1 connection (RFC 9113 8.2.2): `Host` travels as `:authority`.
pub(crate) fn is_connection_field(name: &str, value: &str) -> bool {
    matches!(
        name,
        "connection" | "keep-alive" | "proxy-connection" | "transfer-encoding" | "upgrade" | "host"
    ) || (name == "te" && !value.eq_ignore_ascii_case("trailers"))
}

/// Why a request got no answer: the connection's frames, the caller's body
/// reader, or a phase that ran out of time.
enum Unsent {
    Frames(h2::Error),
    Body(io::Error),
    TimedOut(ureq::Timeout),
}

impl From<h2::Error> for Unsent {
    fn from(error: h2::Error) -> Self {
        Self::Frames(error)
    }
}

/// Open the stream, write the body, and wait for the answer's head, each
/// wait for window and the wait for the head bounded by `timeout` apiece, as
/// HTTP/1.1 bounds its phases apart.
async fn send(
    mut sender: SendRequest<Bytes>,
    request: ureq::http::Request<()>,
    payload: Option<Payload<'_>>,
    timeout: Duration,
) -> std::result::Result<ureq::http::Response<h2::RecvStream>, Unsent> {
    let empty = match &payload {
        None => true,
        Some(Payload::Bytes(bytes)) => bytes.is_empty(),
        Some(Payload::Reader { length, .. }) => *length == 0,
    };
    let (response, mut stream) = sender.send_request(request, empty)?;
    match payload {
        Some(Payload::Bytes(bytes)) if !bytes.is_empty() => {
            let mut rest = Bytes::copy_from_slice(bytes);
            while !rest.is_empty() {
                let granted = capacity(&mut stream, rest.len(), timeout).await?;
                let chunk = rest.split_to(granted);
                stream.send_data(chunk, rest.is_empty())?;
            }
        }
        Some(Payload::Reader { body, length }) if length > 0 => {
            // The reader is the caller's and this is the caller's thread:
            // reading it here blocks nothing but the request it feeds.
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
                let mut chunk = Bytes::copy_from_slice(&buffer[..read]);
                left -= read as u64;
                while !chunk.is_empty() {
                    let granted = capacity(&mut stream, chunk.len(), timeout).await?;
                    let piece = chunk.split_to(granted);
                    stream.send_data(piece, left == 0 && chunk.is_empty())?;
                }
            }
        }
        _ => {}
    }
    match runtime::within(timeout, response).await {
        Some(response) => Ok(response?),
        None => Err(Unsent::TimedOut(ureq::Timeout::RecvResponse)),
    }
}

/// Wait until the stream may send some of `wanted` bytes, at most
/// `timeout`; how many.
async fn capacity(
    stream: &mut h2::SendStream<Bytes>,
    wanted: usize,
    timeout: Duration,
) -> std::result::Result<usize, Unsent> {
    stream.reserve_capacity(wanted.min(SEND_CHUNK));
    loop {
        let granted = runtime::within(
            timeout,
            std::future::poll_fn(|context| stream.poll_capacity(context)),
        )
        .await
        .ok_or(Unsent::TimedOut(ureq::Timeout::SendBody))?;
        match granted {
            Some(Ok(0)) => {}
            Some(Ok(granted)) => return Ok(granted.min(wanted)),
            Some(Err(error)) => return Err(Unsent::Frames(error)),
            None => {
                return Err(Unsent::Body(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "the stream closed before its body was sent",
                )));
            }
        }
    }
}

/// The answer an HTTP/2 response is: its head read, its body left on the
/// stream, each read bounded by `timeout` and all of them by `until`.
fn answer(
    response: ureq::http::Response<h2::RecvStream>,
    timeout: Duration,
    until: Option<Instant>,
) -> std::result::Result<Answer, ureq::Error> {
    let (parts, recv) = response.into_parts();
    Ok(Answer {
        status: status_of(parts.status)?,
        version: HttpVersion::Http2,
        headers: headers_of(&parts.headers)?,
        body: Box::new(Body {
            recv,
            chunk: Bytes::new(),
            timeout,
            until,
            done: false,
        }),
        attempts: 1,
    })
}

/// A framed answer's status as the crate spells it.
pub(crate) fn status_of(
    status: ureq::http::StatusCode,
) -> std::result::Result<Status, ureq::Error> {
    Status::new(status.as_u16()).map_err(|error| ureq::Error::Other(Box::new(error)))
}

/// A framed answer's fields as the crate reads them, each value transcribed.
pub(crate) fn headers_of(fields: &HeaderMap) -> std::result::Result<Headers, ureq::Error> {
    let mut headers = Headers::new();
    for (name, value) in fields {
        let value = crate::Charset::Utf8.transcribe(value.as_bytes());
        headers
            .append(name.as_str(), &value)
            .map_err(|error| ureq::Error::Other(Box::new(error)))?;
    }
    Ok(headers)
}

/// An HTTP/2 answer's body, read on the caller's thread a frame at a time.
struct Body {
    recv: h2::RecvStream,
    /// What is left of the frame read last.
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
            match runtime::wait_for(bound, self.recv.data())? {
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "no HTTP/2 frame arrived within the timeout",
                    ));
                }
                Some(None) => self.done = true,
                Some(Some(Ok(chunk))) => {
                    // The window reopens as the bytes are taken off it.
                    let _ = self.recv.flow_control().release_capacity(chunk.len());
                    self.chunk = chunk;
                }
                Some(Some(Err(error))) => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        format!("http/2: {error}"),
                    ));
                }
            }
        }
    }
}
