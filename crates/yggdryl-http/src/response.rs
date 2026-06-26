//! The [`HttpResponse`] — it holds all the logic to read its body from the server.

use std::sync::Arc;

use yggdryl_core::Url;
use yggdryl_core::{BytesIO, Io, IoError, IoStats, Whence};

use crate::error::HttpError;
use crate::headers::HttpHeaders;
use crate::request::HttpRequest;
use crate::session::HttpSession;
use crate::time::Instant;
use crate::version::HttpVersion;

/// A received HTTP response, modelled on `requests.Response`. It **holds the logic
/// to read its body from the server**: the body is a [`Box<dyn Io>`](Io) — a live
/// [`HttpStream`](crate::HttpStream) when streamed, or an in-memory
/// [`BytesIO`](yggdryl_core::BytesIO) when buffered — read lazily through
/// [`reader`](HttpResponse::reader) and drained by [`bytes`](HttpResponse::bytes)
/// / [`text`](HttpResponse::text) / [`into_bytesio`](HttpResponse::into_bytesio),
/// or taken whole with [`into_io`](HttpResponse::into_io) for seekable access.
///
/// It also carries timing: [`sent_at`](HttpResponse::sent_at) (when the request
/// was dispatched) and [`received_at`](HttpResponse::received_at) (when the
/// connection finished — the body reaching EOF or being closed), both UTC
/// Unix-epoch seconds, `0.0` until set.
///
/// It **holds the [`request`](HttpResponse::request) that produced it** (the
/// originating prepared request, similar to `requests.Response.request`), so the
/// method / URL / headers can always be read back. A verb called with `send =
/// false` returns an **unsent** response — [`is_sent`](HttpResponse::is_sent) is
/// `false`, the status is `0` and the body empty — that carries only the prepared
/// request; dispatch it later with [`send`](HttpResponse::send).
///
/// It is itself an [`Io`] handle (delegating to its body, the live
/// [`HttpStream`](crate::HttpStream)), so a response reads/seeks like any other
/// byte source — the `http`/`https` [`Io`](yggdryl_core::Io) factory hands one back
/// directly. It also carries the **shared per-host [`session`](HttpResponse::session)**
/// it belongs to (a [`shared_for`](HttpSession::shared_for) singleton, never a
/// copy), used by [`send`](HttpResponse::send) to re-dispatch its request.
pub struct HttpResponse {
    status: u16,
    url: Url,
    headers: HttpHeaders,
    body: Box<dyn Io>,
    sent_at: f64,
    received_at: Instant,
    negotiated: HttpVersion,
    /// The request that produced this response (the prepared request, before any
    /// transport-level headers). `None` only for a bare response built without one.
    request: Option<HttpRequest>,
    /// The shared per-host [`HttpSession`] this response belongs to (its
    /// [`shared_for`](HttpSession::shared_for) singleton). `None` until attached, in
    /// which case [`session`](HttpResponse::session) resolves it from the URL host.
    session: Option<Arc<HttpSession>>,
}

impl HttpResponse {
    /// Assembles a response from its status, URL, response headers, the body
    /// [`Io`] (a live [`HttpStream`](crate::HttpStream) or a buffered
    /// [`BytesIO`](yggdryl_core::BytesIO)), its timing (`sent_at` when the request
    /// was dispatched, `received_at` shared with the body's stream) and the
    /// `negotiated` protocol version the hop was delivered over.
    pub(crate) fn new(
        status: u16,
        url: Url,
        headers: HttpHeaders,
        body: Box<dyn Io>,
        sent_at: f64,
        received_at: Instant,
        negotiated: HttpVersion,
    ) -> HttpResponse {
        HttpResponse {
            status,
            url,
            headers,
            body,
            sent_at,
            received_at,
            negotiated,
            request: None,
            session: None,
        }
    }

