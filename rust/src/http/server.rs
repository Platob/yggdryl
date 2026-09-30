//! A base HTTP/1.1 server hosting [`IOBase`](crate::IOBase) handles and
//! programmable routes, built on the same [`wire`](super::wire) grammar, [`Headers`],
//! [`Method`], [`Status`], [`Request`] and [`Response`] the client uses.
//!
//! It exists for two things: to host a holder over HTTP - a mounted
//! [`Holder`] answers `GET`, `HEAD`, `PUT`, `DELETE` and `OPTIONS` under a
//! prefix, with byte ranges, validators and conditionals - and to test every
//! generic client feature against a server the crate controls, which is what
//! [`Server::route`], [`Server::respond`], [`Server::inject`] and the request
//! log are for. One accept thread and one thread per connection; HTTP/1.1
//! keep-alive; request bodies framed by `Content-Length` or chunked. A
//! handler's answer is bytes in hand or a body written as it is sent
//! ([`Response::with_writer`]), and [`ServerOptions::with_trace`] writes
//! every exchange into a folder as the `message/http` documents it was.
//!
//! Faults, routes and mounts are looked up under one lock that is released
//! before any byte is served, and a leaf is never read whole: a child of a
//! mount streams through
//! [`IOBase::pstream_bytes`](crate::IOBase::pstream_bytes), and the holder
//! mounted at the prefix itself through one
//! [`IOBase::read_range_bytes`](crate::IOBase::read_range_bytes) per batch
//! under the mount's lock, so no socket write holds it.
//!
//! The server is hostable behind a reverse proxy or a redirect: every URL it
//! states - a handler's [`Request::url`], a mount's listing - is built on the
//! base the request was made under, which is the
//! [`ServerOptions::with_public_url`] it was given, else the forwarded fields
//! [`ServerOptions::with_forwarded_headers`] lists, read from a peer
//! [`ServerOptions::with_trusted_proxies`] names, else the request's own
//! `Host`; a prefix the proxy leaves on the path is stripped before routing
//! ([`ServerOptions::with_path_prefix`]); and a `GET` or `HEAD` of a route's
//! path with a trailing slash is redirected by a relative `Location`, so it
//! is right under any prefix.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use smol_str::format_smolstr;

use super::response::BodyWriter;
use super::wire::{MAX_LINE_BYTES, RequestHead};
use super::{Body, Headers, Method, Request, Response, Status};
use crate::holder::Holder;
use crate::{Error, MediaType, Result, Url};

mod connection;
pub(crate) mod forwarded;
#[cfg(feature = "http2")]
mod framed;
#[cfg(feature = "http2")]
mod h2;
#[cfg(feature = "http3")]
mod h3;
mod mount;
mod trace;

pub use forwarded::{ForwardedHeader, IpNetwork};

/// What a server answers with, before the connection frames it.
///
/// The status, the headers a handler or the mount stated, and a body that is
/// either bytes in hand or a leaf still to be streamed.
pub(super) struct Answer {
    pub(super) status: Status,
    pub(super) headers: Headers,
    pub(super) body: AnswerBody,
}

/// The body of an [`Answer`].
pub(super) enum AnswerBody {
    /// Bytes in hand: a route's answer, a listing, an error text.
    Bytes(Body),
    /// Written by a handler's writer as it is sent, its length unknown
    /// until it ends ([`Response::with_writer`]).
    Writer(BodyWriter),
    /// `length` bytes of a mounted leaf from `start`, streamed when written.
    Stream {
        source: Source,
        start: u64,
        length: u64,
    },
}

/// Where a streamed body is read from.
pub(super) enum Source {
    /// A child resolved out of a mount, owned by this answer; boxed because
    /// a `Holder` is the crate's widest enum and every other variant is a
    /// pointer.
    Owned(Box<Holder>),
    /// The mounted holder itself at the version the answer described, read
    /// under the mount's own lock one batch at a time.
    Root(Arc<RwLock<Mounted>>, u64),
}

impl Answer {
    /// An empty answer of `status`.
    pub(super) fn status(status: Status) -> Self {
        Self {
            status,
            headers: Headers::new(),
            body: AnswerBody::Bytes(Body::Empty),
        }
    }

    /// `text` under `text/plain; charset=utf-8` with `status`.
    pub(super) fn text(status: Status, text: &str) -> Self {
        Self::status(status)
            .with_header("content-type", "text/plain; charset=utf-8")
            .with_bytes(Body::from(text))
    }

    /// An error the server met while answering: `404` for an absence, else
    /// `500` with the error's text.
    pub(super) fn from_error(error: &Error) -> Self {
        match error {
            Error::Absent { .. } => Self::status(Status::NOT_FOUND),
            other => Self::text(Status::INTERNAL_SERVER_ERROR, &other.to_string()),
        }
    }

    /// A handler's response as an answer: its status, headers and body.
    pub(super) fn from_response(response: &Response) -> Self {
        // The body as the handler stated it: a coded body goes out coded,
        // under the `Content-Encoding` the handler set beside it, and a
        // written one is written into the connection, never into memory.
        let body = match response.take_writer() {
            Ok(Some(writer)) => AnswerBody::Writer(writer),
            Ok(None) => match response.raw_bytes() {
                Ok(bytes) => AnswerBody::Bytes(Body::Bytes(bytes)),
                Err(error) => return Self::from_error(&error),
            },
            Err(error) => return Self::from_error(&error),
        };
        Self {
            status: response.status(),
            headers: response.headers().clone(),
            body,
        }
    }

    /// This answer with one header set; a value the field grammar refuses
    /// is dropped, and every value the server itself renders passes it.
    pub(super) fn with_header(mut self, name: &str, value: &str) -> Self {
        let _ = self.headers.insert(name, value);
        self
    }

    /// This answer with `body` in hand.
    pub(super) fn with_bytes(mut self, body: Body) -> Self {
        self.body = AnswerBody::Bytes(body);
        self
    }
}

/// What one connection hands the server: the parsed head, the body it
/// framed, and the connection it came over.
pub(super) struct Incoming {
    pub(super) head: RequestHead,
    pub(super) body: Vec<u8>,
    /// The TCP or QUIC peer, when the socket names one.
    pub(super) peer: Option<SocketAddr>,
    /// Whether the connection itself is TLS, so the request was made under
    /// `https` as far as this server can see.
    pub(super) secure: bool,
    /// The `:scheme` a framed request states, `http` or `https`: over a
    /// cleartext connection a proxy's statement, believed only from a
    /// trusted peer; `None` over HTTP/1.
    pub(super) scheme: Option<&'static str>,
}

/// What the server decided for one request.
pub(super) enum Outcome {
    /// Write this answer, cut after `cut` body bytes when asked, and close
    /// the connection when cut.
    Answer { answer: Answer, cut: Option<u64> },
    /// Write nothing and close the connection.
    Close,
}

/// A handler a [`Server::route`] runs: the request in, a response out.
pub type Handler = dyn Fn(&Request) -> Result<Response> + Send + Sync + 'static;

