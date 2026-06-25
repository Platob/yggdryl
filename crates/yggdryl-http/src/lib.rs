//! # yggdryl-http
//!
//! A small **blocking HTTP client** for the yggdryl project, shaped after
//! Python's `requests`: a connection-pooling [`HttpSession`] with verb helpers
//! ([`get`](HttpSession::get) / [`post`](HttpSession::post) / …), a builder
//! [`HttpRequest`], and an [`HttpResponse`] whose body **streams over the
//! [`yggdryl-io`](yggdryl_io) abstraction** rather than being eagerly buffered.
//!
//! A response body is a [`ReadBytes`](yggdryl_io::ReadBytes) source
//! ([`HttpResponse::reader`]), so it composes with `copy`, the `Frames` codec, or
//! a `yggdryl-compression` decoder; [`bytes`](HttpResponse::bytes) /
//! [`text`](HttpResponse::text) / [`into_bytesio`](HttpResponse::into_bytesio)
//! drain it. A **request** body can likewise stream straight from any `Io` handle
//! via [`with_body_io`](HttpRequest::with_body_io) — uploading a
//! [`LocalPath`](yggdryl_io::LocalPath) never loads the file into memory.
//!
//! For random access there is [`HttpStream`], a seekable [`Io`](yggdryl_io::Io)
//! that **streams off a held connection** — sequential reads pull straight off the
//! socket, keeping only a sliding 4 MiB cache for short seek-backs, while a
//! pread / seek-back / forward jump reopens a `Range` request on a pooled
//! connection. It retries transient failures and **resumes from the cursor** after
//! a dropped connection, releasing the connection on EOF (or [`close`](Io::close)).
//! [`HttpSession::send_many`] runs an iterator of requests concurrently in batches.
//!
//! ```no_run
//! use yggdryl_http::{HttpSession, HttpRequest};
//!
//! let session = HttpSession::new().with_user_agent("yggdryl-http/0.1");
//! // Verbs raise on a 4xx/5xx by default; pass `false` to keep the response.
//! let body = session.get("https://example.com").unwrap().text().unwrap();
//!
//! // A seekable, lazily-fetched remote Io.
//! use yggdryl_io::{Io, Whence};
//! let mut stream = session.stream(HttpRequest::get("https://example.com/data").unwrap(), true).unwrap();
//! let mut footer = [0u8; 8];
//! stream.pread(&mut footer, -8, Whence::End).unwrap(); // read the tail, one range request
//! ```
//!
//! ## Optional features (off by default)
//!
//! - `compression` — transparently decode a `Content-Encoding` (gzip / zstd /
//!   snappy) response body through `yggdryl-compression`, the way `requests`
//!   auto-decompresses.
//! - `media` — expose the response's [`mime_type`](HttpResponse::mime_type) and
//!   [`HttpStream`]'s media type.
//! - `log` — structured `log` events on the request path.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use yggdryl_io::{BytesIO, Io, IoError, ReadBytes, Seek, Whence};
use yggdryl_url::{FromInput, Url};

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate pulls no `log` dependency by default).
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

/// The error type for every [`HttpSession`] / [`HttpRequest`] / [`HttpResponse`]
/// operation.
#[derive(Debug)]
pub enum HttpError {
    /// The URL could not be parsed.
    InvalidUrl(String),
    /// A header name or value was malformed.
    InvalidHeader(String),
    /// The request could not be sent or the response could not be received
    /// (connection, TLS, timeout, …).
    Transport(String),
    /// [`raise_for_status`](HttpResponse::raise_for_status) saw a 4xx/5xx code.
    Status(u16),
    /// The body could not be decoded (e.g. invalid UTF-8 for [`text`](HttpResponse::text)).
    Decode(String),
    /// An underlying byte-IO error while streaming the body.
    Io(IoError),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::InvalidUrl(what) => write!(f, "invalid url: {what}"),
            HttpError::InvalidHeader(what) => write!(f, "invalid header: {what}"),
            HttpError::Transport(what) => write!(f, "transport error: {what}"),
            HttpError::Status(code) => write!(f, "http status {code}"),
            HttpError::Decode(what) => write!(f, "decode error: {what}"),
            HttpError::Io(err) => write!(f, "io error: {err}"),
        }
    }
}

impl std::error::Error for HttpError {}

impl From<IoError> for HttpError {
    fn from(err: IoError) -> HttpError {
        HttpError::Io(err)
    }
}

impl From<ureq::Error> for HttpError {
    fn from(err: ureq::Error) -> HttpError {
        HttpError::Transport(err.to_string())
    }
}

impl From<ureq::http::Error> for HttpError {
    fn from(err: ureq::http::Error) -> HttpError {
        HttpError::InvalidHeader(err.to_string())
    }
}

/// An HTTP request method.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Method {
    /// `GET` — the default.
    #[default]
    Get,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `HEAD`
    Head,
    /// `OPTIONS`
    Options,
}

impl Method {
    /// Parses a method name (case-insensitive); an unknown method is an
    /// [`HttpError::InvalidHeader`].
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Method, HttpError> {
        let method = match value.trim().to_ascii_uppercase().as_str() {
            "GET" => Method::Get,
            "POST" => Method::Post,
            "PUT" => Method::Put,
            "PATCH" => Method::Patch,
            "DELETE" => Method::Delete,
            "HEAD" => Method::Head,
            "OPTIONS" => Method::Options,
            other => {
                return Err(HttpError::InvalidHeader(format!(
                    "unknown method {other:?}"
                )))
            }
        };
        Ok(method)
    }

    /// The canonical upper-case name (`"GET"`, `"POST"`, …).
    pub fn as_str(&self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Options => "OPTIONS",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The body carried by an [`HttpRequest`].
enum Body {
    /// No body.
    Empty,
    /// An in-memory byte body (replayable, so it can be retried).
    Bytes(Vec<u8>),
    /// A streamed body pulled from any byte source, sent without buffering.
    Reader(Box<dyn ReadBytes + Send>),
    /// A streamed body from an [`Io`] handle: its [`stream_len`](Seek::stream_len)
    /// sets `Content-Length` (so the upload is framed, not chunked) and the bytes
    /// flow straight off the handle — never collected into memory.
    Io(Box<dyn Io>),
}

impl Body {
    /// Whether the body can be re-sent on a retry (no consumed reader).
    fn replayable(&self) -> bool {
        matches!(self, Body::Empty | Body::Bytes(_))
    }
}

/// How [`HttpSession`] retries transient failures: rate-limit / unavailable
/// statuses (429 / 502 / 503 / 504, honouring `Retry-After`) and lost
/// connections, with capped exponential backoff. A retried request resumes a
/// streamed [`HttpStream`] from its current cursor via a `Range` re-request.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries after the first attempt (default `3`).
    pub max_retries: u32,
    /// The base backoff delay, doubled each attempt (default `200ms`).
    pub base_delay: Duration,
    /// The cap on any single backoff delay (default `10s`).
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> RetryConfig {
        RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(10),
        }
    }
}

impl RetryConfig {
    /// Whether a response status is a transient one worth retrying.
    fn retryable_status(&self, status: u16) -> bool {
        matches!(status, 429 | 502 | 503 | 504)
    }

    /// The delay before the next attempt: a `Retry-After` value if the server
    /// gave one, else capped exponential backoff.
    fn backoff(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(retry_after) = retry_after {
            return retry_after.min(self.max_delay);
        }
        let factor = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
        self.base_delay.saturating_mul(factor).min(self.max_delay)
    }
}

/// A builder for one HTTP request: a [`Method`], a [`Url`], headers, and an
/// optional body. Send it with [`HttpSession::request`].
///
/// The `with_*` methods are non-mutating in spirit (they consume and return
/// `self`), mirroring the rest of the project's builders.
pub struct HttpRequest {
    method: Method,
    url: Url,
    headers: Vec<(String, String)>,
    body: Body,
}

impl HttpRequest {
    /// Builds a request for `method` and `url`, returning [`HttpError::InvalidUrl`]
    /// if the URL is malformed.
    pub fn new(method: Method, url: &str) -> Result<HttpRequest, HttpError> {
        let url = Url::from_str(url).map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
        Ok(HttpRequest {
            method,
            url,
            headers: Vec::new(),
            body: Body::Empty,
        })
    }

    /// Builds a request from an already-parsed [`Url`].
    pub fn from_url(method: Method, url: Url) -> HttpRequest {
        HttpRequest {
            method,
            url,
            headers: Vec::new(),
            body: Body::Empty,
        }
    }

