//! [`HttpVersion`] — HTTP protocol version.

use std::fmt;

use crate::error::HttpError;

/// HTTP protocol version negotiated for a request or response.
///
/// [`Auto`](HttpVersion::Auto) performs ALPN negotiation (HTTP/3 → HTTP/2 →
/// HTTP/1.1) and is the default. Pin to a specific version to force it;
/// [`Http2`](HttpVersion::Http2) requires the `http2` feature and
/// [`Http3`](HttpVersion::Http3) requires `http3`.
///
/// ```
/// use yggdryl_http::HttpVersion;
///
/// assert_eq!(HttpVersion::Http1_1.to_str(), "http/1.1");
/// assert_eq!(HttpVersion::from_str("h2").unwrap(), HttpVersion::Http2);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HttpVersion {
    /// Negotiate the highest available version via TLS ALPN: HTTP/3 → HTTP/2
    /// → HTTP/1.1.
    #[default]
    Auto,
    /// HTTP/1.1 (always available; uses the bundled `ureq` transport).
    Http1_1,
    /// HTTP/2 (requires the `http2` cargo feature).
    Http2,
    /// HTTP/3 over QUIC (requires the `http3` cargo feature).
    Http3,
}

impl HttpVersion {
    /// The ALPN / protocol-name string for this version.
    ///
    /// `Auto` returns `"auto"` (not a real ALPN token — use for display only).
    pub fn to_str(&self) -> &str {
        match self {
            HttpVersion::Auto => "auto",
            HttpVersion::Http1_1 => "http/1.1",
            HttpVersion::Http2 => "h2",
            HttpVersion::Http3 => "h3",
        }
    }

    /// Parses a version string.
    ///
    /// Accepts (case-insensitive): `"auto"`, `"http/1.1"`, `"h1"`,
    /// `"h2"`, `"http/2"`, `"h3"`, `"http/3"`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self, HttpError> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(HttpVersion::Auto),
            "http/1.1" | "http1.1" | "h1" | "http1" => Ok(HttpVersion::Http1_1),
            "h2" | "http/2" | "http2" => Ok(HttpVersion::Http2),
            "h3" | "http/3" | "http3" => Ok(HttpVersion::Http3),
            other => Err(HttpError::InvalidUrl(format!(
                "unknown HTTP version {other:?}; \
                 expected one of: auto, http/1.1, h2, h3"
            ))),
        }
    }

    /// Returns the serialized bytes of the version string.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().as_bytes().to_vec()
    }

    /// Parses from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, HttpError> {
        let s = std::str::from_utf8(bytes)
            .map_err(|_| HttpError::InvalidUrl("version bytes are not valid UTF-8".into()))?;
        HttpVersion::from_str(s)
    }
}

impl fmt::Display for HttpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_str())
    }
}