/// What a connection does to one request instead of answering it plainly,
/// injected by [`Server::inject`] for a test to watch a client cope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Write the head and this many body bytes of the answer the route or
    /// mount gives, then close the connection.
    CutBodyAt(u64),
    /// Read the request and close the connection without a byte of answer.
    CloseBeforeAnswer,
    /// Answer `status` with an empty body and, when given, `Retry-After` in
    /// whole seconds.
    Refuse {
        /// The status answered.
        status: Status,
        /// The `Retry-After` delay, rendered in seconds.
        retry_after: Option<Duration>,
    },
    /// Sleep this long before answering as the route or mount would.
    Delay(Duration),
}

/// One request the server handled, as the log keeps it.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The method.
    pub method: Method,
    /// The request target exactly as sent, query included.
    pub target: String,
    /// The path, percent-decoded, without the query.
    pub path: String,
    /// The query pairs, percent-decoded, in wire order.
    pub query: Vec<(String, String)>,
    /// The request headers; a chunked body's trailers folded in and its
    /// framing replaced by the decoded `content-length`.
    pub headers: Headers,
    /// The request body's length in bytes; the body itself is not kept.
    pub body_len: u64,
    /// The status answered; `499` when the connection was closed before any
    /// byte of an answer ([`Fault::CloseBeforeAnswer`]).
    pub status: Status,
    /// The connection's peer, when the socket names one: the proxy, behind
    /// one.
    pub peer: Option<SocketAddr>,
    /// The client: the address a trusted proxy forwarded
    /// ([`ServerOptions::with_trusted_proxies`]), else the peer's.
    pub client: Option<IpAddr>,
}

impl Recorded {
    /// Whether the connection was closed before any byte of an answer.
    pub fn is_closed(&self) -> bool {
        self.status.code() == CLOSED_CODE
    }

    /// The bytes of text this entry holds, as
    /// [`Server::MAX_RECORDED_BYTES`] counts them.
    fn size(&self) -> usize {
        let pairs = |pairs: &mut dyn Iterator<Item = (&str, &str)>| {
            pairs
                .map(|(name, value)| name.len() + value.len())
                .sum::<usize>()
        };
        self.target.len()
            + self.path.len()
            + pairs(
                &mut self
                    .query
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            )
            + pairs(&mut self.headers.iter())
    }
}

/// The status a request closed without an answer is recorded under: nginx's
/// `499 Client Closed Request`, the one convention there is for it.
const CLOSED_CODE: u16 = 499;

/// How long the accept loop waits after the listener refuses a connection.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(10);

/// The methods a mount answers, as `Allow` lists them.
const ALLOW: &str = "GET, HEAD, PUT, DELETE, OPTIONS";

/// How a [`Server`] reads, frames and answers.
///
/// Every knob has its default in the signature and a `with_` setter.
///
/// ```
/// use std::time::Duration;
///
/// use yggdryl::http::ServerOptions;
///
/// let options = ServerOptions::default()
///     .with_read_timeout(Duration::from_secs(5))
///     .with_keep_alive(false);
/// assert_eq!(options.read_timeout(), Duration::from_secs(5));
/// assert!(!options.keep_alive());
/// assert_eq!(options.max_body_size(), 64 << 20);
/// assert!(options.server_header().starts_with("yggdryl/"));
/// ```
#[derive(Clone, Debug)]
pub struct ServerOptions {
    read_timeout: Duration,
    write_timeout: Duration,
    max_connections: usize,
    max_head_size: usize,
    max_body_size: u64,
    keep_alive: bool,
    recording: bool,
    etag: bool,
    tunnel: bool,
    #[cfg(feature = "http3")]
    http3: bool,
    server_header: String,
    /// Shared, since the options are copied and every connection writes
    /// under the one folder.
    trace: Option<Arc<Holder>>,
    public_url: Option<Url>,
    trusted_proxies: Vec<IpNetwork>,
    forwarded_headers: Vec<ForwardedHeader>,
    path_prefix: Option<String>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            read_timeout: Duration::from_secs(30),
            write_timeout: Duration::from_secs(30),
            max_connections: 512,
            max_head_size: MAX_LINE_BYTES * 8,
            max_body_size: 64 << 20,
            keep_alive: true,
            recording: true,
            etag: true,
            tunnel: false,
            #[cfg(feature = "http3")]
            http3: false,
            server_header: format!("yggdryl/{}", env!("CARGO_PKG_VERSION")),
            trace: None,
            public_url: None,
            trusted_proxies: Vec::new(),
            forwarded_headers: ForwardedHeader::DEFAULT.to_vec(),
            path_prefix: None,
        }
    }
}

impl ServerOptions {
    /// The longest read or write timeout [`Server::bind_with`] accepts: one
    /// day. A connection's deadline is the moment it starts plus the
    /// timeout, and a bound keeps that a moment the clock can name.
    pub const MAX_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

    /// How long a connection waits for the next byte of a request, and
    /// the longest one request head may take to arrive whole, before it is
    /// closed (30 seconds): a peer trickling a head a byte at a time holds
    /// its connection no longer than one that sends nothing. A tunnel quiet
    /// both ways this long is closed. Zero, and anything past
    /// [`Self::MAX_TIMEOUT`], is refused by [`Server::bind_with`].
    pub fn read_timeout(&self) -> Duration {
        self.read_timeout
    }

    /// This value with `read_timeout` set.
    pub fn with_read_timeout(mut self, read_timeout: Duration) -> Self {
        self.read_timeout = read_timeout;
        self
    }

    /// How long writing an answer waits for the peer to take more bytes
    /// before the connection is closed (30 seconds). Zero, and anything past
    /// [`Self::MAX_TIMEOUT`], is refused by [`Server::bind_with`].
    pub fn write_timeout(&self) -> Duration {
        self.write_timeout
    }

    /// This value with `write_timeout` set.
    pub fn with_write_timeout(mut self, write_timeout: Duration) -> Self {
        self.write_timeout = write_timeout;
        self
    }

    /// The most connections served at once, one thread each (512); a
    /// connection accepted past it is closed at once, unread.
    pub fn max_connections(&self) -> usize {
        self.max_connections
    }

    /// This value with `max_connections` set.
    pub fn with_max_connections(mut self, max_connections: usize) -> Self {
        self.max_connections = max_connections;
        self
    }

    /// The most bytes one request head - request line, field lines and the
    /// empty line - may take (64 KiB); a longer one is `431` and the
    /// connection closes. Within it the grammar's own bounds apply:
    /// [`MAX_LINE_BYTES`] per line and [`MAX_FIELD_LINES`](super::wire::MAX_FIELD_LINES)
    /// field lines.
    pub fn max_head_size(&self) -> usize {
        self.max_head_size
    }

    /// This value with `max_head_size` set.
    pub fn with_max_head_size(mut self, max_head_size: usize) -> Self {
        self.max_head_size = max_head_size;
        self
    }

    /// The largest request body accepted (64 MiB); a longer one is `413` and
    /// the connection closes.
    pub fn max_body_size(&self) -> u64 {
        self.max_body_size
    }

    /// This value with `max_body_size` set.
    pub fn with_max_body_size(mut self, max_body_size: u64) -> Self {
        self.max_body_size = max_body_size;
        self
    }

    /// Whether a connection stays open after an answer (`true`); `false`
    /// answers every request with `Connection: close`.
    pub fn keep_alive(&self) -> bool {
        self.keep_alive
    }

    /// This value with `keep_alive` set.
    pub fn with_keep_alive(mut self, keep_alive: bool) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    /// Whether handled requests are kept in the log [`Server::requests`]
    /// answers (`true`), the newest [`Server::MAX_RECORDED`] of them;
    /// [`Server::request_count`] counts either way.
    pub fn recording(&self) -> bool {
        self.recording
    }