    /// `GET url`.
    pub fn get(url: &str) -> Result<HttpRequest, HttpError> {
        HttpRequest::new(Method::Get, url)
    }

    /// `POST url`.
    pub fn post(url: &str) -> Result<HttpRequest, HttpError> {
        HttpRequest::new(Method::Post, url)
    }

    /// `PUT url`.
    pub fn put(url: &str) -> Result<HttpRequest, HttpError> {
        HttpRequest::new(Method::Put, url)
    }

    /// `PATCH url`.
    pub fn patch(url: &str) -> Result<HttpRequest, HttpError> {
        HttpRequest::new(Method::Patch, url)
    }

    /// `DELETE url`.
    pub fn delete(url: &str) -> Result<HttpRequest, HttpError> {
        HttpRequest::new(Method::Delete, url)
    }

    /// `HEAD url`.
    pub fn head(url: &str) -> Result<HttpRequest, HttpError> {
        HttpRequest::new(Method::Head, url)
    }

    /// Adds (or appends) a request header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> HttpRequest {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Adds every `(name, value)` pair as a header.
    pub fn with_headers<I, K, V>(mut self, headers: I) -> HttpRequest
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.headers
            .extend(headers.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Adds a query parameter to the URL (percent-encoding `value`).
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> HttpRequest {
        self.url = self.url.add_param(key, vec![value.into()], true);
        self
    }

    /// Sets an in-memory byte body.
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> HttpRequest {
        self.body = Body::Bytes(body.into());
        self
    }

    /// Sets a **streamed** body pulled from any byte source — e.g. a
    /// [`LocalPath`](yggdryl_io::LocalPath) or [`BytesIO`](yggdryl_io::BytesIO) —
    /// so a large upload is never buffered into memory.
    pub fn with_body_reader<R: ReadBytes + Send + 'static>(mut self, reader: R) -> HttpRequest {
        self.body = Body::Reader(Box::new(reader));
        self
    }

    /// Sets a **streamed** body from an [`Io`] handle, the preferred upload path:
    /// its known length frames the request with `Content-Length` and the bytes are
    /// read straight off the handle (a file is never loaded into memory).
    pub fn with_body_io<I: Io + 'static>(mut self, io: I) -> HttpRequest {
        self.body = Body::Io(Box::new(io));
        self
    }

    /// The request method.
    pub fn method(&self) -> Method {
        self.method
    }

    /// The request URL.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The request headers, in insertion order.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
}

/// A streaming HTTP response body: a [`ReadBytes`] source that pulls bytes off
/// the socket on demand (transparently decompressed under the `compression`
/// feature). Returned by [`HttpResponse::reader`].
pub struct HttpBody {
    inner: Box<dyn ReadBytes>,
}

impl ReadBytes for HttpBody {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        self.inner.read_bytes(buf)
    }

    /// Drains into `out` via the inner reader's own `read_to_end`, so an
    /// undecoded body flows once off the socket (the [`HttpStream`] streams
    /// straight into `out`) instead of bouncing through a stack buffer.
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        self.inner.read_to_end(out)
    }
}

/// A received HTTP response, modelled on `requests.Response`. Its body is a live
/// [`HttpStream`] over the held connection, read lazily — drained by
/// [`bytes`](HttpResponse::bytes) / [`text`](HttpResponse::text), or taken whole
/// with [`into_stream`](HttpResponse::into_stream) for seekable access.
pub struct HttpResponse {
    status: u16,
    url: Url,
    headers: Vec<(String, String)>,
    stream: HttpStream,
}

impl HttpResponse {
    #[allow(clippy::too_many_arguments)]
    fn build(
        response: ureq::http::Response<ureq::Body>,
        agent: ureq::Agent,
        url: Url,
        request_headers: Vec<(String, String)>,
        retry: RetryConfig,
        keep_alive: bool,
        held: Arc<AtomicUsize>,
    ) -> HttpResponse {
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let stream = HttpStream::from_response(
            response,
            agent,
            url.clone(),
            request_headers,
            retry,
            keep_alive,
            held,
        );
        HttpResponse {
            status,
            url,
            headers,
            stream,
        }
    }

    /// Consumes the response, returning its body as a seekable [`HttpStream`] over
    /// the held connection.
    pub fn into_stream(self) -> HttpStream {
        self.stream
    }

    /// The HTTP status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Whether the status is below 400 (the `requests` definition of "ok").
    pub fn ok(&self) -> bool {
        self.status < 400
    }

    /// Returns an error ([`HttpError::Status`]) if the status is 4xx or 5xx,
    /// otherwise `self` — the `requests` `raise_for_status` pattern.
    pub fn raise_for_status(self) -> Result<HttpResponse, HttpError> {
        if self.status >= 400 {
            log_event!(warn, "HttpResponse::raise_for_status: {}", self.status);
            return Err(HttpError::Status(self.status));
        }
        Ok(self)
    }

    /// The final request URL (after any redirects the transport followed).
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// The response headers, with lower-cased names, in received order.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Looks up a header by name (case-insensitive), returning its value.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&name))
            .map(|(_, value)| value.as_str())
    }

    /// The `Content-Type` header, if present.
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// The `Content-Length` header parsed as a number, if present and valid.
    pub fn content_length(&self) -> Option<u64> {
        self.header("content-length").and_then(|v| v.parse().ok())
    }

    /// The `Content-Encoding` header, if present.
    pub fn content_encoding(&self) -> Option<&str> {
        self.header("content-encoding")
    }

    /// The response media type, inferred from its `Content-Type`. Only present
    /// under the `media` feature.
    #[cfg(feature = "media")]
    pub fn mime_type(&self) -> Option<yggdryl_media::MimeType> {
        self.content_type()
            .and_then(|content_type| yggdryl_media::MimeType::from_str(content_type).ok())
    }

    /// Consumes the response and returns its body as a streaming [`ReadBytes`]
    /// source. Under the `compression` feature a `Content-Encoding` of gzip / zstd
    /// / snappy is decoded transparently.
    pub fn reader(self) -> HttpBody {
        let HttpResponse {
            headers, stream, ..
        } = self;

        #[cfg(feature = "compression")]
        {
            // Resolve the codec before touching `stream`, so the fall-through to
            // the undecoded body never trips over a moved value.
            let codec = header_value(&headers, "content-encoding")
                .and_then(|encoding| yggdryl_compression::Compression::from_str(encoding).ok())
                .filter(|codec| {
                    *codec != yggdryl_compression::Compression::None && codec.is_available()
                });
            if let Some(codec) = codec {
                log_event!(debug, "HttpResponse::reader decoding {codec}");
                return match codec.decoder(stream) {
                    Ok(decoder) => HttpBody {
                        inner: Box::new(decoder),
                    },
                    Err(err) => HttpBody {
                        inner: Box::new(ErrBody(Some(err))),
                    },
                };
            }
        }
        #[cfg(not(feature = "compression"))]
        let _ = &headers;
        HttpBody {
            inner: Box::new(stream),
        }
    }

    /// Drains the body into a `Vec<u8>` (decompressing under the `compression`
    /// feature).
    pub fn bytes(self) -> Result<Vec<u8>, HttpError> {
        // Pre-size from Content-Length when present (a hint; a compressed body
        // expands past it, an empty one wastes nothing).
        let hint = self.content_length().unwrap_or(0).min(64 * 1024 * 1024) as usize;
        let mut out = Vec::with_capacity(hint);
        self.reader().read_to_end(&mut out)?;
        Ok(out)
    }

    /// Drains the body and decodes it as UTF-8 text.
    pub fn text(self) -> Result<String, HttpError> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes).map_err(|err| HttpError::Decode(err.to_string()))
    }

    /// Drains the body into an in-memory [`BytesIO`] handle — a seekable
    /// [`Io`](yggdryl_io::Io) over the (decompressed) response.
    pub fn into_bytesio(self) -> Result<BytesIO, HttpError> {
        Ok(BytesIO::from_bytes(self.bytes()?))
    }
}

/// Finds a header value by name (case-insensitive).
#[cfg(feature = "compression")]
fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// A connection-pooling HTTP client, like `requests.Session`: it reuses
/// connections across requests and carries default headers applied to each.
pub struct HttpSession {
    agent: ureq::Agent,
    headers: Vec<(String, String)>,
    retry: RetryConfig,
    max_concurrency: usize,
    batch_size: usize,
    /// The idle-connection pool size — reused (keep-alive) connections skip the
    /// TLS handshake on the next request to the same host.
    max_pool: usize,
    /// The live count of open [`HttpStream`]s (held connections), so extra streams
    /// past the pool size can drop keep-alive and not starve the pool.
    held: Arc<AtomicUsize>,
}

