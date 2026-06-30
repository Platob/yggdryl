//! [`HttpRequest`] — the HTTP request builder.

use std::time::Duration;

use yggdryl_core::Io;

use crate::body::RequestBody;
use crate::error::HttpError;
use crate::method::Method;
use crate::version::HttpVersion;

/// A builder for an HTTP request.
///
/// Construct with a verb factory (`get`, `post`, …) or `new`, then chain
/// `with_*` methods to add headers, query parameters, a body, or per-request
/// overrides. Pass to [`HttpSession::request`](crate::HttpSession::request)
/// (or a verb helper) to send.
///
/// ```
/// use yggdryl_http::{HttpRequest, Method};
///
/// let req = HttpRequest::get("https://example.com").unwrap()
///     .with_header("accept", "application/json")
///     .with_param("page", "1");
/// assert_eq!(req.method, Method::Get);
/// ```
#[derive(Debug)]
pub struct HttpRequest {
    /// The HTTP method.
    pub method: Method,
    /// The request URL (without query string; see `params`).
    pub url: String,
    /// Request headers (per-request; merged with session defaults).
    pub headers: Vec<(String, String)>,
    /// Query string parameters (percent-encoded by the transport).
    pub params: Vec<(String, String)>,
    /// The request body, if any.
    pub body: Option<RequestBody>,
    /// Per-request protocol version override (falls back to session default).
    pub version: Option<HttpVersion>,
    /// Per-request timeout override (falls back to session default).
    pub timeout: Option<Duration>,
    /// Per-request redirect limit (falls back to session default).
    pub redirect_limit: Option<usize>,
}

impl HttpRequest {
    /// Creates a new `GET` request.
    pub fn get(url: &str) -> Result<Self, HttpError> {
        Self::new(Method::Get, url)
    }

    /// Creates a new `POST` request with no body (add one with `with_body`).
    pub fn post(url: &str) -> Result<Self, HttpError> {
        Self::new(Method::Post, url)
    }

    /// Creates a new `PUT` request with no body.
    pub fn put(url: &str) -> Result<Self, HttpError> {
        Self::new(Method::Put, url)
    }

    /// Creates a new `DELETE` request.
    pub fn delete(url: &str) -> Result<Self, HttpError> {
        Self::new(Method::Delete, url)
    }

    /// Creates a new `PATCH` request.
    pub fn patch(url: &str) -> Result<Self, HttpError> {
        Self::new(Method::Patch, url)
    }

    /// Creates a new `HEAD` request.
    pub fn head(url: &str) -> Result<Self, HttpError> {
        Self::new(Method::Head, url)
    }

    /// Creates a new `OPTIONS` request.
    pub fn options(url: &str) -> Result<Self, HttpError> {
        Self::new(Method::Options, url)
    }

    /// Creates a request with the given method and URL.
    ///
    /// Returns [`HttpError::InvalidUrl`] if `url` does not start with
    /// `http://` or `https://`.
    pub fn new(method: Method, url: &str) -> Result<Self, HttpError> {
        crate::log_event!(trace, "HttpRequest::new method={method} url={url}");
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(HttpError::InvalidUrl(format!(
                "{url:?} does not start with http:// or https://"
            )));
        }
        Ok(HttpRequest {
            method,
            url: url.to_string(),
            headers: Vec::new(),
            params: Vec::new(),
            body: None,
            version: None,
            timeout: None,
            redirect_limit: None,
        })
    }

    /// Appends a request header (duplicate names are allowed).
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Appends a query-string parameter (percent-encoded when the URL is built).
    pub fn with_param(mut self, key: &str, value: &str) -> Self {
        self.params.push((key.to_string(), value.to_string()));
        self
    }

    /// Sets the request body to `data` (raw bytes).
    pub fn with_body(mut self, data: &[u8]) -> Self {
        self.body = Some(RequestBody::Bytes(data.to_vec()));
        self
    }

    /// Sets the request body to `data` (owned bytes).
    pub fn with_body_bytes(mut self, data: Vec<u8>) -> Self {
        self.body = Some(RequestBody::Bytes(data));
        self
    }

    /// Sets the request body to a streaming `Io` handle.
    ///
    /// The `Content-Length` is set from [`Io::size`](yggdryl_core::Io::size)
    /// so the source is never fully buffered before sending.
    pub fn with_body_io(mut self, io: Box<dyn Io + Send + 'static>) -> Self {
        self.body = Some(RequestBody::Io(io));
        self
    }

    /// Forces a specific HTTP version for this request.
    pub fn with_version(mut self, version: HttpVersion) -> Self {
        self.version = Some(version);
        self
    }

    /// Sets a per-request timeout, overriding the session default.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets a per-request redirect limit, overriding the session default.
    pub fn with_redirect_limit(mut self, limit: usize) -> Self {
        self.redirect_limit = Some(limit);
        self
    }

    /// Returns the fully-qualified URL with any query parameters appended.
    pub fn url_with_params(&self) -> String {
        if self.params.is_empty() {
            return self.url.clone();
        }
        let mut url = self.url.clone();
        let sep = if url.contains('?') { '&' } else { '?' };
        url.push(sep);
        let encoded: Vec<String> = self
            .params
            .iter()
            .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
            .collect();
        url.push_str(&encoded.join("&"));
        url
    }
}

/// Percent-encodes a query-string key or value (RFC 3986 unreserved chars
/// are left unchanged; everything else is `%XX`-encoded).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(
                char::from_digit((byte >> 4) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
            out.push(
                char::from_digit((byte & 0x0f) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_urls() {
        assert!(HttpRequest::get("/path").is_err());
        assert!(HttpRequest::get("example.com").is_err());
    }

    #[test]
    fn builds_url_with_params() {
        let req = HttpRequest::get("https://example.com/q")
            .unwrap()
            .with_param("q", "hello world")
            .with_param("n", "5");
        let url = req.url_with_params();
        assert!(url.contains("q=hello%20world"), "got: {url}");
        assert!(url.contains("n=5"), "got: {url}");
    }
}