    /// This value with `recording` set.
    pub fn with_recording(mut self, recording: bool) -> Self {
        self.recording = recording;
        self
    }

    /// Whether a mounted leaf is served with an `ETag` (`true`): the quoted
    /// lower-case hex of its XXH3-64 digest.
    pub fn etag(&self) -> bool {
        self.etag
    }

    /// This value with `etag` set.
    pub fn with_etag(mut self, etag: bool) -> Self {
        self.etag = etag;
        self
    }

    /// Whether a `CONNECT host:port` is tunnelled to that address (`false`):
    /// the server then stands in for a forward proxy, which is how a
    /// client's proxy handling is tested against a real one. Off, a
    /// `CONNECT` is `405`.
    pub fn tunnel(&self) -> bool {
        self.tunnel
    }

    /// This value with `tunnel` set.
    pub fn with_tunnel(mut self, tunnel: bool) -> Self {
        self.tunnel = tunnel;
        self
    }

    /// Whether HTTP/3 is answered on the UDP port beside the TCP one
    /// (`false`), under a certificate the server signs for itself
    /// ([`Server::certificate`]), and advertised in `Alt-Svc` on every
    /// HTTP/1.1 and HTTP/2 answer. The TCP port then also answers TLS under
    /// that certificate, offering HTTP/2 by ALPN, so the origin is reached as
    /// `https` and upgraded to HTTP/3 as a public one is. HTTP/2 in the clear
    /// needs no switch: a connection that opens with its preface is served
    /// as HTTP/2 whenever the build has it.
    #[cfg(feature = "http3")]
    pub fn http3(&self) -> bool {
        self.http3
    }

    /// This value with `http3` set.
    #[cfg(feature = "http3")]
    pub fn with_http3(mut self, http3: bool) -> Self {
        self.http3 = http3;
        self
    }

    /// The `Server` header on every answer (`yggdryl/<version>`).
    pub fn server_header(&self) -> &str {
        &self.server_header
    }

    /// This value with `server_header` set.
    pub fn with_server_header(mut self, server_header: impl Into<String>) -> Self {
        self.server_header = server_header.into();
        self
    }

    /// The folder every exchange is written into, when the server traces:
    /// `NNNN-request.http` and `NNNN-response.http`, `message/http`
    /// documents numbered from `0000` in the order the requests' heads
    /// arrived, across every connection. An HTTP/1.1 exchange is written byte for
    /// byte as it went over the wire - a request refused on its head, an
    /// interim `100 Continue` and a chunked framing included - and an
    /// HTTP/2 or HTTP/3 one as the HTTP/1.1 message its frames carried; a
    /// connection that goes away before sending a byte wrote nothing, and a
    /// tunnel's bytes past its `200` are not an exchange. An answer is
    /// appended as it goes out, so a streamed one is never held whole for
    /// its copy; a write to the folder that fails closes the connection
    /// after the answer rather than trace on past a gap.
    pub fn trace(&self) -> Option<&Holder> {
        self.trace.as_deref()
    }

    /// This value tracing into `folder` (a container, whose children are
    /// created as the exchanges begin).
    #[must_use]
    pub fn with_trace(mut self, folder: Holder) -> Self {
        self.trace = Some(Arc::new(folder));
        self
    }

    /// The URL clients reach this server at, when one was stated (`None`):
    /// behind a reverse proxy or a redirect, the scheme, host, port and path
    /// prefix the proxy answers on. Set, it is the base of every URL the
    /// server states - a handler's [`Request::url`], a mount's listing,
    /// [`Server::public_url_of`] - whatever the request's own `Host` or
    /// forwarded fields say, and no `Alt-Svc` is advertised, since the
    /// proxy's origin is not this server's. Unset, the base is read from
    /// the request ([`Self::with_trusted_proxies`]).
    pub fn public_url(&self) -> Option<&Url> {
        self.public_url.as_ref()
    }

    /// This value stating the public URL: `http` or `https`, a host, a port
    /// when it is not the scheme's own, and the path prefix the proxy adds
    /// in front of the routed path (`https://data.example.com/olap`). A
    /// query, a fragment or user information is refused by
    /// [`Server::bind_with`], as is any other scheme: every URL the server
    /// states is built on this one, and a credential in it would reach every
    /// client.
    ///
    /// ```
    /// use yggdryl::Url;
    /// use yggdryl::http::ServerOptions;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let options = ServerOptions::default()
    ///     .with_public_url(Url::from_str("https://data.example.com/olap")?);
    /// assert_eq!(
    ///     options.public_url().map(ToString::to_string).as_deref(),
    ///     Some("https://data.example.com/olap")
    /// );
    /// assert_eq!(ServerOptions::default().public_url(), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_public_url(mut self, url: Url) -> Self {
        self.public_url = Some(url);
        self
    }

    /// The peers whose forwarded fields are believed (none), and of those
    /// only the ones [`Self::forwarded_headers`] lists: a request from any
    /// other peer is taken as it arrived, so a client cannot state a host or
    /// a scheme of its own choosing. A `Forwarded` or `X-Forwarded-For`
    /// chain is walked from the element nearest this server past every
    /// address that is itself trusted; the first that is not is the client,
    /// and the host and scheme it stated are the request's. The `:scheme` of
    /// an HTTP/2 request in the clear is believed from these peers alone.
    pub fn trusted_proxies(&self) -> &[IpNetwork] {
        &self.trusted_proxies
    }

    /// This value trusting `networks`: each an IP address or a CIDR network
    /// (`10.0.0.5`, `10.0.0.0/8`, `::1`, `fd00::/8`), an IPv4-mapped IPv6
    /// peer matched as the IPv4 address it maps.
    ///
    /// ```
    /// use std::net::IpAddr;
    ///
    /// use yggdryl::http::ServerOptions;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let options = ServerOptions::default().with_trusted_proxies(["10.0.0.0/8", "::1"])?;
    /// let trusted: Vec<String> = options
    ///     .trusted_proxies()
    ///     .iter()
    ///     .map(ToString::to_string)
    ///     .collect();
    /// assert_eq!(trusted, ["10.0.0.0/8", "::1/128"]);
    /// assert!(options.trusted_proxies()[0].contains("10.20.30.40".parse::<IpAddr>().unwrap()));
    /// assert!(ServerOptions::default().with_trusted_proxies(["pub.example"]).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with target `ip network` for an entry that
    /// is neither ([`IpNetwork::from_str`]).
    pub fn with_trusted_proxies<I, S>(mut self, networks: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.trusted_proxies = networks
            .into_iter()
            .map(|network| IpNetwork::from_str(network.as_ref()))
            .collect::<Result<Vec<_>>>()?;
        Ok(self)
    }

    /// The forwarded fields read from a trusted peer
    /// ([`ForwardedHeader::DEFAULT`]: `X-Forwarded-For` and
    /// `X-Forwarded-Proto`, which every documented proxy sets on every
    /// request). A proxy passes a field it does not set through as the
    /// client wrote it, so a field listed here that the proxy in front does
    /// not set or overwrite is the client's to choose.
    pub fn forwarded_headers(&self) -> &[ForwardedHeader] {
        &self.forwarded_headers
    }

    /// This value reading exactly `headers` from a trusted peer, in place of
    /// the default two: list a field only when the proxy in front sets or
    /// overwrites it on every request it forwards. `Forwarded` is read
    /// before the `X-Forwarded-*` field stating the same fact; an empty list
    /// reads none, the trusted peer's address then being the client.
    ///
    /// ```
    /// use yggdryl::http::{ForwardedHeader, ServerOptions};
    ///
    /// let options = ServerOptions::default().with_forwarded_headers([
    ///     ForwardedHeader::XForwardedPrefix,
    ///     ForwardedHeader::XForwardedFor,
    ///     ForwardedHeader::XForwardedProto,
    /// ]);
    /// assert_eq!(
    ///     options.forwarded_headers(),
    ///     [
    ///         ForwardedHeader::XForwardedFor,
    ///         ForwardedHeader::XForwardedProto,
    ///         ForwardedHeader::XForwardedPrefix,
    ///     ]
    /// );
    /// assert_eq!(ServerOptions::default().forwarded_headers(), ForwardedHeader::DEFAULT);
    /// ```
    #[must_use]
    pub fn with_forwarded_headers(
        mut self,
        headers: impl IntoIterator<Item = ForwardedHeader>,
    ) -> Self {
        let mut headers: Vec<ForwardedHeader> = headers.into_iter().collect();
        headers.sort_unstable();
        headers.dedup();
        self.forwarded_headers = headers;
        self
    }

    /// The path prefix stripped off a request before it is routed (`None`):
    /// `/olap` when a proxy forwards `/olap/xmla` to a server that routes
    /// `/xmla`. A request outside the prefix is routed as it is, so the
    /// routes answer with and without it, and a URL the server states for a
    /// request under it carries the prefix back.
    pub fn path_prefix(&self) -> Option<&str> {
        self.path_prefix.as_deref()
    }

    /// This value stripping `prefix`, spelled as a route path is; `/` is no
    /// prefix at all.
    ///
    /// ```
    /// use yggdryl::http::ServerOptions;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let options = ServerOptions::default().with_path_prefix("/olap/")?;
    /// assert_eq!(options.path_prefix(), Some("/olap"));
    /// assert_eq!(ServerOptions::default().with_path_prefix("/")?.path_prefix(), None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a prefix holding a query, a fragment or a
    /// control byte.
    pub fn with_path_prefix(mut self, prefix: &str) -> Result<Self> {
        let prefix = normalize_path(prefix)?;
        self.path_prefix = (prefix != "/").then_some(prefix);
        Ok(self)
    }
}