/// The default idle-connection pool size.
const DEFAULT_POOL: usize = 16;

impl HttpSession {
    /// Creates a session with a pooled connection (default 16 idle connections,
    /// reused without re-doing the TLS handshake), default retry policy, a
    /// concurrency of 8 and a batch size of 80 (`max_concurrency * 10`).
    pub fn new() -> HttpSession {
        HttpSession::with_config(RetryConfig::default(), DEFAULT_POOL)
    }

    fn with_config(retry: RetryConfig, max_pool: usize) -> HttpSession {
        let max_pool = max_pool.max(1);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_idle_connections(max_pool)
            .max_idle_connections_per_host(max_pool)
            .build()
            .into();
        let max_concurrency = 8;
        HttpSession {
            agent,
            headers: Vec::new(),
            retry,
            max_concurrency,
            batch_size: max_concurrency * 10,
            max_pool,
            held: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Adds a default header sent with every request from this session.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> HttpSession {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sets the default `User-Agent` header.
    pub fn with_user_agent(self, user_agent: impl Into<String>) -> HttpSession {
        self.with_header("user-agent", user_agent)
    }

    /// Sets the [`RetryConfig`] for transient failures.
    pub fn with_retry(mut self, retry: RetryConfig) -> HttpSession {
        self.retry = retry;
        self
    }

    /// Sets the maximum number of concurrent requests in [`send_many`](HttpSession::send_many)
    /// (and resets the batch size to `max_concurrency * 10`).
    pub fn with_max_concurrency(mut self, max_concurrency: usize) -> HttpSession {
        self.max_concurrency = max_concurrency.max(1);
        self.batch_size = self.max_concurrency * 10;
        self
    }

    /// Sets the [`send_many`](HttpSession::send_many) batch size.
    pub fn with_batch_size(mut self, batch_size: usize) -> HttpSession {
        self.batch_size = batch_size.max(1);
        self
    }

    /// Sets the idle-connection pool size (rebuilding the pooled agent). Larger
    /// pools keep more keep-alive connections warm (skipping TLS handshakes).
    pub fn with_pool_size(mut self, max_pool: usize) -> HttpSession {
        let max_pool = max_pool.max(1);
        self.agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_idle_connections(max_pool)
            .max_idle_connections_per_host(max_pool)
            .build()
            .into();
        self.max_pool = max_pool;
        self
    }

    /// The idle-connection pool size.
    pub fn pool_size(&self) -> usize {
        self.max_pool
    }

    /// The number of [`HttpStream`]s currently holding a connection open.
    pub fn open_streams(&self) -> usize {
        self.held.load(Ordering::SeqCst)
    }

    /// The session's default headers.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The maximum number of concurrent requests in [`send_many`](HttpSession::send_many).
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// The [`send_many`](HttpSession::send_many) batch size.
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// `GET url` (raises on a 4xx/5xx status).
    pub fn get(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::get(url)?, true)
    }

    /// `HEAD url` (raises on a 4xx/5xx status).
    pub fn head(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::head(url)?, true)
    }

    /// `DELETE url` (raises on a 4xx/5xx status).
    pub fn delete(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::delete(url)?, true)
    }

    /// `POST url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn post(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::post(url)?.with_body(body), true)
    }

    /// `PUT url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn put(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::put(url)?.with_body(body), true)
    }

    /// `PATCH url` with an in-memory byte body (raises on a 4xx/5xx status).
    pub fn patch(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::patch(url)?.with_body(body), true)
    }

    /// Merges the session's default headers into `request` (a per-request header
    /// overrides a session default) and returns the final request — the single
    /// place every request is assembled before sending.
    pub fn prepare(&self, request: HttpRequest) -> HttpRequest {
        let mut headers = Vec::with_capacity(self.headers.len() + request.headers.len());
        for (name, value) in &self.headers {
            let overridden = request
                .headers
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case(name));
            if !overridden {
                headers.push((name.clone(), value.clone()));
            }
        }
        headers.extend(request.headers);
        HttpRequest {
            method: request.method,
            url: request.url,
            headers,
            body: request.body,
        }
    }

    /// **The one place every request is sent.** [`prepare`](HttpSession::prepare)s
    /// the request, runs it with the retry policy, and returns an [`HttpResponse`]
    /// whose body is a live [`HttpStream`] over the held connection.
    ///
    /// `raise_error` (`true` for the verb helpers) turns a 4xx/5xx status into an
    /// [`HttpError::Status`]. `keep_alive` keeps the connection pooled for reuse;
    /// `false` sends `Connection: close`. As a pool safeguard, once more than
    /// [`pool_size`](HttpSession::pool_size) streams are already open, a new one
    /// drops keep-alive regardless, so streaming reads never starve the pool.
    pub fn send(
        &self,
        request: HttpRequest,
        raise_error: bool,
        keep_alive: bool,
    ) -> Result<HttpResponse, HttpError> {
        let mut request = self.prepare(request);
        let keep_alive = keep_alive && self.held.load(Ordering::SeqCst) < self.max_pool;
        if !keep_alive {
            request
                .headers
                .push(("connection".to_string(), "close".to_string()));
        }
        let url = request.url.clone();
        log_event!(
            debug,
            "HttpSession::send {} {url} keep_alive={keep_alive}",
            request.method.as_str()
        );
        let raw = self.execute(
            request.method,
            url.to_string().as_str(),
            &request.headers,
            request.body,
        )?;
        let response = HttpResponse::build(
            raw,
            self.agent.clone(),
            url,
            request.headers,
            self.retry.clone(),
            keep_alive,
            self.held.clone(),
        );
        if raise_error && response.status >= 400 {
            // Drop closes the held connection; the error carries the status.
            return Err(HttpError::Status(response.status));
        }
        Ok(response)
    }

    /// Sends a request, raising on a 4xx/5xx when `raise_error` (a keep-alive
    /// [`send`](HttpSession::send)).
    pub fn request(
        &self,
        request: HttpRequest,
        raise_error: bool,
    ) -> Result<HttpResponse, HttpError> {
        self.send(request, raise_error, true)
    }

    /// Opens a seekable [`HttpStream`] over a resource by sending `request` and
    /// taking the held connection as the stream — bytes are then streamed off it
    /// on demand. `keep_alive` pools the connection for reuse.
    pub fn stream(&self, request: HttpRequest, keep_alive: bool) -> Result<HttpStream, HttpError> {
        Ok(self.send(request, false, keep_alive)?.into_stream())
    }

    /// Sends an iterator of requests concurrently, **streamed** in batches of
    /// [`batch_size`](HttpSession::batch_size) (each running up to
    /// [`max_concurrency`](HttpSession::max_concurrency) at a time) and yielding
    /// one [`HttpResponseBatch`] per batch. Lazy: only one batch is in flight, so
    /// an unbounded request stream uses bounded memory. Responses are returned
    /// whatever their status (transport/parse failures are `Err` entries).
    pub fn send_many<I>(&self, requests: I) -> SendMany<'_, I::IntoIter>
    where
        I: IntoIterator<Item = HttpRequest>,
    {
        SendMany {
            session: self,
            requests: requests.into_iter(),
        }
    }

    /// Runs one batch with bounded concurrency (waves of `max_concurrency`),
    /// preserving request order.
    fn run_batch(&self, batch: Vec<HttpRequest>) -> HttpResponseBatch {
        let concurrency = self.max_concurrency.max(1);
        let mut results = Vec::with_capacity(batch.len());
        let mut requests = batch.into_iter();
        loop {
            let wave: Vec<HttpRequest> = requests.by_ref().take(concurrency).collect();
            if wave.is_empty() {
                break;
            }
            let wave_results: Vec<Result<HttpResponse, HttpError>> = std::thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .into_iter()
                    .map(|request| scope.spawn(move || self.request(request, false)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|_| Err(HttpError::Transport("worker panicked".into())))
                    })
                    .collect()
            });
            results.extend(wave_results);
        }
        HttpResponseBatch { results }
    }

    /// The retry loop shared by every send: replayable bodies (none / bytes) are
    /// retried on transient statuses and lost connections; a streamed body is
    /// single-shot.
    fn execute(
        &self,
        method: Method,
        url: &str,
        headers: &[(String, String)],
        mut body: Body,
    ) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
        let replayable = body.replayable();
        let mut attempt = 0u32;
        loop {
            let builder = self.builder(method, url, headers);
            let outcome = match &body {
                Body::Empty => self.agent.run(builder.body(ureq::SendBody::none())?),
                Body::Bytes(bytes) => self.agent.run(builder.body(bytes.clone())?),
                Body::Reader(_) | Body::Io(_) => {
                    return self.run_streamed(builder, std::mem::replace(&mut body, Body::Empty));
                }
            };
            match outcome {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if attempt < self.retry.max_retries && self.retry.retryable_status(status) {
                        let delay = self.retry.backoff(attempt, retry_after(response.headers()));
                        log_event!(warn, "retrying status {status} after {delay:?}");
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    return Ok(response);
                }
                Err(error) => {
                    if attempt < self.retry.max_retries && replayable {
                        let delay = self.retry.backoff(attempt, None);
                        log_event!(warn, "reconnecting after transport error: {error}");
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    return Err(error.into());
                }
            }
        }
    }

    /// Builds a request builder with `method`, `url` and all (already merged)
    /// `headers` applied.
    fn builder(
        &self,
        method: Method,
        url: &str,
        headers: &[(String, String)],
    ) -> ureq::http::request::Builder {
        let mut builder = ureq::http::Request::builder()
            .method(method.as_str())
            .uri(url);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        builder
    }

    /// Sends a single-shot streamed body (reader or `Io`). An `Io` body sets
    /// `Content-Length` from its known length so the upload is framed, not chunked.
    fn run_streamed(
        &self,
        builder: ureq::http::request::Builder,
        body: Body,
    ) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
        match body {
            Body::Reader(reader) => {
                let mut bridge = ReadBridge(reader);
                Ok(self
                    .agent
                    .run(builder.body(ureq::SendBody::from_reader(&mut bridge))?)?)
            }
            Body::Io(io) => {
                let length = io.stream_len();
                let mut bridge = IoBridge(io);
                let builder = match length {
                    Some(length) => builder.header("content-length", length.to_string()),
                    None => builder,
                };
                Ok(self
                    .agent
                    .run(builder.body(ureq::SendBody::from_reader(&mut bridge))?)?)
            }
            Body::Empty | Body::Bytes(_) => {
                unreachable!("run_streamed called with a replayable body")
            }
        }
    }
}

