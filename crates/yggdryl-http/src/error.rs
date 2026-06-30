//! [`HttpError`] — the error type for all HTTP client operations.

use std::fmt;

use crate::version::HttpVersion;

/// Failure during an HTTP operation.
///
/// All variants carry a description of what went wrong and, where the fix is
/// knowable, how to address it (e.g. missing feature, bad URL format).
#[derive(Debug)]
#[non_exhaustive]
pub enum HttpError {
    /// A network-level failure (connection refused, DNS, TLS handshake, reset,
    /// unexpected EOF, timeout at the transport layer).
    Transport(String),
    /// The server returned a 4xx or 5xx response and `raise_for_status` was
    /// called.
    Status {
        /// The HTTP status code.
        status: u16,
        /// The request URL.
        url: String,
        /// The response body, if it could be read (truncated to 1 KiB).
        body: String,
    },
    /// A redirect chain exceeded the configured limit.
    Redirect {
        /// The configured redirect limit that was exceeded.
        limit: usize,
    },
    /// The connection or read timed out.
    Timeout,
    /// TLS configuration failed (invalid CA certificate, hostname mismatch, …).
    Tls(String),
    /// The URL string could not be parsed or is missing a required component.
    InvalidUrl(String),
    /// The requested HTTP version is not compiled in; enable the named cargo
    /// feature to use it.
    VersionUnavailable {
        /// The version that was requested.
        version: HttpVersion,
        /// The cargo feature that enables support for it.
        feature: &'static str,
    },
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Transport(msg) => write!(f, "HTTP transport error: {msg}"),
            HttpError::Status { status, url, body } => {
                write!(f, "HTTP {status} from {url}")?;
                if !body.is_empty() {
                    write!(f, ": {body}")?;
                }
                Ok(())
            }
            HttpError::Redirect { limit } => {
                write!(f, "exceeded redirect limit ({limit})")
            }
            HttpError::Timeout => write!(f, "request timed out"),
            HttpError::Tls(msg) => write!(f, "TLS error: {msg}"),
            HttpError::InvalidUrl(msg) => write!(f, "invalid URL: {msg}"),
            HttpError::VersionUnavailable { version, feature } => write!(
                f,
                "HTTP version {version} is not available; enable the `{feature}` cargo feature"
            ),
        }
    }
}

impl std::error::Error for HttpError {}