/// A mounted holder and the media types `PUT` declared for its children.
///
/// A media type a `PUT` states in `Content-Type` is set on the leaf and kept
/// here besides, because a backend that infers the type from the name -
/// every filesystem - forgets a `set_media_type` once the handle is dropped,
/// and the next `GET` resolves a fresh one.
pub(super) struct Mounted {
    pub(super) holder: Holder,
    pub(super) declared: BTreeMap<String, MediaType>,
    /// How many writes the mounted holder itself took: a body streamed from
    /// it batch by batch stops where this moves, rather than splicing two
    /// versions under the first one's length and validator.
    pub(super) version: u64,
}

/// One mount: a normalized prefix and what is served under it.
struct Mount {
    prefix: String,
    shared: Arc<RwLock<Mounted>>,
}

/// One injected fault and how many requests it still applies to.
struct Injected {
    path: String,
    fault: Fault,
    /// `0` is every request.
    remaining: u32,
}

/// What every connection consults under one lock.
#[derive(Default)]
struct State {
    /// Longest prefix first.
    mounts: Vec<Mount>,
    routes: BTreeMap<(Option<Method>, String), Route>,
    faults: Vec<Injected>,
    recorded: VecDeque<Recorded>,
    /// What the recorded heads hold, as [`Recorded::size`] counts it.
    recorded_bytes: usize,
    recording: bool,
}

impl State {
    /// The next fault of `path`, consumed when it was counted.
    fn take_fault(&mut self, path: &str) -> Option<Fault> {
        let index = self.faults.iter().position(|fault| fault.path == path)?;
        let fault = self.faults[index].fault.clone();
        if self.faults[index].remaining == 1 {
            self.faults.remove(index);
        } else if self.faults[index].remaining > 1 {
            self.faults[index].remaining -= 1;
        }
        Some(fault)
    }

    /// The route of `method` at `path`, the exact method before the
    /// any-method one; a `HEAD` no route names is answered by the `GET`
    /// route, since it is that `GET` without its body (RFC 9110 9.3.2).
    fn route(&self, method: Method, path: &str) -> Option<Route> {
        let exact = |method: Method| self.routes.get(&(Some(method), path.to_owned()));
        exact(method)
            .or_else(|| self.routes.get(&(None, path.to_owned())))
            .or_else(|| {
                (method == Method::Head)
                    .then(|| exact(Method::Get))
                    .flatten()
            })
            .cloned()
    }

    /// The methods routed at `path`, as a `405` states them in `Allow`:
    /// empty when no route names the path, `HEAD` beside a `GET`.
    fn allowed(&self, path: &str) -> Vec<Method> {
        let mut allowed: Vec<Method> = Method::ALL
            .into_iter()
            .filter(|method| self.routes.contains_key(&(Some(*method), path.to_owned())))
            .collect();
        if allowed.contains(&Method::Get) && !allowed.contains(&Method::Head) {
            let at = allowed
                .iter()
                .position(|method| *method == Method::Get)
                .map_or(0, |get| get + 1);
            allowed.insert(at, Method::Head);
        }
        allowed
    }

    /// The longest mount whose prefix covers `path`, and the path below it.
    fn mount<'path>(&self, path: &'path str) -> Option<(&Mount, &'path str)> {
        self.mounts.iter().find_map(|mount| {
            let rest = path_below(&mount.prefix, path)?;
            Some((mount, rest))
        })
    }

    /// What `path`, spelled with one trailing slash, finds at the route
    /// path without it: a `GET` or `HEAD` is sent there by a relative
    /// `Location` - `../name`, the last segment as it was sent, the query
    /// kept - which resolves right under any prefix a proxy adds; any other
    /// method is served by that route, since a client that was redirected
    /// may not repeat its body, and a path routed for other methods alone
    /// is `405` there. `None` when nothing is routed at either spelling.
    fn slashed(
        &self,
        method: Method,
        path: &str,
        raw_last: &str,
        raw_query: &str,
    ) -> Option<Found> {
        let bare = path.strip_suffix('/').filter(|bare| !bare.is_empty())?;
        if bare.ends_with('/') || raw_last.is_empty() {
            return None;
        }
        let route = self.route(method, bare);
        let allowed = self.allowed(bare);
        if route.is_none() && allowed.is_empty() {
            return None;
        }
        if matches!(method, Method::Get | Method::Head) {
            return Some(Found::Redirect(format!("../{raw_last}{raw_query}")));
        }
        Some(match route {
            Some(route) => Found::Route(route),
            None => Found::NotAllowed(allowed),
        })
    }
}

