//! The crate-wide [`HttpError`] type.

use std::fmt;

use yggdryl_core::IoError;

/// The error type for every [`HttpSession`](crate::HttpSession) /
/// [`HttpRequest`](crate::HttpRequest) / [`HttpResponse`](crate::HttpResponse)
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
    /// [`raise_for_status`](crate::HttpResponse::raise_for_status) saw a 4xx/5xx code.
    Status(u16),
    /// The body could not be decoded (e.g. invalid UTF-8 for
    /// [`text`](crate::HttpResponse::text)).
    Decode(String),
    /// An underlying byte-IO error while streaming the body.
    Io(IoError),
    /// More than the session's `max_redirects` 3xx hops were followed (carries the
    /// limit), or a redirect chain looped back to a `(method, url)` already visited
    /// (carries the repeated URL).
    TooManyRedirects(String),
    /// A requested capability has no implementation in this build — e.g. an
    /// [`HttpVersion`](crate::HttpVersion) was pinned whose transport is not yet
    /// wired. The message names the unavailable feature and the alternative.
    Unsupported(String),
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
            HttpError::TooManyRedirects(what) => write!(f, "too many redirects: {what}"),
            HttpError::Unsupported(what) => write!(f, "unsupported: {what}"),
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
        match &err {
            // A TLS / certificate failure gets an actionable hint: the usual cause
            // is a self-signed or internal certificate, fixable by trusting its CA
            // or (insecurely) turning verification off.
            ureq::Error::Tls(message) => HttpError::Transport(format!(
                "tls error: {message}; if this host uses a self-signed or internal \
                 certificate, install its CA or set verify=false (insecure) to skip \
                 verification"
            )),
            // A read/response timeout gets an actionable hint: the server was too
            // slow for the configured budget — raise it if the endpoint genuinely
            // needs longer (a big server-side computation, a slow generation).
            ureq::Error::Timeout(_) => HttpError::Transport(format!(
                "{err}; the server sent no data within the read timeout — raise it with \
                 HttpSession::with_read_timeout(seconds) if this endpoint needs longer"
            )),
            _ => HttpError::Transport(err.to_string()),
        }
    }
}

impl From<ureq::http::Error> for HttpError {
    fn from(err: ureq::http::Error) -> HttpError {
        HttpError::InvalidHeader(err.to_string())
    }
}