    /// Builds an **unsent** response: a placeholder that carries only the prepared
    /// `request` (it has not been dispatched). The status is `0`, the body empty
    /// and [`is_sent`](HttpResponse::is_sent) `false`. Returned by a verb called
    /// with `send = false` so the caller can inspect the request via
    /// [`request`](HttpResponse::request) and dispatch it later with
    /// [`send`](HttpResponse::send).
    pub(crate) fn unsent(request: HttpRequest) -> HttpResponse {
        let url = request.url().clone();
        let negotiated = request.http_version().unwrap_or(HttpVersion::Auto);
        let session = HttpSession::shared_for(url.host());
        HttpResponse {
            status: 0,
            url,
            headers: HttpHeaders::new(),
            body: Box::new(BytesIO::from_bytes(Vec::new())),
            sent_at: 0.0,
            received_at: Instant::new(),
            negotiated,
            request: Some(request),
            session: Some(session),
        }
    }

    /// Overwrites the dispatch timestamp — used so a redirected response reports
    /// the **first** hop's `sent_at`, not the final hop's.
    pub(crate) fn set_sent_at(&mut self, sent_at: f64) {
        self.sent_at = sent_at;
    }

    /// Attaches the originating `request` (a copy of the prepared request), so the
    /// response can report what produced it. Set by [`HttpSession::send`].
    pub(crate) fn set_request(&mut self, request: HttpRequest) {
        self.request = Some(request);
    }

    /// Attaches the shared per-host [`session`](HttpResponse::session) this response
    /// belongs to. Set by [`HttpSession::send`] from the request's host.
    pub(crate) fn set_session(&mut self, session: Arc<HttpSession>) {
        self.session = Some(session);
    }

    /// The shared per-host [`HttpSession`] this response belongs to — the
    /// [`shared_for`](HttpSession::shared_for) singleton for the response URL's host
    /// (never a per-response copy). [`send`](HttpResponse::send) re-dispatches
    /// through it.
    pub fn session(&self) -> Arc<HttpSession> {
        self.session
            .clone()
            .unwrap_or_else(|| HttpSession::shared_for(self.url.host()))
    }

    /// Consumes the response, returning its body as a [`Box<dyn Io>`](Io) — the
    /// live [`HttpStream`](crate::HttpStream) — for seekable access.
    pub fn into_io(self) -> Box<dyn Io> {
        self.body
    }

    /// A mutable handle to the **raw** body [`Io`] (the live
    /// [`HttpStream`](crate::HttpStream)), to read or seek it in place without
    /// consuming the response. This is the undecoded body; for transparent
    /// `Content-Encoding` decoding use [`reader`](HttpResponse::reader) /
    /// [`bytes`](HttpResponse::bytes), which consume the response.
    pub fn body_mut(&mut self) -> &mut dyn Io {
        &mut *self.body
    }

    /// The HTTP status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Whether the response is a success: it was actually dispatched
    /// ([`is_sent`](HttpResponse::is_sent)) **and** its status is below 400 (the
    /// `requests` definition of "ok"). An **unsent** placeholder (status `0`) is
    /// therefore *not* ok, so `if response.ok()` never treats a never-sent response
    /// as a 2xx success.
    pub fn ok(&self) -> bool {
        self.is_sent() && self.status < 400
    }

    /// Whether this response was actually dispatched. `false` for the **unsent**
    /// placeholder a verb returns with `send = false` (status `0`, empty body),
    /// which carries only the prepared [`request`](HttpResponse::request).
    pub fn is_sent(&self) -> bool {
        self.status != 0
    }

    /// The request that produced this response — the **prepared, originating**
    /// request (method, URL, headers and settings), similar to
    /// `requests.Response.request`. After a redirect chain it is the *original*
    /// request that entered the chain (not the final hop), so its method/URL may
    /// differ from [`url`](HttpResponse::url). `None` only for a bare response built
    /// without one.
    pub fn request(&self) -> Option<&HttpRequest> {
        self.request.as_ref()
    }

    /// Consumes the response, returning the originating [`request`](HttpResponse::request).
    pub fn into_request(self) -> Option<HttpRequest> {
        self.request
    }