/// What the accept thread and every connection thread share.
pub(super) struct Inner {
    address: SocketAddr,
    url: Url,
    pub(super) options: ServerOptions,
    stopping: AtomicBool,
    connections: AtomicU64,
    /// Connections being served now, against `max_connections`.
    live: AtomicUsize,
    count: AtomicUsize,
    state: Mutex<State>,
    /// Where exchanges are written, when the options trace.
    trace: Option<trace::Trace>,
    /// The QUIC side, when the options answer HTTP/3.
    #[cfg(feature = "http3")]
    h3: Option<h3::Listener>,
}

impl Inner {
    /// The state, poison ignored: a panicking handler must not take the
    /// server down with it.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The TLS the TCP port answers, when the server holds a certificate.
    #[cfg(feature = "http3")]
    pub(super) fn tls(&self) -> Option<Arc<rustls::ServerConfig>> {
        self.h3.as_ref().map(|h3| Arc::clone(&h3.tcp))
    }

    /// Decide one request: the fault, the route or the mount, then the log.
    pub(super) fn dispatch(&self, incoming: Incoming) -> Outcome {
        let routed = self.route_of(&incoming);
        self.answer_routed(routed, incoming)
    }

    /// Where `incoming` goes - its fault taken, its route or mount found -
    /// with nothing run that may block.
    ///
    /// The target is joined onto the server's own URL, which resolves its
    /// dot segments, and decoded; the options' path prefix comes off the
    /// decoded path when it lies under it; and the request's URL is built on
    /// the base the request was made under ([`forwarded::resolve`]), the
    /// routed path and the query spelled as they were sent.
    pub(super) fn route_of(&self, incoming: &Incoming) -> Routed {
        let target = &incoming.head.target;
        let absolute = !target.starts_with('/') && target.contains("://");
        let joined = self
            .url
            .join_reference(target)
            .and_then(|url| decoded_path(&url).map(|path| (url, path)));
        let (url, path, query, raw_last, raw_query, client, trusted) = match joined {
            Ok((url, decoded)) => {
                let (routed, under_prefix) = match &self.options.path_prefix {
                    Some(prefix) => match path_below(prefix, &decoded) {
                        Some(rest) => (format!("/{rest}"), true),
                        None => (decoded.clone(), false),
                    },
                    None => (decoded.clone(), false),
                };
                let dropped = if under_prefix {
                    self.options
                        .path_prefix
                        .as_deref()
                        .map_or(0, |prefix| prefix.matches('/').count())
                } else {
                    0
                };
                // The prefix goes back on the URL as it was sent, escapes
                // and all: the options hold it decoded, as routes are.
                let (raw_prefix, raw_path) = url
                    .path_text(false)
                    .map(|raw| {
                        let (prefix, rest) = split_leading_segments(&raw, dropped);
                        (prefix.to_owned(), rest)
                    })
                    .unwrap_or_else(|_| (String::new(), routed.clone()));
                let addressed = forwarded::resolve(
                    &self.options,
                    incoming,
                    absolute.then_some(&url),
                    &self.url,
                    &raw_prefix,
                );
                let raw_query = url
                    .query(false)
                    .ok()
                    .flatten()
                    .map(|query| format!("?{query}"))
                    .unwrap_or_default();
                let raw_last = raw_path
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                let effective = Url::from_str(&format!(
                    "{}://{}{}{raw_path}{raw_query}",
                    addressed.scheme, addressed.authority, addressed.prefix
                ));
                let query = url
                    .parameters(true)
                    .map(|parameters| {
                        parameters
                            .iter()
                            .map(|(name, value)| (name.to_owned(), value.to_owned()))
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    effective,
                    routed,
                    query,
                    raw_last,
                    raw_query,
                    addressed.client,
                    addressed.trusted,
                )
            }
            Err(error) => (
                Err(error),
                String::new(),
                Vec::new(),
                String::new(),
                String::new(),
                incoming.peer.map(|peer| peer.ip().to_canonical()),
                false,
            ),
        };
        let (fault, found) = {
            let mut state = self.state();
            let fault = state.take_fault(&path);
            let found = match state.route(incoming.head.method, &path) {
                Some(route) => Some(Found::Route(route)),
                None => match state.mount(&path) {
                    Some((mount, rest)) => Some(Found::Mount {
                        shared: Arc::clone(&mount.shared),
                        rest: rest.to_owned(),
                    }),
                    // A path routed for other methods is there: the method
                    // is what it does not answer (RFC 9110 15.5.6).
                    None => {
                        let allowed = state.allowed(&path);
                        if allowed.is_empty() {
                            state.slashed(incoming.head.method, &path, &raw_last, &raw_query)
                        } else {
                            Some(Found::NotAllowed(allowed))
                        }
                    }
                },
            };
            (fault, found)
        };
        Routed {
            url,
            path,
            query,
            fault,
            found,
            client,
            trusted,
        }
    }

    /// Answer `incoming` where `routed` sends it: a pause, a handler or a
    /// mounted leaf may block here ([`Routed::may_block`]).
    pub(super) fn answer_routed(&self, routed: Routed, incoming: Incoming) -> Outcome {
        let Routed {
            url,
            path,
            query,
            fault,
            found,
            client,
            trusted,
        } = routed;
        let mut cut = None;
        match fault {
            Some(Fault::CloseBeforeAnswer) => {
                self.record(
                    &incoming,
                    &path,
                    query,
                    Status::new(CLOSED_CODE).unwrap_or(Status::INTERNAL_SERVER_ERROR),
                    client,
                );
                return Outcome::Close;
            }
            Some(Fault::Refuse {
                status,
                retry_after,
            }) => {
                let mut answer = Answer::status(status);
                if let Some(pause) = retry_after {
                    answer = answer.with_header("retry-after", &pause.as_secs().to_string());
                }
                self.record(&incoming, &path, query, status, client);
                return Outcome::Answer { answer, cut };
            }
            Some(Fault::Delay(pause)) => std::thread::sleep(pause),
            Some(Fault::CutBodyAt(at)) => cut = Some(at),
            None => {}
        }
        let answer = match (url, found) {
            (Err(error), _) => Answer::text(Status::BAD_REQUEST, &error.to_string()),
            (Ok(_), None) => Answer::status(Status::NOT_FOUND),
            (Ok(_), Some(Found::Route(Route::Fixed(response)))) => Answer::from_response(&response),
            (Ok(url), Some(Found::Route(Route::Handler(handler)))) => {
                let request = Request::new(incoming.head.method, url)
                    .with_headers(incoming.head.headers.clone())
                    .with_body(Body::from(incoming.body.as_slice()));
                match handler(&request) {
                    Ok(response) => Answer::from_response(&response),
                    Err(error) => Answer::text(Status::INTERNAL_SERVER_ERROR, &error.to_string()),
                }
            }
            (Ok(url), Some(Found::Mount { shared, rest })) => {
                mount::serve(&shared, &rest, &incoming, &url, &self.options)
            }
            (Ok(_), Some(Found::Redirect(location))) => {
                Answer::status(Status::PERMANENT_REDIRECT).with_header("location", &location)
            }
            (Ok(_), Some(Found::NotAllowed(allowed))) => {
                let allow = allowed
                    .iter()
                    .map(|method| method.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                Answer::text(
                    Status::METHOD_NOT_ALLOWED,
                    &format!(
                        "{} is not answered at {path}; {allow} are",
                        incoming.head.method
                    ),
                )
                .with_header("allow", &allow)
            }
        };
        self.record(&incoming, &path, query, answer.status, client);
        #[cfg(feature = "http3")]
        let answer = match &self.h3 {
            // Where else this origin answers, for a client to go there next.
            // A handler's own `Alt-Svc` stands; behind a stated public URL or
            // a trusted proxy the origin is the proxy's, which does not answer
            // on this port.
            Some(h3)
                if incoming.head.version != super::HttpVersion::Http3
                    && !answer.headers.contains_key("alt-svc")
                    && self.options.public_url.is_none()
                    && !trusted =>
            {
                answer.with_header("alt-svc", &format!("h3=\":{}\"; ma=86400", h3.port))
            }
            _ => answer,
        };
        #[cfg(not(feature = "http3"))]
        let _ = trusted;
        Outcome::Answer { answer, cut }
    }

    /// Count the request and, when recording, log it under `status`.
    pub(super) fn record(
        &self,
        incoming: &Incoming,
        path: &str,
        query: Vec<(String, String)>,
        status: Status,
        client: Option<IpAddr>,
    ) {
        self.count.fetch_add(1, Ordering::Relaxed);
        let mut state = self.state();
        if !state.recording {
            return;
        }
        let recorded = Recorded {
            method: incoming.head.method,
            target: incoming.head.target.clone(),
            path: path.to_owned(),
            query,
            headers: incoming.head.headers.clone(),
            body_len: incoming.body.len() as u64,
            status,
            peer: incoming.peer,
            client,
        };
        let size = recorded.size();
        while state.recorded.len() == Server::MAX_RECORDED
            || (!state.recorded.is_empty()
                && state.recorded_bytes + size > Server::MAX_RECORDED_BYTES)
        {
            if let Some(dropped) = state.recorded.pop_front() {
                state.recorded_bytes -= dropped.size();
            }
        }
        state.recorded_bytes += size;
        state.recorded.push_back(recorded);
    }
}

/// What a route answers.
#[derive(Clone)]
enum Route {
    /// The same response every time ([`Server::respond`]).
    Fixed(Arc<Response>),
    /// Whatever the handler answers ([`Server::route`]).
    Handler(Arc<Handler>),
}

/// What the lookup found for a path.
enum Found {
    Route(Route),
    Mount {
        shared: Arc<RwLock<Mounted>>,
        rest: String,
    },
    /// Routes for other methods, which a `405` names.
    NotAllowed(Vec<Method>),
    /// A `308` to this relative `Location`: the route's path without the
    /// trailing slash the request spelled ([`State::slashed`]).
    Redirect(String),
}

/// Where one request goes, decided under the state's lock and answered
/// after it ([`Inner::route_of`], [`Inner::answer_routed`]).
pub(super) struct Routed {
    /// The request's URL on the base it was made under, or why the target
    /// would not resolve.
    url: Result<Url>,
    path: String,
    query: Vec<(String, String)>,
    fault: Option<Fault>,
    found: Option<Found>,
    /// The client, for the log.
    client: Option<IpAddr>,
    /// Whether the peer is a trusted proxy.
    trusted: bool,
}

impl Routed {
    /// Whether answering may block the thread: a pause a fault asked for,
    /// a handler, or a mounted leaf's reads. A fixed answer, a refusal and
    /// an absence do not.
    #[cfg(feature = "http2")]
    pub(super) fn may_block(&self) -> bool {
        matches!(self.fault, Some(Fault::Delay(_)))
            || matches!(
                self.found,
                Some(Found::Route(Route::Handler(_)) | Found::Mount { .. })
            )
    }
}

/// The path below `prefix`, when `prefix` covers `path`: the root prefix
/// covers everything, and another one covers itself and the paths under a
/// slash after it.
fn path_below<'path>(prefix: &str, path: &'path str) -> Option<&'path str> {
    if prefix == "/" {
        return Some(path.strip_prefix('/').unwrap_or(path));
    }
    let rest = path.strip_prefix(prefix)?;
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix('/')
}

