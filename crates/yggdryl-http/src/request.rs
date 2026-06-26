//! The [`HttpRequest`] builder and its private [`Body`].

use std::time::Duration;

use yggdryl_core::Io;
use yggdryl_core::Url;

use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::method::Method;
use crate::response::HttpResponse;
use crate::session::HttpSession;
use crate::version::HttpVersion;

/// The default connection keep-alive idle time-to-live: a pooled connection
/// unused for this long is dropped. Five minutes, matching common server defaults.
pub(crate) const DEFAULT_KEEP_ALIVE: Duration = Duration::from_secs(300);

/// The body carried by an [`HttpRequest`].
pub(crate) enum Body {
    /// No body.
    Empty,
    /// An in-memory byte body (replayable, so it can be retried).
    Bytes(Vec<u8>),
    /// A streamed body pulled from any [`Io`] handle, sent without buffering.
    Reader(Box<dyn Io>),
    /// A streamed body from an [`Io`] handle: its
    /// [`stream_len`](yggdryl_core::Io::stream_len) sets `Content-Length` (so the
    /// upload is framed, not chunked) and the bytes flow straight off the handle —
    /// never collected into memory.
    Io(Box<dyn Io>),
}

impl Body {
    /// Whether the body can be re-sent on a retry (no consumed reader).
    pub(crate) fn replayable(&self) -> bool {
        matches!(self, Body::Empty | Body::Bytes(_))
    }

    /// A re-sendable copy of a replayable body (for a redirect re-dispatch), or
    /// [`Body::Empty`] for a single-shot streamed body that cannot be replayed.
    pub(crate) fn replay_copy(&self) -> Body {
        match self {
            Body::Empty => Body::Empty,
            Body::Bytes(bytes) => Body::Bytes(bytes.clone()),
            Body::Reader(_) | Body::Io(_) => Body::Empty,
        }
    }
}

/// A builder for one HTTP request: a [`Method`], a [`Url`], headers, and an
/// optional body. Send it with [`HttpSession::send`](crate::HttpSession::send) (or
/// [`request`](crate::HttpSession::request)).
///
/// The `with_*` methods are non-mutating in spirit (they consume and return
/// `self`), mirroring the rest of the project's builders.
pub struct HttpRequest {
    pub(crate) method: Method,
    pub(crate) url: Url,
    pub(crate) headers: HttpHeaders,
    pub(crate) body: Body,
    /// Whether [`send`](crate::HttpSession::send) follows 3xx redirects for this
    /// request (default `true`).
    pub(crate) allow_redirect: bool,
    /// How long the connection may stay pooled, idle, before it is dropped — the
    /// keep-alive idle TTL (default [`DEFAULT_KEEP_ALIVE`], 5 minutes). `Duration::
    /// ZERO` disables pooling (the request carries `Connection: close`).
    pub(crate) keep_alive: Duration,
    /// The pinned HTTP protocol version for this request, or `None` to inherit the
    /// session's [`http_version`](crate::HttpSession::http_version).
    pub(crate) http_version: Option<HttpVersion>,
}

impl HttpRequest {
    /// Builds a request for `method` and `url`, returning [`HttpError::InvalidUrl`]
    /// if the URL is malformed.
    pub fn new(method: Method, url: &str) -> Result<HttpRequest, HttpError> {
        let url = Url::from_str(url).map_err(|err| HttpError::InvalidUrl(err.to_string()))?;
        Ok(HttpRequest {
            method,
            url,
            headers: HttpHeaders::new(),
            body: Body::Empty,
            allow_redirect: true,
            keep_alive: DEFAULT_KEEP_ALIVE,
            http_version: None,
        })
    }

    /// Builds a request from an already-parsed [`Url`].
    pub fn from_url(method: Method, url: Url) -> HttpRequest {
        HttpRequest {
            method,
            url,
            headers: HttpHeaders::new(),
            body: Body::Empty,
            allow_redirect: true,
            keep_alive: DEFAULT_KEEP_ALIVE,
            http_version: None,
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
        self.headers.insert(name, value);
        self
    }

    /// Adds every `(name, value)` pair as a header.
    pub fn with_headers<I, K, V>(mut self, headers: I) -> HttpRequest
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (name, value) in headers {
            self.headers.insert(name, value);
        }
        self
    }

    /// Adds a query parameter to the URL (percent-encoding `value`).
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> HttpRequest {
        self.url = self.url.add_param(key, vec![value.into()], true);
        self
    }