    /// Dispatches this response's [`request`](HttpResponse::request) through its
    /// attached shared per-host [`session`](HttpResponse::session) and returns the
    /// resulting response. This is how an **unsent** response (one returned by a verb
    /// with `send = false`) is sent later; on an already-sent response it replays the
    /// originating request. `raise_error` turns a 4xx/5xx status into an
    /// [`HttpError::Status`]. Errors with [`HttpError::Unsupported`] if the response
    /// carries no request. (Replaying an *already-sent* request whose body was a
    /// one-shot stream re-sends it with an empty body, since a streamed body cannot
    /// be duplicated — see [`HttpRequest::copy`](crate::HttpRequest::copy); an unsent
    /// response keeps its full body.)
    pub fn send(self, raise_error: bool) -> Result<HttpResponse, HttpError> {
        let session = self.session();
        match self.request {
            Some(request) => session.send(request, raise_error),
            None => Err(HttpError::Unsupported(
                "this response carries no request to send".into(),
            )),
        }
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

    /// The HTTP protocol [`version`](HttpVersion) the response was actually
    /// delivered over — the result of negotiating the request's pinned version (or
    /// the session default) against the wired transports. Today this is always
    /// [`Http11`](HttpVersion::Http11); it will reflect a negotiated
    /// [`Http2`](HttpVersion::Http2) / [`Http3`](HttpVersion::Http3) once those
    /// transports land.
    pub fn negotiated_version(&self) -> HttpVersion {
        self.negotiated
    }

    /// When the request was dispatched (the response headers came back), as UTC
    /// Unix-epoch seconds; `0.0` if unset.
    pub fn sent_at(&self) -> f64 {
        self.sent_at
    }

    /// When the connection finished — the body reached EOF or was closed — as UTC
    /// Unix-epoch seconds; `0.0` until the body has been fully read or closed.
    pub fn received_at(&self) -> f64 {
        self.received_at.get()
    }

    /// The response headers (case-insensitive).
    pub fn headers(&self) -> &HttpHeaders {
        &self.headers
    }

    /// Looks up a header by name (case-insensitive), returning its first value.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)
    }

    /// The `Content-Type` header, if present.
    pub fn content_type(&self) -> Option<&str> {
        self.headers.get("content-type")
    }

    /// The `Content-Length` header parsed as a number, if present and valid.
    pub fn content_length(&self) -> Option<u64> {
        self.headers.get_u64("content-length")
    }

    /// The `Content-Encoding` header, if present.
    pub fn content_encoding(&self) -> Option<&str> {
        self.headers.get("content-encoding")
    }

    /// The response's single MIME type, inferred from its `Content-Type`. Only
    /// present under the `media` feature.
    #[cfg(feature = "media")]
    pub fn mime_type(&self) -> Option<yggdryl_core::MimeType> {
        self.content_type()
            .and_then(|content_type| yggdryl_core::MimeType::from_str(content_type).ok())
    }

    /// The response's layered [`MediaType`](yggdryl_core::MediaType) stack,
    /// **combining `Content-Type` with `Content-Encoding`**: the content MIME is the
    /// inner type and the transfer encoding (`gzip` / `zstd` / `br`) the outer layer,
    /// so a gzipped CSV (`Content-Type: text/csv`, `Content-Encoding: gzip`) reads as
    /// `[Csv, Gzip]` — exactly like the media type of a `data.csv.gz` path. Only
    /// present under the `media` feature.
    #[cfg(feature = "media")]
    pub fn media_type(&self) -> Option<yggdryl_core::MediaType> {
        let mut types: Vec<yggdryl_core::MimeType> = self.mime_type().into_iter().collect();
        if let Some(encoding) = self.content_encoding() {
            if let Some(mime) = yggdryl_core::Compression::from_str(encoding)
                .ok()
                .and_then(|codec| codec.mime())
            {
                types.push(mime);
            }
        }
        (!types.is_empty()).then(|| yggdryl_core::MediaType::new(types))
    }