/// The raw path `raw` split after its first `count` segments, both halves
/// spelled as they were sent: `/my%20olap/x%20y` after one is `/my%20olap`
/// and `/x%20y`, and a path of nothing but those segments leaves `/`.
fn split_leading_segments(raw: &str, count: usize) -> (&str, String) {
    let mut rest = raw;
    for _ in 0..count {
        rest = rest.strip_prefix('/').unwrap_or(rest);
        rest = rest.find('/').map_or("", |at| &rest[at..]);
    }
    let prefix = &raw[..raw.len() - rest.len()];
    if rest.is_empty() {
        (prefix, "/".to_owned())
    } else {
        (prefix, rest.to_owned())
    }
}

/// The canonical spelling of a mount prefix or a route path: one leading
/// slash, no trailing slash except on the root.
///
/// # Errors
///
/// Returns [`Error::Parse`] with target `http path` for a query, a fragment
/// or a control byte in it.
pub(crate) fn normalize_path(path: &str) -> Result<String> {
    if let Some(position) = path
        .bytes()
        .position(|byte| byte == b'?' || byte == b'#' || byte < b' ' || byte == 0x7f)
    {
        return Err(Error::Parse {
            target: "http path",
            position,
            reason: format_smolstr!(
                "a server path holds no query, fragment or control byte, got {path:?}"
            ),
        });
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok("/".to_owned());
    }
    Ok(if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    })
}

/// A loopback-or-anywhere HTTP/1.1 server hosting holders and routes.
///
/// Binding starts accepting; dropping the server stops it. Every method
/// takes `&self`, so a test edits the routes and reads the log while
/// connections are being served.
///
/// ```
/// use yggdryl::holder::Holder;
/// use yggdryl::http::{Request, Server, Status};
/// use yggdryl::{IOBase, Url};
///
/// # fn main() -> yggdryl::Result<()> {
/// let server = Server::bind("127.0.0.1:0")?;
/// let mut folder = Holder::Buffer(yggdryl::holder::Buffer::new());
/// folder.write_all_bytes(b"[1, 2, 3]")?;
/// server.mount("/rows.json", folder)?;
///
/// let response = Request::get(&server.url_of("/rows.json")?.to_string())?.send()?;
/// assert_eq!(response.status(), Status::OK);
/// assert_eq!(&*response.bytes()?, b"[1, 2, 3]");
/// assert_eq!(server.request_count(), 1);
/// # Ok(())
/// # }
/// ```
pub struct Server {
    inner: Arc<Inner>,
    accept: Option<JoinHandle<()>>,
}

/// One admitted connection, given back when its thread ends however it ends.
struct Live<'a>(&'a AtomicUsize);

impl Drop for Live<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Server {
    /// The most requests [`requests`](Self::requests) keeps, the oldest
    /// dropped first: a server left recording under load holds a bounded
    /// log rather than every request it ever answered.
    pub const MAX_RECORDED: usize = 65_536;
    /// The most bytes of heads - targets, paths, query pairs and fields -
    /// the log keeps, the oldest request dropped first: 64 MiB, so heads of
    /// the largest size the options admit cannot fill the count's bound
    /// with gibibytes.
    pub const MAX_RECORDED_BYTES: usize = 64 << 20;

    /// Bind `address` (`127.0.0.1:0` for any loopback port) with the default
    /// options and start accepting.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the listener cannot be bound.
    pub fn bind(address: &str) -> Result<Self> {
        Self::bind_with(address, ServerOptions::default())
    }

