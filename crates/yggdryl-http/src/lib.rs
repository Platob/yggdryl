//! # yggdryl-http
//!
//! A small **blocking HTTP client** for the yggdryl project, shaped after
//! Python's `requests`: a connection-pooling [`HttpSession`] with verb helpers
//! ([`get`](HttpSession::get) / [`post`](HttpSession::post) / …), a builder
//! [`HttpRequest`], and an [`HttpResponse`] whose body **streams over the
//! [`yggdryl-core`](yggdryl_core) byte-IO abstraction** rather than being eagerly
//! buffered.
//!
//! A response holds all the logic to read its body from the server: it is a
//! [`Box<dyn Io>`](yggdryl_core::Io) — a live [`HttpStream`] when streamed, or an
//! in-memory [`BytesIO`](yggdryl_core::BytesIO) when buffered — read lazily through
//! [`reader`](HttpResponse::reader) (decoded under `compression`), and drained by
//! [`bytes`](HttpResponse::bytes) / [`text`](HttpResponse::text) /
//! [`into_bytesio`](HttpResponse::into_bytesio) / [`into_io`](HttpResponse::into_io).
//! A **request** body can likewise stream straight from any `Io` handle via
//! [`with_body_io`](HttpRequest::with_body_io) — uploading a
//! [`LocalPath`](yggdryl_core::LocalPath) never loads the file into memory.
//!
//! For random access there is [`HttpStream`], a seekable [`Io`](yggdryl_core::Io)
//! that **streams off a held connection** — sequential reads pull straight off the
//! socket, keeping only a sliding 4 MiB cache for short seek-backs, while a
//! pread / seek-back / forward jump reopens a `Range` request on a pooled
//! connection. It retries transient failures and **resumes from the cursor** after
//! a dropped connection, releasing the connection on EOF (or
//! [`close`](yggdryl_core::Io::close)). [`HttpSession::send_many`] runs an iterator
//! of requests concurrently in batches.
//!
//! Header logic is centralised in the case-insensitive [`HttpHeaders`] type, which
//! every request, response and stream carries.
//!
//! The HTTP protocol version is tunable through [`HttpVersion`]: pin one per
//! session ([`HttpSession::with_http_version`]) or per request
//! ([`HttpRequest::with_http_version`]), and read the one a response was delivered
//! over via [`HttpResponse::negotiated_version`]. [`HttpVersion::Auto`] (the
//! default) negotiates the best available. HTTP/1.1 is always wired (the blocking
//! `ureq` transport); the `http2` feature adds a real HTTP/2 transport (so `Auto`
//! negotiates `h2` over TLS ALPN) and the `http3` feature a real HTTP/3-over-QUIC
//! transport. A pinned version whose feature is off errors with
//! [`HttpError::Unsupported`] rather than downgrade silently.
//!
//! ```no_run
//! use yggdryl_http::{HttpSession, HttpRequest};
//!
//! let session = HttpSession::new().with_user_agent("yggdryl-http/0.1");
//! // Verbs raise on a 4xx/5xx by default; pass `false` to keep the response.
//! let body = session.get("https://example.com", true).unwrap().text().unwrap();
//!
//! // A seekable, lazily-fetched remote Io (stream = true keeps the live connection).
//! use yggdryl_core::{Io, Whence};
//! let request = HttpRequest::get("https://example.com/data").unwrap();
//! let mut stream = session.send(request, false).unwrap().into_io();
//! let mut footer = [0u8; 8];
//! stream.pread(&mut footer, -8, Whence::End).unwrap(); // read the tail, one range request
//! ```
//!
//! ## Optional features (off by default)
//!
//! - `compression` — transparently decode a `Content-Encoding` (gzip / zstd /
//!   snappy) response body through `yggdryl-compression`, the way `requests`
//!   auto-decompresses.
//! - `media` — expose the response's [`mime_type`](HttpResponse::mime_type) and
//!   [`HttpStream`]'s media type.
//! - `serde` — `Serialize`/`Deserialize` for the data types ([`Method`],
//!   [`HttpVersion`], [`HttpHeaders`], [`Cookie`], [`HttpCookies`],
//!   [`RetryConfig`]) and, transitively, the core value types; a live
//!   request/response body is deliberately not serialisable.
//! - `http2` — the optional async **HTTP/2** transport (hyper + a small
//!   multi-threaded tokio runtime + tokio-rustls). With it on,
//!   [`HttpVersion::Http2`] speaks h2 (TLS ALPN for `https`, h2c for cleartext) and
//!   [`HttpVersion::Auto`] negotiates `h2`/`http/1.1` over TLS; off, those error.
//! - `http3` — the optional async **HTTP/3** transport (quinn + h3 over QUIC). With
//!   it on, [`HttpVersion::Http3`] speaks h3 over `https` (TLS ALPN `h3`); off, it
//!   errors. Shares the tokio runtime and TLS stack with `http2`.
//! - `log` — structured `log` events on the request path.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate pulls no `log` dependency by default).
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

#[cfg(any(feature = "http2", feature = "http3"))]
mod async_body;
mod auth;
mod bridge;
mod cookies;
mod error;
mod factory;
mod headers;
mod method;
mod redirect;
mod request;
mod response;
mod retry;
mod session;
mod stream;
mod time;
#[cfg(any(feature = "http2", feature = "http3"))]
mod transport;
mod version;

pub use cookies::{Cookie, HttpCookies};
pub use error::HttpError;
pub use factory::register;
pub use headers::HttpHeaders;
pub use method::Method;
pub use request::HttpRequest;
pub use response::HttpResponse;
pub use retry::RetryConfig;
pub use session::{
    delete, get, head, patch, post, put, request, HttpResponseBatch, HttpSession, SendMany,
};
pub use stream::HttpStream;
pub use version::HttpVersion;

#[cfg(test)]
mod tests;