impl Default for HttpSession {
    fn default() -> HttpSession {
        HttpSession::new()
    }
}

/// The lazy iterator returned by [`HttpSession::send_many`]: each `next` pulls up
/// to [`batch_size`](HttpSession::batch_size) requests and runs them concurrently.
pub struct SendMany<'a, I: Iterator<Item = HttpRequest>> {
    session: &'a HttpSession,
    requests: I,
}

impl<I: Iterator<Item = HttpRequest>> Iterator for SendMany<'_, I> {
    type Item = HttpResponseBatch;

    fn next(&mut self) -> Option<HttpResponseBatch> {
        let batch: Vec<HttpRequest> = self
            .requests
            .by_ref()
            .take(self.session.batch_size)
            .collect();
        if batch.is_empty() {
            return None;
        }
        Some(self.session.run_batch(batch))
    }
}

/// One batch of results from [`HttpSession::send_many`], in request order.
pub struct HttpResponseBatch {
    results: Vec<Result<HttpResponse, HttpError>>,
}

impl HttpResponseBatch {
    /// The number of responses in the batch.
    pub fn len(&self) -> usize {
        self.results.len()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// Consumes the batch, yielding each request's `Result` in order.
    pub fn into_results(self) -> Vec<Result<HttpResponse, HttpError>> {
        self.results
    }
}

impl IntoIterator for HttpResponseBatch {
    type Item = Result<HttpResponse, HttpError>;
    type IntoIter = std::vec::IntoIter<Result<HttpResponse, HttpError>>;

    fn into_iter(self) -> Self::IntoIter {
        self.results.into_iter()
    }
}

/// A seekable [`Io`] over an HTTP resource that **streams off a held
/// connection** rather than collecting the body: sequential [`read`](ReadBytes)
/// pulls bytes straight off the socket on demand, keeping only a sliding 4 MiB
/// cache for short seek-backs. Random access (`pread`, a footer read) or a
/// seek-back past the cache re-opens a `Range` request on a pooled connection.
/// The connection is released to the pool on EOF (or closed, with no keep-alive),
/// and [`close`](Io::close) drops it eagerly. A connection lost mid-stream is
/// reconnected and **resumed from the cursor**.
pub struct HttpStream {
    agent: ureq::Agent,
    url: Url,
    headers: Vec<(String, String)>,
    retry: RetryConfig,
    /// When `false`, requests carry `Connection: close` so the socket is not
    /// pooled (the pool-saturation safeguard sets this on extra streams).
    keep_alive: bool,
    size: Option<u64>,
    content_type: Option<String>,
    /// The live response-body reader, positioned at `reader_pos`. `None` once the
    /// stream is closed, exhausted, or awaiting a (re)open for `position`.
    reader: Option<Box<dyn std::io::Read + Send + Sync>>,
    reader_pos: u64,
    /// A sliding cache of recently-streamed bytes for short seek-backs, never
    /// larger than `CACHE_LIMIT`.
    cache: Vec<u8>,
    cache_start: u64,
    position: u64,
    closed: bool,
    /// Shared count of live streams (held connections) for the pool safeguard.
    held: Arc<AtomicUsize>,
}

/// The most recently-streamed bytes [`HttpStream`] keeps for a seek-back (4 MiB).
const CACHE_LIMIT: usize = 4 * 1024 * 1024;

impl HttpStream {
    /// Builds a stream from a freshly-received response, holding its live body as
    /// the connection at offset 0.
    fn from_response(
        response: ureq::http::Response<ureq::Body>,
        agent: ureq::Agent,
        url: Url,
        headers: Vec<(String, String)>,
        retry: RetryConfig,
        keep_alive: bool,
        held: Arc<AtomicUsize>,
    ) -> HttpStream {
        let size = response_size(response.headers());
        let content_type = header_string(response.headers(), "content-type");
        let reader: Box<dyn std::io::Read + Send + Sync> =
            Box::new(response.into_body().into_reader());
        held.fetch_add(1, Ordering::SeqCst);
        HttpStream {
            agent,
            url,
            headers,
            retry,
            keep_alive,
            size,
            content_type,
            reader: Some(reader),
            reader_pos: 0,
            cache: Vec::new(),
            cache_start: 0,
            position: 0,
            closed: false,
            held,
        }
    }

    /// The total size in bytes, if the server reported it.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    /// The `Content-Type`, if the server reported it.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Closes the held connection eagerly (idempotent); further reads return EOF.
    pub fn close(&mut self) {
        self.reader = None;
        self.closed = true;
    }

    /// The number of bytes still readable from `position`, if the size is known.
    fn remaining(&self) -> Option<u64> {
        self.size.map(|size| size.saturating_sub(self.position))
    }

    /// Records end-of-input on an unknown-size stream and releases the socket.
    fn on_eof(&mut self) {
        if self.size.is_none() {
            self.size = Some(self.position);
        }
        self.reader = None; // exhausted: return the connection to the pool
    }

    /// (Re)opens a `Range` request from `start`, replacing the live reader — used
    /// to seek back past the cache, jump forward, or resume after a drop.
    fn open_at(&mut self, start: u64) -> Result<(), IoError> {
        let (size, reader) = self.request_reader(start)?;
        if self.size.is_none() {
            self.size = size;
        }
        self.reader = reader;
        self.reader_pos = start;
        self.cache.clear();
        self.cache_start = start;
        Ok(())
    }