    /// Bind `address` with `options` and start accepting.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when a read or write timeout is zero or past
    /// [`ServerOptions::MAX_TIMEOUT`], or the public URL is not a plain
    /// `http` or `https` origin with a path and no user information,
    /// [`Error::Io`] when the listener cannot be bound, or a parse error
    /// when the bound address is not a URL host.
    pub fn bind_with(address: &str, options: ServerOptions) -> Result<Self> {
        let refuse = |reason: smol_str::SmolStr| Error::Parse {
            target: "http server option",
            position: 0,
            reason,
        };
        for (name, timeout) in [
            ("read_timeout", options.read_timeout),
            ("write_timeout", options.write_timeout),
        ] {
            if timeout.is_zero() {
                return Err(refuse(format_smolstr!(
                    "{name}: expected a positive duration, got zero, which no read or write meets"
                )));
            }
            if timeout > ServerOptions::MAX_TIMEOUT {
                return Err(refuse(format_smolstr!(
                    "{name}: expected at most {} seconds, got {} seconds",
                    ServerOptions::MAX_TIMEOUT.as_secs(),
                    timeout.as_secs_f64()
                )));
            }
        }
        if let Some(public) = &options.public_url {
            let scheme = public.scheme().as_str();
            if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
                return Err(refuse(format_smolstr!(
                    "public_url: expected an http or https URL, got {public}"
                )));
            }
            if public.query(false)?.is_some() || public.fragment(false)?.is_some() {
                return Err(refuse(format_smolstr!(
                    "public_url: expected a URL with no query or fragment, got {public}"
                )));
            }
            // Every URL the server states is built on this one, so a
            // credential in it would be handed to every client; the URL is
            // not repeated, for the same reason.
            if public.authority().user().is_some() {
                return Err(refuse(format_smolstr!(
                    "public_url: expected a URL with no user information, got one for host {}",
                    public.authority().host_port()
                )));
            }
        }
        let listener = TcpListener::bind(address)?;
        let address = listener.local_addr()?;
        let url = Url::from_str(&format!("http://{}/", loopback_of(address)))?;
        #[cfg(feature = "http3")]
        let h3 = if options.http3 {
            Some(h3::Listener::bind(address, options.read_timeout)?)
        } else {
            None
        };
        let inner = Arc::new(Inner {
            address,
            url,
            stopping: AtomicBool::new(false),
            connections: AtomicU64::new(0),
            live: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
            state: Mutex::new(State {
                recording: options.recording,
                ..State::default()
            }),
            trace: options.trace.clone().map(trace::Trace::new),
            options,
            #[cfg(feature = "http3")]
            h3,
        });
        #[cfg(feature = "http3")]
        if let Some(h3) = &inner.h3 {
            h3.accept(&inner)?;
        }
        let shared = Arc::clone(&inner);
        let accept = std::thread::spawn(move || {
            for connection in listener.incoming() {
                if shared.stopping.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = connection else {
                    // Out of descriptors, most likely: waiting a moment for
                    // connections to close beats spinning on the refusal.
                    std::thread::sleep(ACCEPT_BACKOFF);
                    continue;
                };
                shared.connections.fetch_add(1, Ordering::Relaxed);
                // Past the cap the stream is dropped here, closing it unread:
                // a thread per connection is only bounded if this is.
                // `fetch_update` is the spelling the declared MSRV, Rust
                // 1.94, knows; its rename `try_update` came later.
                #[allow(deprecated)]
                let admitted = shared
                    .live
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
                        (live < shared.options.max_connections).then_some(live + 1)
                    })
                    .is_ok();
                if admitted {
                    let inner = Arc::clone(&shared);
                    let spawned = std::thread::Builder::new()
                        .name("yggdryl-http-connection".to_owned())
                        .spawn(move || {
                            let _live = Live(&inner.live);
                            connection::serve(&inner, stream);
                        });
                    // No thread to serve on: the stream closed with the
                    // closure, and its slot is given back.
                    if spawned.is_err() {
                        shared.live.fetch_sub(1, Ordering::AcqRel);
                    }
                }
            }
        });
        Ok(Self {
            inner,
            accept: Some(accept),
        })
    }

    /// The bound address.
    pub fn address(&self) -> SocketAddr {
        self.inner.address
    }

    /// The bound port.
    pub fn port(&self) -> u16 {
        self.inner.address.port()
    }

    /// `http://<address>/`, the bound address with an unspecified one
    /// (`0.0.0.0`, `[::]`) replaced by its loopback, so the URL is one a
    /// process on this host can dial.
    pub fn url(&self) -> &Url {
        &self.inner.url
    }

    /// The certificate HTTP/3 is answered under, as PEM, when the options
    /// turned it on: a client verifies the server against it by naming it
    /// as its CA bundle.
    #[cfg(feature = "http3")]
    pub fn certificate(&self) -> Option<&str> {
        self.inner.h3.as_ref().map(|h3| h3.certificate.as_str())
    }

    /// `path` resolved against [`url`](Self::url).
    ///
    /// # Errors
    ///
    /// Returns what [`Url::join_reference`] refuses.
    pub fn url_of(&self, path: &str) -> Result<Url> {
        self.inner.url.join_reference(path)
    }

    /// The URL clients reach `path` at: under the options'
    /// [`public_url`](ServerOptions::public_url) when one was stated - its
    /// path is the prefix the proxy adds, so `/xmla` under
    /// `https://data.example.com/olap` is `https://data.example.com/olap/xmla`
    /// - else [`url_of`](Self::url_of).
    ///
    /// ```
    /// use yggdryl::Url;
    /// use yggdryl::http::{Server, ServerOptions};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let options = ServerOptions::default()
    ///     .with_public_url(Url::from_str("https://data.example.com/olap")?);
    /// let server = Server::bind_with("127.0.0.1:0", options)?;
    /// assert_eq!(
    ///     server.public_url_of("/xmla")?.to_string(),
    ///     "https://data.example.com/olap/xmla"
    /// );
    /// assert!(server.url_of("/xmla")?.to_string().starts_with("http://127.0.0.1:"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what the URL grammar refuses in `path`.
    pub fn public_url_of(&self, path: &str) -> Result<Url> {
        let Some(public) = &self.inner.options.public_url else {
            return self.url_of(path);
        };
        let base = public.to_string();
        let base = base.trim_end_matches('/');
        let separator = if path.starts_with('/') { "" } else { "/" };
        Url::from_str(&format!("{base}{separator}{path}"))
    }

    /// The options the server was bound with.
    pub fn options(&self) -> &ServerOptions {
        &self.inner.options
    }

    /// Serve `holder` under `prefix`: the prefix itself is the holder and a
    /// path below it is `holder.child_by_path(rest)`. A longer prefix wins
    /// over a shorter one; `/` is the root; mounting a prefix again replaces
    /// what was there.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a prefix holding a query, a fragment or
    /// a control byte.
    pub fn mount(&self, prefix: &str, holder: Holder) -> Result<()> {
        let prefix = normalize_path(prefix)?;
        let mut state = self.inner.state();
        state.mounts.retain(|mount| mount.prefix != prefix);
        state.mounts.push(Mount {
            prefix,
            shared: Arc::new(RwLock::new(Mounted {
                holder,
                declared: BTreeMap::new(),
                version: 0,
            })),
        });
        state
            .mounts
            .sort_by_key(|mount| std::cmp::Reverse(mount.prefix.len()));
        Ok(())
    }

    /// Stop serving `prefix`; whether something was mounted there.
    pub fn unmount(&self, prefix: &str) -> bool {
        let Ok(prefix) = normalize_path(prefix) else {
            return false;
        };
        let mut state = self.inner.state();
        let before = state.mounts.len();
        state.mounts.retain(|mount| mount.prefix != prefix);
        state.mounts.len() < before
    }

    /// Serve the media type `media_type` for `path` under its mount, as a
    /// `PUT` carrying that `Content-Type` leaves it: the leaf is served
    /// under it whatever its name says.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] when no mount covers `path`.
    pub fn set_media_type(&self, path: &str, media_type: MediaType) -> Result<()> {
        let path = normalize_path(path)?;
        let state = self.inner.state();
        let (mount, rest) = state
            .mount(&path)
            .ok_or_else(|| Error::absent("mount", &path))?;
        let mut mounted = mount.shared.write().unwrap_or_else(PoisonError::into_inner);
        mounted.declared.insert(rest.to_owned(), media_type);
        Ok(())
    }

    /// Answer `path` with `handler`, for `method` or for every method
    /// (`None`); the exact method's route wins over the any-method one, and
    /// a route wins over a mount. A path that will not normalize is ignored.
    pub fn route(
        &self,
        method: Option<Method>,
        path: &str,
        handler: impl Fn(&Request) -> Result<Response> + Send + Sync + 'static,
    ) {
        let Ok(path) = normalize_path(path) else {
            return;
        };
        self.inner
            .state()
            .routes
            .insert((method, path), Route::Handler(Arc::new(handler)));
    }

    /// Answer `path` with `response` every time: its status, headers and
    /// body, the body read - or written - whole once here. A body that
    /// cannot be read, or a writer that fails, is answered as that failure,
    /// `500` with its text, rather than an empty body under the stated
    /// status.
    pub fn respond(&self, method: Option<Method>, path: &str, response: Response) {
        let Ok(path) = normalize_path(path) else {
            return;
        };
        let fixed = match response.raw_bytes() {
            Ok(bytes) => response.with_body(Body::Bytes(bytes)),
            Err(error) => {
                Response::new(Status::INTERNAL_SERVER_ERROR).with_text(&error.to_string())
            }
        };
        let fixed = Route::Fixed(Arc::new(fixed));
        self.inner.state().routes.insert((method, path), fixed);
    }

    /// Forget the route of `method` at `path`; whether there was one.
    pub fn unroute(&self, method: Option<Method>, path: &str) -> bool {
        let Ok(path) = normalize_path(path) else {
            return false;
        };
        self.inner.state().routes.remove(&(method, path)).is_some()
    }

    /// Apply `fault` to the next `times` requests of `path` (`0`: every
    /// request), before any route or mount answers.
    pub fn inject(&self, path: &str, fault: Fault, times: u32) {
        let Ok(path) = normalize_path(path) else {
            return;
        };
        self.inner.state().faults.push(Injected {
            path,
            fault,
            remaining: times,
        });
    }

    /// Forget every injected fault.
    pub fn clear_faults(&self) {
        self.inner.state().faults.clear();
    }

    /// Every request handled while recording since the last
    /// [`clear_requests`](Self::clear_requests), in order - the newest
    /// [`MAX_RECORDED`](Self::MAX_RECORDED) of them.
    pub fn requests(&self) -> Vec<Recorded> {
        self.inner.state().recorded.iter().cloned().collect()
    }

    /// Requests handled since the last [`clear_requests`](Self::clear_requests),
    /// recording or not.
    pub fn request_count(&self) -> usize {
        self.inner.count.load(Ordering::Relaxed)
    }

    /// Forget the recorded requests and zero the count.
    pub fn clear_requests(&self) {
        let mut state = self.inner.state();
        state.recorded.clear();
        state.recorded_bytes = 0;
        self.inner.count.store(0, Ordering::Relaxed);
    }

    /// Keep handled requests in the log (`true`) or only count them.
    pub fn set_recording(&self, on: bool) {
        self.inner.state().recording = on;
    }

    /// Connections accepted so far.
    pub fn connections(&self) -> u64 {
        self.inner.connections.load(Ordering::Relaxed)
    }

    /// Stop accepting and join the accept thread; connections being served
    /// finish their request. `Drop` does the same.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the accept thread panicked.
    pub fn shutdown(mut self) -> Result<()> {
        self.stop()
            .map_err(|_| Error::Io(std::io::Error::other("the accept thread panicked")))
    }

    /// Raise the stop flag, wake the accept loop and join it.
    fn stop(&mut self) -> std::thread::Result<()> {
        self.inner.stopping.store(true, Ordering::SeqCst);
        #[cfg(feature = "http3")]
        if let Some(h3) = &self.inner.h3 {
            h3.close();
        }
        // The accept loop reads the flag after a connection arrives, so one
        // is made - to the loopback address of the bound family when the
        // bind was to every address, which not every platform dials; the
        // timeout bounds the wait when the listener is gone.
        drop(TcpStream::connect_timeout(
            &loopback_of(self.inner.address),
            Duration::from_millis(100),
        ));
        match self.accept.take() {
            Some(accept) => accept.join(),
            None => Ok(()),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        drop(self.stop());
    }
}

