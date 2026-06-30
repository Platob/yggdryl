//! [`HttpResponse`] — the HTTP response.

use std::io::Read;

use crate::body::ResponseBody;
use crate::error::HttpError;
use crate::version::HttpVersion;

/// An HTTP response with lazy body access.
///
/// The body is a lazy stream: call [`bytes`](HttpResponse::bytes),
/// [`text`](HttpResponse::text), or [`reader`](HttpResponse::reader)
/// to drain it. Each draining method consumes `self` so the body can only
/// be read once; call [`into_bytes`](HttpResponse::bytes) early if you need
/// random access.
///
/// ```no_run
/// # fn main() -> Result<(), yggdryl_http::HttpError> {
/// use yggdryl_http::{HttpSession, HttpRequest};
///
/// let session = HttpSession::new();
/// let resp = session.get("https://example.com")?;
/// assert!(resp.ok());
/// let body = resp.text()?;
/// # Ok(())
/// # }
/// ```
pub struct HttpResponse {
    /// HTTP status code (e.g. `200`, `404`).
    pub status: u16,
    headers: Vec<(String, String)>,
    /// The negotiated protocol version.
    pub version: HttpVersion,
    content_length: Option<u64>,
    body: Option<ResponseBody>,
}

impl HttpResponse {
    /// Constructs an `HttpResponse` from the raw transport fields.
    pub(crate) fn new(
        status: u16,
        headers: Vec<(String, String)>,
        version: HttpVersion,
        content_length: Option<u64>,
        body: ResponseBody,
    ) -> Self {
        HttpResponse {
            status,
            headers,
            version,
            content_length,
            body: Some(body),
        }
    }

    /// Whether the status indicates success (`100–399`).
    pub fn ok(&self) -> bool {
        self.status < 400
    }

    /// Returns `Err(HttpError::Status { … })` for 4xx/5xx responses, `Ok(&self)` otherwise.
    pub fn raise_for_status(&self) -> Result<&Self, HttpError> {
        if self.status >= 400 {
            let url = self
                .header("x-request-url")
                .unwrap_or("(unknown)")
                .to_string();
            Err(HttpError::Status {
                status: self.status,
                url,
                body: String::new(),
            })
        } else {
            Ok(self)
        }
    }

    /// All response headers (name + value pairs).
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The first header value for `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase() == name_lower)
            .map(|(_, v)| v.as_str())
    }

    /// The `Content-Type` header value, if present.
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// The `Content-Length` in bytes, if the server reported it.
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    /// The negotiated protocol version (e.g. `HttpVersion::Http2`).
    pub fn version(&self) -> HttpVersion {
        self.version
    }

    /// Under the `media` feature: the MIME type parsed from `Content-Type`.
    ///
    /// Returns the type/subtype portion (e.g. `"application/json"`), stripping
    /// parameters such as `; charset=utf-8`.
    #[cfg(feature = "media")]
    pub fn mime_type(&self) -> Option<&str> {
        self.content_type().and_then(|ct| {
            // "text/html; charset=utf-8" → "text/html"
            let mime = ct.split(';').next()?.trim();
            if mime.is_empty() {
                None
            } else {
                Some(mime)
            }
        })
    }

    /// Consumes the response and drains the body into raw bytes.
    pub fn bytes(mut self) -> Result<Vec<u8>, HttpError> {
        let body = self
            .body
            .take()
            .unwrap_or_else(|| ResponseBody::new(std::io::empty()));
        body.drain_bytes()
            .map_err(|e| HttpError::Transport(e.to_string()))
    }

    /// Consumes the response and drains the body as a UTF-8 string.
    pub fn text(mut self) -> Result<String, HttpError> {
        let body = self
            .body
            .take()
            .unwrap_or_else(|| ResponseBody::new(std::io::empty()));
        body.drain_text()
            .map_err(|e| HttpError::Transport(e.to_string()))
    }

    /// Consumes the response, returning a [`Read`] over the raw body stream.
    pub fn reader(mut self) -> Box<dyn Read + Send + 'static> {
        match self.body.take() {
            Some(b) => Box::new(b),
            None => Box::new(std::io::empty()),
        }
    }
}

impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("version", &self.version)
            .field("content_length", &self.content_length)
            .finish_non_exhaustive()
    }
}