    /// Issues `GET <url>` with `Range: bytes=start-` (and `Connection: close` when
    /// not keep-alive), retrying transient statuses and reconnecting on error.
    /// Returns the total size (if newly learnt) and the live reader, or `None`
    /// reader at a clean EOF (`416`).
    #[allow(clippy::type_complexity)]
    fn request_reader(
        &self,
        start: u64,
    ) -> Result<(Option<u64>, Option<Box<dyn std::io::Read + Send + Sync>>), IoError> {
        let url = self.url.to_string();
        let range = format!("bytes={start}-");
        let mut attempt = 0u32;
        loop {
            let mut builder = ureq::http::Request::builder()
                .method("GET")
                .uri(url.as_str());
            for (name, value) in &self.headers {
                builder = builder.header(name, value);
            }
            builder = builder.header("range", range.as_str());
            if !self.keep_alive {
                builder = builder.header("connection", "close");
            }
            let outcome = builder
                .body(ureq::SendBody::none())
                .map_err(|err| IoError::Io(err.to_string()))
                .and_then(|request| {
                    self.agent
                        .run(request)
                        .map_err(|err| IoError::Io(err.to_string()))
                });
            match outcome {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if attempt < self.retry.max_retries && self.retry.retryable_status(status) {
                        let delay = self.retry.backoff(attempt, retry_after(response.headers()));
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    if status == 416 {
                        return Ok((None, None)); // range past the end: clean EOF
                    }
                    if status >= 400 {
                        return Err(IoError::Io(format!(
                            "http status {status} fetching a range (check the URL and that the resource still exists)"
                        )));
                    }
                    if status == 200 && start > 0 {
                        return Err(IoError::Unsupported(
                            "server ignored the Range request (it does not support range reads)"
                                .to_string(),
                        ));
                    }
                    let size = response_size(response.headers());
                    let reader: Box<dyn std::io::Read + Send + Sync> =
                        Box::new(response.into_body().into_reader());
                    return Ok((size, Some(reader)));
                }
                Err(error) => {
                    if attempt < self.retry.max_retries {
                        let delay = self.retry.backoff(attempt, None);
                        log_event!(warn, "HttpStream reconnect after error: {error}");
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }

    /// Reads off the live connection into `buf`, reconnecting (resuming the range
    /// from `reader_pos`) if the connection drops, up to the retry limit.
    fn read_live(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        let mut attempt = 0u32;
        loop {
            let result = match self.reader.as_mut() {
                Some(reader) => std::io::Read::read(reader, buf),
                None => return Ok(0),
            };
            match result {
                Ok(count) => return Ok(count),
                Err(_error) if attempt < self.retry.max_retries => {
                    attempt += 1;
                    std::thread::sleep(self.retry.backoff(attempt - 1, None));
                    log_event!(
                        warn,
                        "HttpStream reconnect mid-stream at {} (attempt {attempt})",
                        self.reader_pos
                    );
                    self.open_at(self.reader_pos)?; // resume from the cursor
                    continue;
                }
                Err(error) => return Err(IoError::from(error)),
            }
        }
    }

    /// Fetches exactly `[start, start+len)` into a fresh `Vec` via a one-off
    /// `Range` request — used by [`pread`](Io::pread), leaving the live reader
    /// untouched.
    fn fetch_range(&self, start: u64, len: u64) -> Result<Vec<u8>, IoError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let end = start
            .checked_add(len)
            .and_then(|end| end.checked_sub(1))
            .ok_or_else(|| IoError::Invalid("range offset overflow".to_string()))?;
        let url = self.url.to_string();
        let range = format!("bytes={start}-{end}");
        let mut out = Vec::with_capacity(len.min(CACHE_LIMIT as u64) as usize);
        let mut attempt = 0u32;
        loop {
            out.clear();
            let mut builder = ureq::http::Request::builder()
                .method("GET")
                .uri(url.as_str());
            for (name, value) in &self.headers {
                builder = builder.header(name, value);
            }
            builder = builder.header("range", range.as_str());
            let outcome = builder
                .body(ureq::SendBody::none())
                .map_err(|err| IoError::Io(err.to_string()))
                .and_then(|request| {
                    self.agent
                        .run(request)
                        .map_err(|err| IoError::Io(err.to_string()))
                });
            match outcome {
                Ok(mut response) => {
                    let status = response.status().as_u16();
                    if attempt < self.retry.max_retries && self.retry.retryable_status(status) {
                        let delay = self.retry.backoff(attempt, retry_after(response.headers()));
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    if status == 416 {
                        return Ok(Vec::new());
                    }
                    if status >= 400 {
                        return Err(IoError::Io(format!("http status {status}")));
                    }
                    if status == 200 && start > 0 {
                        return Err(IoError::Unsupported(
                            "server ignored the Range request (it does not support range reads)"
                                .to_string(),
                        ));
                    }
                    let mut reader = response.body_mut().as_reader();
                    match std::io::Read::read_to_end(&mut reader, &mut out) {
                        Ok(_) => {
                            if status == 200 {
                                out.truncate(len as usize);
                            }
                            return Ok(out);
                        }
                        Err(_error) if attempt < self.retry.max_retries => {
                            let delay = self.retry.backoff(attempt, None);
                            attempt += 1;
                            std::thread::sleep(delay);
                            continue;
                        }
                        Err(error) => return Err(IoError::from(error)),
                    }
                }
                Err(error) => {
                    if attempt < self.retry.max_retries {
                        let delay = self.retry.backoff(attempt, None);
                        attempt += 1;
                        std::thread::sleep(delay);
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        self.held.fetch_sub(1, Ordering::SeqCst);
    }
}

impl fmt::Debug for HttpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpStream")
            .field("url", &self.url.to_string())
            .field("size", &self.size)
            .field("position", &self.position)
            .field("reader_pos", &self.reader_pos)
            .field("cache", &(self.cache_start, self.cache.len()))
            .field("closed", &self.closed)
            .finish()
    }
}

impl ReadBytes for HttpStream {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        if buf.is_empty() || self.closed || self.remaining() == Some(0) {
            return Ok(0);
        }
        // A short seek-back is served from the sliding cache.
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.position >= self.cache_start && self.position < cache_end {
            let offset = (self.position - self.cache_start) as usize;
            let available = &self.cache[offset..];
            let count = buf.len().min(available.len());
            buf[..count].copy_from_slice(&available[..count]);
            self.position += count as u64;
            return Ok(count);
        }
        // Otherwise stream off the live connection (re-opening if the cursor moved
        // off the reader).
        if self.reader.is_none() || self.reader_pos != self.position {
            self.open_at(self.position)?;
        }
        let count = self.read_live(buf)?;
        if count == 0 {
            self.on_eof();
            return Ok(0);
        }
        self.cache.extend_from_slice(&buf[..count]);
        if self.cache.len() > CACHE_LIMIT {
            let evict = self.cache.len() - CACHE_LIMIT;
            self.cache.drain(..evict);
            self.cache_start += evict as u64;
        }
        self.reader_pos += count as u64;
        self.position += count as u64;
        Ok(count)
    }

    /// Drains the rest of the stream straight into `out`, reading whole chunks off
    /// the connection into `out`'s own buffer (one copy), reconnecting on a drop.
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        if self.closed {
            return Ok(0);
        }
        let start_len = out.len();
        // Serve any cached tail first.
        let cache_end = self.cache_start + self.cache.len() as u64;
        if self.position >= self.cache_start && self.position < cache_end {
            let offset = (self.position - self.cache_start) as usize;
            out.extend_from_slice(&self.cache[offset..]);
            self.position += (self.cache.len() - offset) as u64;
        }
        if self.remaining() != Some(0)
            && (self.reader.is_none() || self.reader_pos != self.position)
        {
            self.open_at(self.position)?;
        }
        while self.remaining() != Some(0) {
            let base = out.len();
            out.resize(base + 64 * 1024, 0);
            let count = self.read_live(&mut out[base..])?;
            out.truncate(base + count);
            if count == 0 {
                self.on_eof();
                break;
            }
            self.reader_pos += count as u64;
            self.position += count as u64;
        }
        Ok(out.len() - start_len)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base = match whence {
            Whence::Start => 0i64,
            Whence::Current => self.position as i64,
            Whence::End => self.size.ok_or_else(|| {
                IoError::Unsupported(
                    "seek from end needs a known size (the server sent no Content-Length)"
                        .to_string(),
                )
            })? as i64,
        };
        let target = base
            .checked_add(offset)
            .ok_or_else(|| IoError::Invalid("seek offset overflow".to_string()))?;
        if target < 0 {
            return Err(IoError::Invalid("seek before start".to_string()));
        }
        self.position = target as u64;
        Ok(self.position)
    }

    fn stream_position(&self) -> u64 {
        self.position
    }

    fn stream_len(&self) -> Option<u64> {
        self.size
    }
}

impl Io for HttpStream {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn stats(&self) -> Result<yggdryl_io::IoStats, IoError> {
        let mut stats =
            yggdryl_io::IoStats::new(self.size.unwrap_or(0)).with_kind(yggdryl_io::Kind::File);
        if let Some(content_type) = &self.content_type {
            stats = stats.with_content_type(content_type.clone());
        }
        #[cfg(feature = "media")]
        if let Some(media_type) = self.media_type() {
            stats = stats.with_media_type(media_type);
        }
        Ok(stats)
    }

    /// Releases the held connection (idempotent); further reads return EOF.
    fn close(&mut self) -> Result<(), IoError> {
        HttpStream::close(self);
        Ok(())
    }

    /// A positional read via a one-off `Range` request, leaving the live reader
    /// and — for [`Whence::Start`]/[`Whence::End`] — the cursor untouched, so a
    /// footer can be read without disturbing a sequential scan.
    fn pread(&mut self, buf: &mut [u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        let base = match whence {
            Whence::Start => 0i64,
            Whence::Current => self.position as i64,
            Whence::End => self.size.ok_or_else(|| {
                IoError::Unsupported(
                    "pread from end needs a known size (the server sent no Content-Length)"
                        .to_string(),
                )
            })? as i64,
        };
        let start = base
            .checked_add(offset)
            .ok_or_else(|| IoError::Invalid("pread offset overflow".to_string()))?;
        if start < 0 {
            return Err(IoError::Invalid("pread before start".to_string()));
        }
        let start = start as u64;
        if self.size.is_some_and(|size| start >= size) {
            return Ok(0);
        }
        let want = self.size.map_or(buf.len() as u64, |size| {
            (buf.len() as u64).min(size - start)
        });
        let bytes = self.fetch_range(start, want)?;
        let count = buf.len().min(bytes.len());
        buf[..count].copy_from_slice(&bytes[..count]);
        if matches!(whence, Whence::Current) {
            self.position = start + count as u64;
        }
        Ok(count)
    }

    #[cfg(feature = "media")]
    fn media_type(&self) -> Option<yggdryl_media::MediaType> {
        let content_type = self.content_type.as_ref()?;
        let essence = content_type.split(';').next()?.trim();
        yggdryl_media::MimeType::from_str(essence)
            .ok()
            .map(|mime| yggdryl_media::MediaType::new(vec![mime]))
    }
}

/// Bridges an [`Io`] request body to [`std::io::Read`] for the transport.
struct IoBridge(Box<dyn Io>);

impl std::io::Read for IoBridge {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0
            .read_bytes(buf)
            .map_err(|err| std::io::Error::other(err.to_string()))
    }
}

/// Reads a header value as an owned `String` (case-insensitive).
fn header_string(headers: &ureq::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

/// Reads a header value parsed as a `u64`.
fn header_u64(headers: &ureq::http::HeaderMap, name: &str) -> Option<u64> {
    header_string(headers, name).and_then(|value| value.trim().parse().ok())
}

/// Parses a `Retry-After` header given in seconds.
fn retry_after(headers: &ureq::http::HeaderMap) -> Option<Duration> {
    header_string(headers, "retry-after")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// The total resource size from a response: the total in a `Content-Range`
/// (`bytes a-b/total`) when present, else `Content-Length`.
fn response_size(headers: &ureq::http::HeaderMap) -> Option<u64> {
    if let Some(range) = header_string(headers, "content-range") {
        if let Some((_, total)) = range.rsplit_once('/') {
            if let Ok(total) = total.trim().parse() {
                return Some(total);
            }
        }
    }
    header_u64(headers, "content-length")
}

/// Bridges a [`ReadBytes`] request body to [`std::io::Read`] for the transport.
struct ReadBridge(Box<dyn ReadBytes + Send>);

impl std::io::Read for ReadBridge {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0
            .read_bytes(buf)
            .map_err(|err| std::io::Error::other(err.to_string()))
    }
}

/// A body that surfaces a deferred decoder-construction error on first read,
/// keeping [`HttpResponse::reader`] infallible.
#[cfg(feature = "compression")]
struct ErrBody(Option<IoError>);

#[cfg(feature = "compression")]
impl ReadBytes for ErrBody {
    fn read_bytes(&mut self, _buf: &mut [u8]) -> Result<usize, IoError> {
        Err(self.0.take().unwrap_or(IoError::UnexpectedEof))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Spawns a one-shot localhost HTTP/1.1 server that replies with `reply` and
    /// hands back the raw request line/headers it received. Hermetic — no network.
    fn serve_once(reply: Vec<u8>) -> (String, std::sync::mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Drain the whole request (headers + any chunked/fixed body) using a
                // short read timeout, so a multi-packet request is captured before we
                // reply — and so the client finishes writing before the socket closes.
                stream
                    .set_read_timeout(Some(std::time::Duration::from_millis(150)))
                    .ok();
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(count) => request.extend_from_slice(&buf[..count]),
                        Err(_) => break, // timed out: assume the request is complete
                    }
                }
                tx.send(request).ok();
                let _ = stream.write_all(&reply);
                let _ = stream.flush();
            }
        });
        (url, rx)
    }

    fn ok_reply(content_type: &str, body: &[u8]) -> Vec<u8> {
        let mut reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        reply.extend_from_slice(body);
        reply
    }

    #[test]
    fn method_parses_and_names() {
        assert_eq!(Method::from_str("get").unwrap(), Method::Get);
        assert_eq!(Method::from_str(" Post ").unwrap(), Method::Post);
        assert_eq!(Method::Delete.as_str(), "DELETE");
        assert!(Method::from_str("teleport").is_err());
    }

    #[test]
    fn get_reads_status_headers_and_text() {
        let (url, _rx) = serve_once(ok_reply("text/plain", b"hello world"));
        let session = HttpSession::new().with_user_agent("yggdryl-http-test");
        let response = session.get(&url).unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.ok());
        assert_eq!(response.content_type(), Some("text/plain"));
        assert_eq!(response.content_length(), Some(11));
        assert_eq!(response.text().unwrap(), "hello world");
    }