    /// The [`Compression`](yggdryl_core::Compression) codec named by the response's
    /// `Content-Encoding` header (`gzip` / `zstd` / `snappy` / `br`), or `None` when
    /// the header is absent or `identity`. This is the codec
    /// [`reader`](HttpResponse::reader) / [`bytes`](HttpResponse::bytes) /
    /// [`text`](HttpResponse::text) / [`json`](HttpResponse::json) transparently
    /// decode. Only present under the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn compression(&self) -> Option<yggdryl_core::Compression> {
        self.content_encoding()
            .and_then(|encoding| yggdryl_core::Compression::from_str(encoding).ok())
            .filter(|codec| *codec != yggdryl_core::Compression::None)
    }

    /// Consumes the response and returns its body as a streaming [`Io`] source.
    /// Under the `compression` feature a `Content-Encoding` of gzip / zstd / snappy
    /// is decoded transparently.
    pub fn reader(self) -> Box<dyn Io> {
        let HttpResponse { headers, body, .. } = self;

        #[cfg(feature = "compression")]
        {
            // Resolve the codec before touching `body`, so the fall-through to the
            // undecoded body never trips over a moved value.
            let codec = headers
                .get("content-encoding")
                .and_then(|encoding| yggdryl_core::Compression::from_str(encoding).ok())
                .filter(|codec| *codec != yggdryl_core::Compression::None && codec.is_available());
            if let Some(codec) = codec {
                log_event!(debug, "HttpResponse::reader decoding {codec}");
                return match codec.decoder(body) {
                    Ok(decoder) => Box::new(decoder),
                    Err(err) => Box::new(crate::bridge::ErrBody(Some(err))),
                };
            }
        }
        #[cfg(not(feature = "compression"))]
        let _ = &headers;
        body
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

    /// Drains the (decoded) body into a `Vec<u8>` **and** returns the
    /// [`received_at`](HttpResponse::received_at) timestamp stamped as the body
    /// reached EOF — captured in one move, since draining consumes the response.
    /// The buffering bindings use this so the finish time reflects the full read.
    pub fn read_all(self) -> Result<(Vec<u8>, f64), HttpError> {
        let received_at = self.received_at.clone();
        let bytes = self.bytes()?;
        Ok((bytes, received_at.get()))
    }

    /// Drains the body and decodes it as UTF-8 text. The body is **transparently
    /// decompressed** first (under the `compression` feature) per its
    /// `Content-Encoding`, so the text is always the decoded payload.
    pub fn text(self) -> Result<String, HttpError> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes).map_err(|err| HttpError::Decode(err.to_string()))
    }

    /// Drains the body and parses it as JSON, returning a
    /// [`serde_json::Value`](yggdryl_core::serde_json::Value). The body is
    /// **transparently decompressed** first (under the `compression` feature) per
    /// its `Content-Encoding`, and parsed in Rust off the decoded stream — so the
    /// bytes are read once and never crossed back across an FFI boundary.
    pub fn json(self) -> Result<yggdryl_core::serde_json::Value, HttpError> {
        self.reader()
            .json()
            .map_err(|err| HttpError::Decode(err.to_string()))
    }

    /// Drains the body into an in-memory [`BytesIO`] handle — a seekable
    /// [`Io`] over the (decompressed) response.
    pub fn into_bytesio(self) -> Result<BytesIO, HttpError> {
        Ok(BytesIO::from_bytes(self.bytes()?))
    }
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("url", &self.url.to_string())
            .field("negotiated", &self.negotiated)
            .finish_non_exhaustive()
    }
}

/// A response **is** an [`Io`] handle: every byte operation delegates to its body
/// (the live [`HttpStream`](crate::HttpStream) or buffered
/// [`BytesIO`](yggdryl_core::BytesIO)), so a returned response reads, seeks and
/// `pread`s like any other source — the raw, undecoded body (use
/// [`reader`](HttpResponse::reader) for transparent `Content-Encoding` decoding).
impl Io for HttpResponse {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        self.body.stats()
    }

    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        self.body.seek(offset, whence)
    }

    fn stream_position(&self) -> u64 {
        self.body.stream_position()
    }

    fn stream_len(&self) -> Option<u64> {
        self.body.stream_len()
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        self.body.read(buf)
    }

    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        self.body.read_to_end(out)
    }

    fn pread(&mut self, buf: &mut [u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        self.body.pread(buf, offset, whence)
    }

    fn flush(&mut self) -> Result<(), IoError> {
        self.body.flush()
    }

    fn close(&mut self) -> Result<(), IoError> {
        self.body.close()
    }

    fn as_slice(&self) -> Option<&[u8]> {
        self.body.as_slice()
    }

    #[cfg(feature = "media")]
    fn media_type(&self) -> Option<yggdryl_core::MediaType> {
        self.body.media_type()
    }
}
