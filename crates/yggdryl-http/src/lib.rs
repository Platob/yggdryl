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
//! via [`with_body_reader`](HttpRequest::with_body_reader) — uploading a
//! [`LocalPath`](yggdryl_io::LocalPath) never loads the file into memory.
//!
//! ```no_run
//! use yggdryl_http::{HttpSession, Method};
//!
//! let session = HttpSession::new().with_user_agent("yggdryl-http/0.1");
//! let response = session.get("https://example.com").unwrap();
//! assert!(response.ok());
//! let body = response.text().unwrap();
//! ```
//!
//! ## Optional features (off by default)
//!
//! - `compression` — transparently decode a `Content-Encoding` (gzip / zstd /
//!   snappy) response body through `yggdryl-compression`, the way `requests`
//!   auto-decompresses.
//! - `media` — expose the response's [`mime_type`](HttpResponse::mime_type).
//! - `log` — structured `log` events on the request path.

use std::fmt;

use yggdryl_io::{BytesIO, IoError, ReadBytes};
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
    /// An in-memory byte body.
    Bytes(Vec<u8>),
    /// A streamed body pulled from any byte source (e.g. an [`Io`](yggdryl_io::Io)
    /// handle), sent without buffering it into memory first.
    Reader(Box<dyn ReadBytes + Send>),
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
}

impl HttpSession {
    /// Creates a session with a fresh connection pool and no default headers.
    ///
    /// Like `requests`, a 4xx/5xx status is returned as a normal
    /// [`HttpResponse`] (not an error) — call
    /// [`raise_for_status`](HttpResponse::raise_for_status) to opt into raising.
    pub fn new() -> HttpSession {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        HttpSession {
            agent,
            headers: Vec::new(),
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

    /// The session's default headers.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// `GET url`.
    pub fn get(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::get(url)?)
    }

    /// `HEAD url`.
    pub fn head(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::head(url)?)
    }

    /// `DELETE url`.
    pub fn delete(&self, url: &str) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::delete(url)?)
    }

    /// `POST url` with an in-memory byte body.
    pub fn post(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::post(url)?.with_body(body))
    }

    /// `PUT url` with an in-memory byte body.
    pub fn put(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::put(url)?.with_body(body))
    }

    /// `PATCH url` with an in-memory byte body.
    pub fn patch(&self, url: &str, body: impl Into<Vec<u8>>) -> Result<HttpResponse, HttpError> {
        self.request(HttpRequest::patch(url)?.with_body(body))
    }

    /// Sends a fully-built [`HttpRequest`], applying the session's default headers
    /// first, and returns the [`HttpResponse`] (its body still unread).
    pub fn request(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let url_string = request.url.to_string();
        log_event!(
            debug,
            "HttpSession::request {} {url_string}",
            request.method.as_str()
        );

        let mut builder = ureq::http::Request::builder()
            .method(request.method.as_str())
            .uri(url_string.as_str());
        // Apply session defaults, then the request's own headers — but a default
        // is skipped when the request sets the same name, so a per-request header
        // overrides the session default (the `requests` merge semantics).
        for (name, value) in &self.headers {
            let overridden = request
                .headers
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case(name));
            if !overridden {
                builder = builder.header(name, value);
            }
        }
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }

        // Each arm builds and runs its own request so the body source (a stack
        // local) stays alive across `agent.run`, which borrows it. An empty body
        // uses `SendBody::none()` so no `Content-Length` framing is added.
        let response = match request.body {
            Body::Empty => self.agent.run(builder.body(ureq::SendBody::none())?)?,
            // A known-length byte body is sent with `Content-Length` (not chunked).
            Body::Bytes(bytes) => self.agent.run(builder.body(bytes)?)?,
            Body::Reader(reader) => {
                let mut reader = ReadBridge(reader);
                self.agent
                    .run(builder.body(ureq::SendBody::from_reader(&mut reader))?)?
            }
        };
        Ok(HttpResponse::from_ureq(response, request.url))
    }
}

impl Default for HttpSession {
    fn default() -> HttpSession {
        HttpSession::new()
    }
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
            .request(HttpRequest::put(&url).unwrap().with_body_reader(upload))
            .unwrap();
        let request = String::from_utf8(rx.recv().unwrap()).unwrap();
        assert!(request.starts_with("PUT / HTTP/1.1"), "{request}");
        assert!(request.contains("streamed-upload-payload"), "{request}");
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
        let response = HttpSession::new().get(&url).unwrap();
        assert_eq!(response.status(), 404);
        assert!(!response.ok());
        assert!(matches!(
            response.raise_for_status(),
            Err(HttpError::Status(404))
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
}