    #[test]
    fn post_sends_method_headers_and_body() {
        let (url, rx) = serve_once(ok_reply("application/json", b"{}"));
        let session = HttpSession::new();
        let response = session
            .request(
                HttpRequest::post(&url)
                    .unwrap()
                    .with_header("x-custom", "42")
                    .with_body(b"the-body".to_vec()),
                false,
            )
            .unwrap();
        assert_eq!(response.status(), 200);

        let request = String::from_utf8(rx.recv().unwrap()).unwrap();
        assert!(request.starts_with("POST / HTTP/1.1"), "{request}");
        assert!(request.to_lowercase().contains("x-custom: 42"), "{request}");
        assert!(request.ends_with("the-body"), "{request}");
    }

    #[test]
    fn request_headers_override_session_defaults() {
        let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
        let session = HttpSession::new().with_header("x-tag", "session");
        session
            .request(
                HttpRequest::get(&url)
                    .unwrap()
                    .with_header("x-tag", "request"),
                false,
            )
            .unwrap();
        let request = String::from_utf8(rx.recv().unwrap())
            .unwrap()
            .to_lowercase();
        assert!(request.contains("x-tag: request"), "{request}");
        assert!(!request.contains("x-tag: session"), "{request}");
    }

    #[test]
    fn streamed_request_body_from_an_io_handle() {
        let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
        let session = HttpSession::new();
        // Upload straight from a BytesIO handle, never buffering a Vec.
        let upload = BytesIO::from_bytes(b"streamed-upload-payload".to_vec());
        session
            .request(
                HttpRequest::put(&url).unwrap().with_body_reader(upload),
                false,
            )
            .unwrap();
        let request = String::from_utf8(rx.recv().unwrap()).unwrap();
        assert!(request.starts_with("PUT / HTTP/1.1"), "{request}");
        assert!(request.contains("streamed-upload-payload"), "{request}");
    }

    #[test]
    fn io_body_sets_content_length_and_streams() {
        let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
        let session = HttpSession::new();
        // An Io body knows its length, so the request is framed with Content-Length.
        let upload = BytesIO::from_bytes(b"io-streamed-body".to_vec());
        session
            .request(HttpRequest::put(&url).unwrap().with_body_io(upload), false)
            .unwrap();
        let request = String::from_utf8(rx.recv().unwrap())
            .unwrap()
            .to_lowercase();
        assert!(request.contains("content-length: 16"), "{request}");
        assert!(request.contains("io-streamed-body"), "{request}");
        assert!(!request.contains("transfer-encoding: chunked"), "{request}");
    }

    #[test]
    fn response_body_streams_into_a_bytesio() {
        let (url, _rx) = serve_once(ok_reply("application/octet-stream", &vec![7u8; 5000]));
        let session = HttpSession::new();
        let handle = session.get(&url).unwrap().into_bytesio().unwrap();
        assert_eq!(handle.len(), 5000);
    }