/// `address` with an unspecified IP (`0.0.0.0`, `[::]`) replaced by the
/// loopback of its family: the one a process on this host dials.
fn loopback_of(mut address: SocketAddr) -> SocketAddr {
    if address.ip().is_unspecified() {
        address.set_ip(match address {
            SocketAddr::V4(_) => std::net::Ipv4Addr::LOCALHOST.into(),
            SocketAddr::V6(_) => std::net::Ipv6Addr::LOCALHOST.into(),
        });
    }
    address
}

impl fmt::Debug for Server {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.state();
        formatter
            .debug_struct("Server")
            .field("address", &self.inner.address)
            .field(
                "mounts",
                &state
                    .mounts
                    .iter()
                    .map(|mount| &mount.prefix)
                    .collect::<Vec<_>>(),
            )
            .field("routes", &state.routes.len())
            .field("faults", &state.faults.len())
            .field("requests", &self.inner.count.load(Ordering::Relaxed))
            .finish()
    }
}

/// The request path, each segment percent-decoded on its own.
///
/// A literal `..` was already removed by the reference resolution, which
/// never climbs above the root; a segment spelling one in escapes
/// (`%2e%2e`), or decoding to a separator (`%2F`, `%5C`) or a NUL, would
/// reach a mount's `child_by_path` as a step the resolution never saw, so
/// the target is refused instead of served.
fn decoded_path(url: &Url) -> crate::Result<String> {
    let raw = url.path_text(false)?;
    let mut path = String::with_capacity(raw.len());
    for (index, segment) in raw.split('/').enumerate() {
        let decoded = crate::uri::percent_decode(segment, "http path")?;
        if decoded == "." || decoded == ".." || decoded.contains(['/', '\\', '\0']) {
            return Err(crate::Error::Parse {
                target: "http path",
                position: 0,
                reason: smol_str::format_smolstr!(
                    "expected a path whose segments decode to names, got the segment {segment:?}"
                ),
            });
        }
        if index > 0 {
            path.push('/');
        }
        path.push_str(&decoded);
    }
    Ok(path)
}
