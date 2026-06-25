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
//! whose size is discoverable up front and whose bytes are fetched lazily in
//! 4 MiB windows via `Range` requests — with transient-failure retries and
//! cursor resume on a dropped connection. [`HttpSession::send_many`] runs an
//! iterator of requests concurrently in batches.
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
//! let mut stream = session.stream(HttpRequest::get("https://example.com/data").unwrap()).unwrap();
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
}

/// A received HTTP response, modelled on `requests.Response`: a status, headers,
/// and a body that is read lazily.
pub struct HttpResponse {
    status: u16,
    url: Url,
    headers: Vec<(String, String)>,
    body: ureq::Body,
}

impl HttpResponse {
    fn from_ureq(response: ureq::http::Response<ureq::Body>, url: Url) -> HttpResponse {
        let (parts, body) = response.into_parts();
        let headers = parts
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        HttpResponse {
            status: parts.status.as_u16(),
            url,
            headers,
            body,
        }
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
        let HttpResponse { headers, body, .. } = self;
        let raw = ReadAdapter(body.into_reader());

        #[cfg(feature = "compression")]
        {
            // Resolve the codec before touching `raw`, so the fall-through to the
            // undecoded body never trips over a moved value.
            let codec = header_value(&headers, "content-encoding")
                .and_then(|encoding| yggdryl_compression::Compression::from_str(encoding).ok())
                .filter(|codec| {
                    *codec != yggdryl_compression::Compression::None && codec.is_available()
                });
            if let Some(codec) = codec {
                log_event!(debug, "HttpResponse::reader decoding {codec}");
                return match codec.decoder(raw) {
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
            inner: Box::new(raw),
        }
    }

    /// Drains the body into a `Vec<u8>` (decompressing under the `compression`
    /// feature).
    pub fn bytes(self) -> Result<Vec<u8>, HttpError> {
        let mut out = Vec::new();
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
}

impl HttpSession {
    /// Creates a session with a fresh connection pool, default retry policy, a
    /// concurrency of 8 and a batch size of 80 (`max_concurrency * 10`).
    pub fn new() -> HttpSession {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let max_concurrency = 8;
        HttpSession {
            agent,
            headers: Vec::new(),
            retry: RetryConfig::default(),
            max_concurrency,
            batch_size: max_concurrency * 10,
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

    /// Sends a [`prepare`](HttpSession::prepare)d request with the retry policy.
    /// When `raise_error` is `true` a 4xx/5xx status becomes an
    /// [`HttpError::Status`] (the `requests` `raise_for_status` default); pass
    /// `false` to receive the response whatever its status.
    pub fn request(
        &self,
        request: HttpRequest,
        raise_error: bool,
    ) -> Result<HttpResponse, HttpError> {
        let request = self.prepare(request);
        let url = request.url.clone();
        log_event!(
            debug,
            "HttpSession::request {} {url}",
            request.method.as_str()
        );
        let response = self.execute(
            request.method,
            url.to_string().as_str(),
            &request.headers,
            request.body,
        )?;
        let response = HttpResponse::from_ureq(response, url);
        if raise_error {
            response.raise_for_status()
        } else {
            Ok(response)
        }
    }

    /// Opens a seekable [`HttpStream`] over a resource: a `HEAD` discovers its
    /// size / content type, then bytes are fetched lazily in 4 MiB windows via
    /// `Range` requests (only when read). The request supplies the URL and any
    /// headers (e.g. auth); its method/body are ignored.
    pub fn stream(&self, request: HttpRequest) -> Result<HttpStream, HttpError> {
        let request = self.prepare(request);
        let url_string = request.url.to_string();
        let head = self.execute(Method::Head, &url_string, &request.headers, Body::Empty)?;
        let headers = head.headers();
        let size = header_u64(headers, "content-length");
        let content_type = header_string(headers, "content-type");
        let ranges = header_string(headers, "accept-ranges")
            .map(|value| value.eq_ignore_ascii_case("bytes"))
            .unwrap_or(false);
        log_event!(
            debug,
            "HttpStream::open {url_string} size={size:?} ranges={ranges}"
        );
        Ok(HttpStream {
            agent: self.agent.clone(),
            url: request.url,
            headers: request.headers,
            retry: self.retry.clone(),
            size,
            content_type,
            ranges,
            position: 0,
            window_start: 0,
            window: Vec::new(),
        })
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

/// A seekable, lazily-fetched [`Io`] over an HTTP resource: a `HEAD` makes its
/// size and content type discoverable up front, then bytes are pulled on demand
/// in **4 MiB windows** via `Range` requests — random access (`pread`, a footer
/// read) issues a one-off range, while sequential [`read`](ReadBytes) walks the
/// window. Reads retry transient failures and **resume from the cursor** on a
/// dropped connection (each window is an independent, idempotent range request).
pub struct HttpStream {
    agent: ureq::Agent,
    url: Url,
    headers: Vec<(String, String)>,
    retry: RetryConfig,
    size: Option<u64>,
    content_type: Option<String>,
    ranges: bool,
    position: u64,
    window_start: u64,
    window: Vec<u8>,
}

/// The window size for sequential [`HttpStream`] reads (4 MiB).
const WINDOW_SIZE: u64 = 4 * 1024 * 1024;

impl HttpStream {
    /// The total size in bytes, if the server reported it.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    /// The `Content-Type`, if the server reported it.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Whether the server advertised `Accept-Ranges: bytes`.
    pub fn supports_ranges(&self) -> bool {
        self.ranges
    }

    /// Fetches `[start, start+len)` with the retry policy — retrying transient
    /// statuses and reconnecting (resuming the same range) on a lost connection.
    fn fetch_range(&self, start: u64, len: u64) -> Result<Vec<u8>, IoError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let url = self.url.to_string();
        let range = format!("bytes={}-{}", start, start + len - 1);
        let mut attempt = 0u32;
        loop {
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
                    if status >= 400 {
                        return Err(IoError::Io(format!("http status {status}")));
                    }
                    // 200 means the server ignored the range and sent the whole body.
                    if status == 200 && start > 0 {
                        return Err(IoError::Unsupported(
                            "server does not support Range requests".to_string(),
                        ));
                    }
                    let mut reader = response.body_mut().as_reader();
                    let mut out = Vec::new();
                    match std::io::Read::read_to_end(&mut reader, &mut out) {
                        Ok(_) => {
                            if status == 200 {
                                out.truncate(len as usize);
                            }
                            return Ok(out);
                        }
                        // A body cut mid-stream (dropped connection): re-fetch the
                        // same range — i.e. resume from the cursor.
                        Err(_error) if attempt < self.retry.max_retries => {
                            let delay = self.retry.backoff(attempt, None);
                            log_event!(warn, "HttpStream resume mid-body after {_error}");
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

    /// The number of bytes still readable from `position`, if the size is known.
    fn remaining(&self) -> Option<u64> {
        self.size.map(|size| size.saturating_sub(self.position))
    }
}

impl fmt::Debug for HttpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpStream")
            .field("url", &self.url.to_string())
            .field("size", &self.size)
            .field("position", &self.position)
            .field("window", &(self.window_start, self.window.len()))
            .finish()
    }
}

impl ReadBytes for HttpStream {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        if buf.is_empty() || self.remaining() == Some(0) {
            return Ok(0);
        }
        let in_window = self.position >= self.window_start
            && self.position < self.window_start + self.window.len() as u64;
        if !in_window {
            let len = self.remaining().map_or(WINDOW_SIZE, |r| r.min(WINDOW_SIZE));
            let bytes = self.fetch_range(self.position, len)?;
            if bytes.is_empty() {
                return Ok(0);
            }
            self.window_start = self.position;
            self.window = bytes;
        }
        let offset = (self.position - self.window_start) as usize;
        let available = &self.window[offset..];
        let count = buf.len().min(available.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base = match whence {
            Whence::Start => 0i64,
            Whence::Current => self.position as i64,
            Whence::End => self
                .size
                .ok_or_else(|| IoError::Unsupported("seek from end: unknown size".to_string()))?
                as i64,
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

    /// A positional read via a one-off `Range` request, leaving the window and —
    /// for [`Whence::Start`]/[`Whence::End`] — the cursor untouched, so a footer
    /// or header can be discovered without disturbing a sequential scan.
    fn pread(&mut self, buf: &mut [u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        let base = match whence {
            Whence::Start => 0i64,
            Whence::Current => self.position as i64,
            Whence::End => self
                .size
                .ok_or_else(|| IoError::Unsupported("pread from end: unknown size".to_string()))?
                as i64,
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

/// Bridges a [`ReadBytes`] request body to [`std::io::Read`] for the transport.
struct ReadBridge(Box<dyn ReadBytes + Send>);

impl std::io::Read for ReadBridge {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0
            .read_bytes(buf)
            .map_err(|err| std::io::Error::other(err.to_string()))
    }
}

/// Adapts the transport's [`std::io::Read`] response body to [`ReadBytes`], the
/// hook that lets a response stream over the yggdryl-io abstraction.
struct ReadAdapter<R: std::io::Read>(R);

impl<R: std::io::Read> ReadBytes for ReadAdapter<R> {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        self.0.read(buf).map_err(IoError::from)
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
                Some((start.parse().ok()?, end.parse().ok()?))
            });
        Some((method, path, range))
    }

    /// A looping server that serves `payload` with HEAD + `Range` (206) support.
    fn serve_ranges(payload: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                let Some((method, _path, range)) = read_request(&mut stream) else {
                    continue;
                };
                let total = payload.len();
                if method == "HEAD" {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                    );
                    let _ = stream.write_all(head.as_bytes());
                } else if let Some((start, end)) = range {
                    let end = (end as usize).min(total.saturating_sub(1));
                    let slice = &payload[start as usize..=end];
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{total}\r\nContent-Length: {}\r\n\r\n",
                        slice.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(slice);
                } else {
                    let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\n\r\n");
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&payload);
                }
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
        let mut stream = session.stream(HttpRequest::get(&url).unwrap()).unwrap();
        assert_eq!(stream.size(), Some(payload.len() as u64));
        assert!(stream.supports_ranges());
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
            .stream(HttpRequest::get(&url).unwrap())
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
            .stream(HttpRequest::get(&url).unwrap())
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
                if n == 0 && request.as_ref().is_some_and(|(m, ..)| m == "HEAD") {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nAccept-Ranges: bytes\r\n\r\n"
                    );
                    let _ = stream.write_all(head.as_bytes());
                } else if n == 1 {
                    // First range GET: send a truncated body, then drop the socket.
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{total}\r\nContent-Length: {total}\r\n\r\n",
                        total - 1
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&served[..100]); // far short of Content-Length
                                                              // stream dropped here -> client sees a truncated/aborted body
                } else if let Some((_, _, Some((start, end)))) = request {
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
        let mut stream = session.stream(HttpRequest::get(&url).unwrap()).unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        assert_eq!(out, payload); // resumed and completed despite the mid-stream drop
        assert!(hits.load(Ordering::SeqCst) >= 3); // HEAD, dropped GET, resumed GET
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
