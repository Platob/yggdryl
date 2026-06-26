//! The [`HttpResponse`] — it holds all the logic to read its body from the server.

use yggdryl_core::Url;
use yggdryl_core::{BytesIO, Io};

use crate::error::HttpError;
use crate::headers::HttpHeaders;
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
pub struct HttpResponse {
    status: u16,
    url: Url,
    headers: HttpHeaders,
    body: Box<dyn Io>,
    sent_at: f64,
    received_at: Instant,
    negotiated: HttpVersion,
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
        }
    }

    /// Overwrites the dispatch timestamp — used so a redirected response reports
    /// the **first** hop's `sent_at`, not the final hop's.
    pub(crate) fn set_sent_at(&mut self, sent_at: f64) {
        self.sent_at = sent_at;
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

    /// The response media type, inferred from its `Content-Type`. Only present
    /// under the `media` feature.
    #[cfg(feature = "media")]
    pub fn mime_type(&self) -> Option<yggdryl_core::MimeType> {
        self.content_type()
            .and_then(|content_type| yggdryl_core::MimeType::from_str(content_type).ok())
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

    /// Drains the body and decodes it as UTF-8 text.
    pub fn text(self) -> Result<String, HttpError> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes).map_err(|err| HttpError::Decode(err.to_string()))
    }

    /// Drains the body into an in-memory [`BytesIO`] handle — a seekable
    /// [`Io`] over the (decompressed) response.
    pub fn into_bytesio(self) -> Result<BytesIO, HttpError> {
        Ok(BytesIO::from_bytes(self.bytes()?))
    }
}
