//! The [`HttpVersion`] enum — selecting and reporting the HTTP protocol version.

use std::fmt;

use crate::error::HttpError;

/// An HTTP protocol version, used both to **pin** the version a request speaks
/// (per request via
/// [`HttpRequest::with_http_version`](crate::HttpRequest::with_http_version), or
/// per session via
/// [`HttpSession::with_http_version`](crate::HttpSession::with_http_version)) and to
/// **report** the version a response was actually delivered over
/// ([`HttpResponse::negotiated_version`](crate::HttpResponse::negotiated_version)).
///
/// [`Auto`](HttpVersion::Auto) (the default) negotiates the highest mutually
/// supported version through TLS ALPN; an explicit variant pins that version and
/// errors (rather than silently downgrading) when its transport is not
/// [`available`](HttpVersion::is_available).
///
/// Only HTTP/1.1 has a wired transport today: [`Http2`](HttpVersion::Http2) and
/// [`Http3`](HttpVersion::Http3) parse and name themselves (so selecting and
/// reporting them is API-stable) but report **unavailable** until their async
/// transports land — the same "parses and names itself, reports unsupported until
/// its backend is built" shape the compression codecs use.
///
/// ```
/// use yggdryl_http::HttpVersion;
/// assert_eq!(HttpVersion::from_str("h2").unwrap(), HttpVersion::Http2);
/// assert_eq!(HttpVersion::Http2.alpn(), Some("h2"));
/// assert_eq!(HttpVersion::Http11.as_str(), "HTTP/1.1");
/// assert!(HttpVersion::Http11.is_available()); // always wired
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HttpVersion {
    /// Negotiate the highest mutually supported version through ALPN (the default).
    #[default]
    Auto,
    /// HTTP/1.1 — the only version with a wired transport today.
    Http11,
    /// HTTP/2 (ALPN `h2`) — recognised but not yet implemented.
    Http2,
    /// HTTP/3 over QUIC (ALPN `h3`) — recognised but not yet implemented.
    Http3,
}

impl HttpVersion {
    /// Parses a version selector (case-insensitive): `auto` / `negotiate` (or an
    /// empty string) → [`Auto`](HttpVersion::Auto); `1` / `1.0` / `1.1` /
    /// `http/1.1` / `h1` → [`Http11`](HttpVersion::Http11) (HTTP/1.0 folds into
    /// HTTP/1.1, having no separate transport); `2` / `2.0` / `http/2` / `h2` →
    /// [`Http2`](HttpVersion::Http2); `3` / `3.0` / `http/3` / `h3` →
    /// [`Http3`](HttpVersion::Http3). An unknown selector is an
    /// [`HttpError::InvalidHeader`] naming the accepted values.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<HttpVersion, HttpError> {
        let version = match value.trim().to_ascii_lowercase().as_str() {
            "" | "auto" | "negotiate" => HttpVersion::Auto,
            "1" | "1.0" | "1.1" | "h1" | "http/1.0" | "http/1.1" | "http11" => HttpVersion::Http11,
            "2" | "2.0" | "h2" | "http/2" | "http2" => HttpVersion::Http2,
            "3" | "3.0" | "h3" | "http/3" | "http3" => HttpVersion::Http3,
            other => {
                return Err(HttpError::InvalidHeader(format!(
                    "unknown http version {other:?}; expected auto, 1.1, 2 or 3"
                )))
            }
        };
        Ok(version)
    }

    /// The canonical display name: `"auto"`, `"HTTP/1.1"`, `"HTTP/2"`, `"HTTP/3"`.
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpVersion::Auto => "auto",
            HttpVersion::Http11 => "HTTP/1.1",
            HttpVersion::Http2 => "HTTP/2",
            HttpVersion::Http3 => "HTTP/3",
        }
    }

    /// The TLS ALPN protocol id offered/selected for this version (`http/1.1` /
    /// `h2` / `h3`), or `None` for [`Auto`](HttpVersion::Auto) (which offers the
    /// full set and lets the server pick, rather than naming a single id).
    pub fn alpn(&self) -> Option<&'static str> {
        match self {
            HttpVersion::Auto => None,
            HttpVersion::Http11 => Some("http/1.1"),
            HttpVersion::Http2 => Some("h2"),
            HttpVersion::Http3 => Some("h3"),
        }
    }

    /// The [`HttpVersion`] for a TLS ALPN protocol id (`http/1.1` / `h2` / `h3`,
    /// case-insensitive), or `None` for an unrecognised id — the inverse of
    /// [`alpn`](HttpVersion::alpn), used to read back the negotiated protocol.
    pub fn from_alpn(id: &str) -> Option<HttpVersion> {
        match id.trim().to_ascii_lowercase().as_str() {
            "http/1.0" | "http/1.1" => Some(HttpVersion::Http11),
            "h2" => Some(HttpVersion::Http2),
            "h3" => Some(HttpVersion::Http3),
            _ => None,
        }
    }

    /// Whether a transport for this version is wired into this build.
    /// [`Auto`](HttpVersion::Auto) and [`Http11`](HttpVersion::Http11) are always
    /// available; [`Http2`](HttpVersion::Http2) needs the `http2` feature (its
    /// hyper/tokio transport) and [`Http3`](HttpVersion::Http3) the `http3` feature
    /// (its quinn/h3 QUIC transport). Pinning an unavailable version errors at
    /// dispatch rather than silently downgrading — check this first to choose ahead.
    pub fn is_available(&self) -> bool {
        match self {
            HttpVersion::Auto | HttpVersion::Http11 => true,
            HttpVersion::Http2 => cfg!(feature = "http2"),
            HttpVersion::Http3 => cfg!(feature = "http3"),
        }
    }
}

impl fmt::Display for HttpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