    #[test]
    fn raise_for_status_flags_errors() {
        let reply =
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        let (url, _rx) = serve_once(reply);
        let session = HttpSession::new();
        // raise_error = false returns the 404 response instead of erroring.
        let response = session
            .request(HttpRequest::get(&url).unwrap(), false)
            .unwrap();
        assert_eq!(response.status(), 404);
        assert!(!response.ok());
        assert!(matches!(
            response.raise_for_status(),
            Err(HttpError::Status(404))
        ));
    }

    #[test]
    fn get_raises_on_error_status_by_default() {
        let reply =
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec();
        let (url, _rx) = serve_once(reply);
        // get() uses raise_error = true, so a 500 is an error.
        assert!(matches!(
            HttpSession::new().get(&url),
            Err(HttpError::Status(500))
        ));
    }

    #[cfg(feature = "compression")]
    #[test]
    fn gzip_response_is_decoded_transparently() {
        let body = b"this body was gzip-encoded on the wire".to_vec();
        let packed = yggdryl_compression::Compression::Gzip
            .compress(&body)
            .unwrap();
        let mut reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            packed.len()
        )
        .into_bytes();
        reply.extend_from_slice(&packed);

        let (url, _rx) = serve_once(reply);
        let response = HttpSession::new().get(&url).unwrap();
        assert_eq!(response.content_encoding(), Some("gzip"));
        assert_eq!(response.text().unwrap(), String::from_utf8(body).unwrap());
    }

    #[cfg(feature = "media")]
    #[test]
    fn response_mime_type_from_content_type() {
        let (url, _rx) = serve_once(ok_reply("application/json", b"{}"));
        let response = HttpSession::new().get(&url).unwrap();
        assert_eq!(
            response.mime_type(),
            Some(yggdryl_media::MimeType::from_str("application/json").unwrap())
        );
    }

    // --- multi-request server for the HttpStream / retry / send_many tests ---