    /// Sets the `Authorization` header to HTTP Basic credentials
    /// (`Basic base64(username:password)`, RFC 7617), replacing any existing one.
    ///
    /// ```
    /// use yggdryl_http::HttpRequest;
    ///
    /// let request = HttpRequest::get("https://example.com")
    ///     .unwrap()
    ///     .with_basic_auth("Aladdin", "open sesame");
    /// assert_eq!(
    ///     request.headers().get("authorization"),
    ///     Some("Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="),
    /// );
    /// ```
    pub fn with_basic_auth(mut self, username: &str, password: &str) -> HttpRequest {
        self.headers.set(
            "authorization",
            crate::auth::basic_auth_header(username, password),
        );
        self
    }

    /// Sets the `Authorization` header to an HTTP Bearer token
    /// (`Bearer <token>`, RFC 6750), replacing any existing one.
    pub fn with_bearer_auth(mut self, token: &str) -> HttpRequest {
        self.headers
            .set("authorization", crate::auth::bearer_auth_header(token));
        self
    }

    /// Sets an in-memory byte body.
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> HttpRequest {
        self.body = Body::Bytes(body.into());
        self
    }

    /// Sets a **streamed** body pulled from any [`Io`] handle — e.g. a
    /// [`LocalPath`](yggdryl_core::LocalPath) or [`BytesIO`](yggdryl_core::BytesIO) —
    /// so a large upload is never buffered into memory.
    pub fn with_body_reader<R: Io + 'static>(mut self, reader: R) -> HttpRequest {
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

    /// Sets whether [`send`](crate::HttpSession::send) follows 3xx redirects for
    /// this request (default `true`). With `false` a redirect is returned as the
    /// 3xx response itself.
    pub fn with_allow_redirect(mut self, allow_redirect: bool) -> HttpRequest {
        self.allow_redirect = allow_redirect;
        self
    }

    /// Sets the keep-alive idle TTL in `seconds`: how long the connection may sit
    /// idle in the pool before it is dropped (default 300 — 5 minutes). A positive
    /// TTL pools the connection so the next request skips the TLS handshake; `0`
    /// (or negative) disables pooling, sending `Connection: close` so the socket is
    /// released the moment the body is drained. A pool-saturation safeguard can
    /// still force `close` on a streamed body past the pool size.
    pub fn with_keep_alive(mut self, seconds: f64) -> HttpRequest {
        self.keep_alive = if seconds > 0.0 {
            Duration::from_secs_f64(seconds)
        } else {
            Duration::ZERO
        };
        self
    }

    /// Pins the HTTP protocol [`version`](HttpVersion) for this request, overriding
    /// the session's default. [`send`](crate::HttpSession::send) errors with
    /// [`HttpError::Unsupported`] if the pinned version has no wired transport (e.g.
    /// [`Http2`](HttpVersion::Http2) today) rather than silently downgrading.
    pub fn with_http_version(mut self, http_version: HttpVersion) -> HttpRequest {
        self.http_version = Some(http_version);
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
    pub fn headers(&self) -> &HttpHeaders {
        &self.headers
    }

    /// Whether [`send`](crate::HttpSession::send) follows 3xx redirects for this
    /// request.
    pub fn allow_redirect(&self) -> bool {
        self.allow_redirect
    }

    /// The keep-alive idle TTL in seconds (`0.0` means pooling is disabled).
    pub fn keep_alive(&self) -> f64 {
        self.keep_alive.as_secs_f64()
    }

    /// The pinned HTTP protocol [`version`](HttpVersion) for this request, or
    /// `None` when it inherits the session's
    /// [`http_version`](crate::HttpSession::http_version).
    pub fn http_version(&self) -> Option<HttpVersion> {
        self.http_version
    }

    /// An independent copy of this request — same method, URL, headers and
    /// settings, ready to send again. A replayable body (none / in-memory bytes)
    /// is copied; a **streamed** body (a reader or [`Io`] handle) cannot be
    /// duplicated, so the copy carries no body (build it with a fresh handle).
    pub fn copy(&self) -> HttpRequest {
        HttpRequest {
            method: self.method,
            url: self.url.clone(),
            headers: self.headers.clone(),
            body: self.body.replay_copy(),
            allow_redirect: self.allow_redirect,
            keep_alive: self.keep_alive,
            http_version: self.http_version,
        }
    }

    /// Sends this request through the shared per-host
    /// [`HttpSession`](crate::HttpSession::shared_for) (the singleton for the
    /// request URL's host) and returns the [`HttpResponse`] — the self-contained
    /// entry point, so a request needs no session reference to fetch a response.
    /// `raise_error` turns a 4xx/5xx status into an [`HttpError::Status`]. For a
    /// custom-configured client (its own pool, TLS, proxy, default headers) send it
    /// through that session instead with
    /// [`HttpSession::send`](crate::HttpSession::send).
    pub fn send(self, raise_error: bool) -> Result<HttpResponse, HttpError> {
        HttpSession::shared_for(self.url.host()).send(self, raise_error)
    }
}
