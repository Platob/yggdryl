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
//! ```no_run
//! use yggdryl_http::{HttpSession, HttpRequest};
//!
//! let session = HttpSession::new().with_user_agent("yggdryl-http/0.1");
//! // Verbs raise on a 4xx/5xx by default; pass `false` to keep the response.
//! let body = session.get("https://example.com").unwrap().text().unwrap();
//!
//! // A seekable, lazily-fetched remote Io (stream = true keeps the live connection).
//! use yggdryl_core::{Io, Whence};
//! let request = HttpRequest::get("https://example.com/data").unwrap();
//! let mut stream = session.send(request, false, true, true).unwrap().into_io();
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
//! - `log` — structured `log` events on the request path.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate pulls no `log` dependency by default).
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

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

pub use cookies::{Cookie, HttpCookies};
pub use error::HttpError;
pub use factory::register;
pub use headers::HttpHeaders;
pub use method::Method;
pub use request::HttpRequest;
pub use response::HttpResponse;
pub use retry::RetryConfig;
pub use session::{HttpResponseBatch, HttpSession, SendMany};
pub use stream::HttpStream;

#[cfg(test)]
mod tests;