    /// Reads one HTTP request off `stream`, returning `(method, path, range)`.
    #[allow(clippy::type_complexity)]
    fn read_request(
        stream: &mut std::net::TcpStream,
    ) -> Option<(String, String, Option<(u64, u64)>)> {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) | Err(_) => return None,
                Ok(_) => buf.push(byte[0]),
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let mut lines = text.lines();
        let first = lines.next()?;
        let mut parts = first.split_whitespace();
        let method = parts.next()?.to_string();
        let path = parts.next()?.to_string();
        let range = text
            .lines()
            .find_map(|line| {
                line.strip_prefix("Range: ")
                    .or_else(|| line.strip_prefix("range: "))
            })
            .and_then(|value| value.trim().strip_prefix("bytes="))
            .and_then(|spec| {
                let (start, end) = spec.split_once('-')?;
                // An open-ended range (`bytes=5000-`) has an empty end: treat it as
                // "to the end" (the server clamps to the payload length).
                let end = if end.is_empty() {
                    u64::MAX
                } else {
                    end.parse().ok()?
                };
                Some((start.parse().ok()?, end))
            });
        Some((method, path, range))
    }

    /// A looping server that serves `payload` with HEAD + `Range` (206) support.
    /// Each connection runs on its own thread and serves **multiple** requests
    /// (HTTP/1.1 keep-alive), so it exercises the pooled-connection reuse path.
    fn serve_ranges(payload: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let payload = std::sync::Arc::new(payload);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let payload = payload.clone();
                thread::spawn(move || {
                    let mut stream = stream;
                    let _ = stream.set_nodelay(true);
                    let total = payload.len();
                    // Keep serving requests on this connection until the peer
                    // closes it or asks to (`Connection: close`).
                    while let Some((method, _path, range)) = read_request(&mut stream) {
                        if method == "HEAD" {
                            let head = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                            );
                            if stream.write_all(head.as_bytes()).is_err() {
                                break;
                            }
                        } else if let Some((start, end)) = range {
                            let end = (end as usize).min(total.saturating_sub(1));
                            let slice = &payload[start as usize..=end];
                            let header = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
                                slice.len()
                            );
                            if stream.write_all(header.as_bytes()).is_err()
                                || stream.write_all(slice).is_err()
                            {
                                break;
                            }
                        } else {
                            let header =
                                format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n");
                            if stream.write_all(header.as_bytes()).is_err()
                                || stream.write_all(&payload).is_err()
                            {
                                break;
                            }
                        }
                    }
                });
            }
        });
        url
    }

    fn stream_payload() -> Vec<u8> {
        (0..10_000u32).map(|n| (n % 251) as u8).collect()
    }

    #[test]
    fn httpstream_discovers_size_and_reads_sequentially() {
        let payload = stream_payload();
        let url = serve_ranges(payload.clone());
        let session = HttpSession::new();
        let mut stream = session
            .stream(HttpRequest::get(&url).unwrap(), true)
            .unwrap();
        assert_eq!(stream.size(), Some(payload.len() as u64));
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        assert_eq!(out, payload);
    }

    #[test]
    fn httpstream_seek_and_positional_pread() {
        use yggdryl_io::Io;
        let payload = stream_payload();
        let url = serve_ranges(payload.clone());
        let mut stream = HttpSession::new()
            .stream(HttpRequest::get(&url).unwrap(), true)
            .unwrap();

        // Seek then sequential read.
        stream.seek(5000, Whence::Start).unwrap();
        let mut buf = [0u8; 100];
        stream.read_bytes(&mut buf).unwrap();
        assert_eq!(&buf[..], &payload[5000..5100]);

        // A footer pread leaves the cursor (still at 5100) untouched.
        let mut footer = [0u8; 20];
        let n = stream.pread(&mut footer, -20, Whence::End).unwrap();
        assert_eq!(&footer[..n], &payload[payload.len() - 20..]);
        assert_eq!(Seek::stream_position(&stream), 5100);
    }

    #[cfg(feature = "media")]
    #[test]
    fn httpstream_is_an_io_with_url_and_stats() {
        use yggdryl_io::Io;
        let payload = stream_payload();
        let url = serve_ranges(payload.clone());
        let stream = HttpSession::new()
            .stream(HttpRequest::get(&url).unwrap(), true)
            .unwrap();
        assert_eq!(stream.url().scheme(), "http");
        let stats = stream.stats().unwrap();
        assert_eq!(stats.size(), payload.len() as u64);
    }

    #[test]
    fn retries_429_then_succeeds() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicU32::new(0));
        let counter = hits.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let _ = read_request(&mut stream);
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    let _ = stream.write_all(
                        b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\n\r\n",
                    );
                } else {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                }
            }
        });
        let session = HttpSession::new().with_retry(RetryConfig {
            max_retries: 5,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        });
        let response = session.get(&url).unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 3); // two 429s, then 200
    }

    #[test]
    fn httpstream_resumes_after_a_dropped_connection() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let payload = stream_payload();
        let served = payload.clone();
        let hits = Arc::new(AtomicU32::new(0));
        let counter = hits.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let request = read_request(&mut stream);
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let total = served.len();
                if n == 0 {
                    // Initial streaming GET (no Range): promise the full body via
                    // Content-Length but send only a prefix, then drop the socket —
                    // the client sees a truncated body mid-stream.
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&served[..100]); // far short of Content-Length
                                                              // stream dropped here -> client sees a truncated/aborted body
                } else if let Some((_, _, Some((start, end)))) = request {
                    // Resume: serve the requested range to the end.
                    let end = (end as usize).min(total - 1);
                    let slice = &served[start as usize..=end];
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
                        slice.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(slice);
                }
            }
        });
        let session = HttpSession::new().with_retry(RetryConfig {
            max_retries: 5,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        });
        let mut stream = session
            .stream(HttpRequest::get(&url).unwrap(), true)
            .unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        assert_eq!(out, payload); // resumed and completed despite the mid-stream drop
        assert!(hits.load(Ordering::SeqCst) >= 2); // dropped streaming GET, then resumed range GET
    }

    #[test]
    fn send_many_streams_batches() {
        let url = serve_ranges(b"hello".to_vec());
        let session = HttpSession::new()
            .with_max_concurrency(4)
            .with_batch_size(3);
        let requests: Vec<HttpRequest> = (0..7).map(|_| HttpRequest::get(&url).unwrap()).collect();
        let batches: Vec<HttpResponseBatch> = session.send_many(requests).collect();
        assert_eq!(batches.len(), 3); // 3 + 3 + 1
        let total: usize = batches.iter().map(|b| b.len()).sum();
        assert_eq!(total, 7);
        for batch in batches {
            for result in batch {
                assert_eq!(result.unwrap().status(), 200);
            }
        }
    }

    /// A server that never advertises a `Content-Length` (size unknown): a plain
    /// `GET` streams the whole body then closes the socket (close-delimited), and a
    /// `Range` request answers 206 from the offset, or 416 once it starts past the
    /// end. Each connection is one-shot (closed after the reply) so the unknown
    /// size is delimited by the close.
    fn serve_unknown_size(payload: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let payload = std::sync::Arc::new(payload);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let payload = payload.clone();
                thread::spawn(move || {
                    let mut stream = stream;
                    let _ = stream.set_nodelay(true);
                    let Some((method, _path, range)) = read_request(&mut stream) else {
                        return;
                    };
                    let total = payload.len();
                    if method == "HEAD" {
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        );
                    } else if let Some((start, end)) = range {
                        if start as usize >= total {
                            let _ = stream.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        } else {
                            let end = (end as usize).min(total - 1);
                            let slice = &payload[start as usize..=end];
                            let header = format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                slice.len()
                            );
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(slice);
                        }
                    } else {
                        // Plain GET, no range: stream the whole body with no
                        // Content-Length and close — the client learns the size at
                        // EOF (the socket close).
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        );
                        let _ = stream.write_all(&payload);
                    }
                });
            }
        });
        url
    }

    #[test]
    fn httpstream_unknown_size_reads_to_eof() {
        let payload = stream_payload();
        let url = serve_unknown_size(payload.clone());
        let mut stream = HttpSession::new()
            .stream(HttpRequest::get(&url).unwrap(), false)
            .unwrap();
        assert_eq!(stream.size(), None); // size not advertised
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        assert_eq!(out, payload); // the close-delimited body ends the read cleanly
        assert_eq!(stream.size(), Some(payload.len() as u64)); // discovered at EOF
    }

    #[test]
    fn httpstream_range_past_end_is_clean_eof_via_416() {
        use yggdryl_io::Io;
        let payload = stream_payload();
        let url = serve_unknown_size(payload.clone());
        let mut stream = HttpSession::new()
            .stream(HttpRequest::get(&url).unwrap(), false)
            .unwrap();
        // Size is unknown, so the past-the-end guard can't short-circuit: the
        // request is issued and the server's 416 surfaces as a clean 0-byte read.
        let mut buf = [0u8; 16];
        let n = stream
            .pread(&mut buf, payload.len() as i64, Whence::Start)
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn httpstream_close_releases_the_connection_and_reads_eof() {
        use yggdryl_io::Io;
        let payload = stream_payload();
        let url = serve_ranges(payload);
        let session = HttpSession::new();
        let mut stream = session
            .stream(HttpRequest::get(&url).unwrap(), true)
            .unwrap();
        assert_eq!(session.open_streams(), 1); // the stream holds one connection
        let mut head = [0u8; 16];
        assert_eq!(stream.read_bytes(&mut head).unwrap(), 16);
        Io::close(&mut stream).unwrap();
        let mut more = [0u8; 16];
        assert_eq!(stream.read_bytes(&mut more).unwrap(), 0); // closed -> clean EOF
        drop(stream);
        assert_eq!(session.open_streams(), 0); // connection released on drop
    }

    #[test]
    fn keep_alive_false_sends_connection_close() {
        let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
        HttpSession::new()
            .send(HttpRequest::get(&url).unwrap(), false, false)
            .unwrap();
        let request = String::from_utf8(rx.recv().unwrap())
            .unwrap()
            .to_lowercase();
        assert!(request.contains("connection: close"), "{request}");
    }

    #[test]
    fn keep_alive_true_does_not_close_the_connection() {
        let (url, rx) = serve_once(ok_reply("text/plain", b"ok"));
        HttpSession::new()
            .send(HttpRequest::get(&url).unwrap(), false, true)
            .unwrap();
        let request = String::from_utf8(rx.recv().unwrap())
            .unwrap()
            .to_lowercase();
        assert!(!request.contains("connection: close"), "{request}");
    }

    #[test]
    fn pool_safeguard_closes_extra_streams_when_saturated() {
        use std::sync::{Arc, Mutex};
        // Records every request line/headers, replying 200 with the payload.
        let payload = stream_payload();
        let served = payload.clone();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorder = seen.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let served = served.clone();
                let recorder = recorder.clone();
                thread::spawn(move || {
                    let mut stream = stream;
                    let _ = stream.set_nodelay(true);
                    let mut buf = Vec::new();
                    let mut byte = [0u8; 1];
                    while !buf.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte) {
                            Ok(0) | Err(_) => return,
                            Ok(_) => buf.push(byte[0]),
                        }
                    }
                    recorder
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf).into_owned());
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
                        served.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&served);
                });
            }
        });
        // A pool that holds a single keep-alive connection.
        let session = HttpSession::new().with_pool_size(1);
        assert_eq!(session.pool_size(), 1);
        // The first stream fills the pool; the second is over capacity, so the
        // safeguard forces it to close (it must not starve the keep-alive pool).
        let s1 = session
            .stream(HttpRequest::get(&url).unwrap(), true)
            .unwrap();
        let s2 = session
            .stream(HttpRequest::get(&url).unwrap(), true)
            .unwrap();
        assert_eq!(session.open_streams(), 2);
        drop(s1);
        drop(s2);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        let closing = seen
            .iter()
            .filter(|r| r.to_lowercase().contains("connection: close"))
            .count();
        assert_eq!(
            closing, 1,
            "exactly the over-capacity stream closes: {seen:?}"
        );
    }

    #[test]
    fn keep_alive_requests_release_the_connection_on_eof() {
        let url = serve_ranges(stream_payload());
        let session = HttpSession::new();
        for _ in 0..5 {
            // Each drained response returns its connection to the pool (EOF), so no
            // stream is left holding one between requests.
            let _ = session.get(&url).unwrap().bytes().unwrap();
            assert_eq!(session.open_streams(), 0);
        }
    }

    #[test]
    fn httpstream_reconnects_through_multiple_drops() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        let payload = stream_payload();
        let served = payload.clone();
        let hits = Arc::new(AtomicU32::new(0));
        let counter = hits.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let request = read_request(&mut stream);
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let total = served.len();
                // The resume cursor is the start of the requested range (0 first).
                let start = match &request {
                    Some((_, _, Some((start, _)))) => *start as usize,
                    _ => 0,
                };
                let header = if start == 0 {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                    )
                } else {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{total}\r\nContent-Length: {}\r\n\r\n",
                        total - 1,
                        total - start
                    )
                };
                let _ = stream.write_all(header.as_bytes());
                if n < 3 {
                    // Truncate the body 1000 bytes in, then drop — forcing a fresh
                    // mid-stream reconnect each time.
                    let chunk_end = (start + 1000).min(total);
                    let _ = stream.write_all(&served[start..chunk_end]);
                } else {
                    // Finally serve the remainder in full.
                    let _ = stream.write_all(&served[start..]);
                }
            }
        });
        let session = HttpSession::new().with_retry(RetryConfig {
            max_retries: 10,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        });
        let mut stream = session
            .stream(HttpRequest::get(&url).unwrap(), true)
            .unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        assert_eq!(out, payload); // resumed through every drop and completed
        assert!(hits.load(Ordering::SeqCst) >= 4); // 3 truncated drops, then the full tail
    }

    #[test]
    fn retry_exhaustion_surfaces_the_error_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let _ = read_request(&mut stream);
                let _ = stream.write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let session = HttpSession::new().with_retry(RetryConfig {
            max_retries: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        });
        // After exhausting retries the persistent 503 is returned, and get() raises.
        assert!(matches!(session.get(&url), Err(HttpError::Status(503))));
    }

    #[test]
    fn send_many_handles_an_empty_iterator() {
        let session = HttpSession::new();
        let batches: Vec<HttpResponseBatch> = session.send_many(Vec::new()).collect();
        assert!(batches.is_empty());
    }

    #[test]
    fn io_json_parses_a_response_body() {
        use yggdryl_io::Io;
        let (url, _rx) = serve_once(ok_reply("application/json", br#"{"a":1,"b":[2,3]}"#));
        let mut handle = HttpSession::new()
            .get(&url)
            .unwrap()
            .into_bytesio()
            .unwrap();
        let value = handle.json().unwrap();
        assert_eq!(value["a"].as_u64(), Some(1));
        assert_eq!(value["b"][0].as_u64(), Some(2));
        assert_eq!(value["b"][1].as_u64(), Some(3));
    }
}
