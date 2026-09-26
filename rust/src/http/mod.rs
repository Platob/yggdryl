//! HTTP behind [`IOBase`](crate::IOBase): one synchronous HTTP/1.1 client, the
//! sessions, requests, responses and resumable streams built over it, and the
//! `message/http` medium.
//!
//! A resource an HTTP URL names is read and written through the same
//! [`IOBase`](crate::IOBase) verbs every other backend answers, and every one
//! of those verbs is a stated number of requests. The count is the cost model,
//! as it is for the object stores: one request is one round trip, and the
//! tests assert the number rather than the time.
//!
//! | operation | requests |
//! | --- | --- |
//! | building a session, a request, resolving a child | none |
//! | `pread`, `read_range_bytes`, `read_range_digest` | one ranged `GET` |
//! | `read_all_bytes`, `read_digest`, a `pstream_bytes` drain | one `GET`, plus one per resume |
//! | `size`, `mtime`, `kind` while closed | one `HEAD`; none while open, or once a read learned it |
//! | `write_all_bytes`, `clear` | one `PUT` |
//! | `pwrite` then `flush` | one `GET` and one `PUT` |
//! | `remove` | one `DELETE` |
//! | `send`, `stream` | one request per attempt, plus one per redirect hop |
//! | `pages` | one `GET` per page |
//!
//! The doors at the root ([`get`], [`head`], [`post`], [`put`], [`patch`],
//! [`delete`]) send on the process-wide default [`session()`], and [`located`]
//! holds the resource a URL names as a [`Holder`]. What every request shares
//! (the retry budget, the jittered backoff, the `Retry-After` reading, and
//! the two verdicts on whether a failure is worth another attempt) is
//! `retry`, crate-private, which the S3 backend draws on as well.
//!
//! ```no_run
//! use yggdryl::http;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let response = http::get("https://api.example.com/v1/health")?;
//! assert!(response.is_ok());
//! assert_eq!(response.text()?, "ok");
//! # Ok(())
//! # }
//! ```
//!
//! The module is laid out one file per type: [`Client`], [`Session`],
//! [`Request`] and [`Body`], [`Response`], [`Stream`], [`Pages`]; [`Headers`]
//! over the crate's metadata map with the typed readers beside it, [`Method`],
//! [`Status`] and the [`wire`] grammar of RFC 9112, [`Cookie`] and
//! [`CookieJar`], [`Authorization`], [`HttpOptions`], [`Pagination`].

pub mod authorization;
pub mod client;
pub mod cookie;
pub mod headers;
pub mod method;
pub(crate) mod netrc;
pub mod options;
pub mod pages;
pub mod pagination;
pub(crate) mod proxy;
pub mod request;
pub mod response;
pub(crate) mod retry;
pub mod server;
pub mod session;
pub mod status;
pub mod stream;
pub mod wire;

use std::sync::OnceLock;

pub use authorization::Authorization;
pub use client::{Client, StatsSnapshot};
pub use cookie::{Cookie, CookieJar};
pub use headers::{
    ContentRange, ETag, Headers, HeadersIntoIter, HeadersIter, Link, parse_http_date, parse_links,
    range_header, render_http_date,
};
pub use method::Method;
pub use options::HttpOptions;
pub use pages::Pages;
pub use pagination::{NextPage, Pagination};
pub use request::{Body, Request};
pub use response::Response;
pub use server::{Fault, Handler, Recorded, Server, ServerOptions};
pub use session::Session;
pub use status::Status;
pub use stream::Stream;
pub use wire::{
    ChunkedReader, ChunkedWriter, HttpVersion, RequestHead, ResponseHead, decode_chunked,
    encode_chunked, parse_request, parse_response, render_request, render_response,
};

use crate::Result;
use crate::holder::Holder;

/// The process-wide default session: default options over the shared
/// client, one cookie jar for the process.
///
/// Every door at the root and every [`Request`] built without a session
/// sends on it, so cookies a response sets ride on the next request from
/// anywhere in the process.
///
/// ```
/// use yggdryl::http;
///
/// // One session, however often it is asked for.
/// assert_eq!(http::session().stats(), http::session().stats());
/// ```
#[must_use]
pub fn session() -> Session {
    static DEFAULT: OnceLock<Session> = OnceLock::new();
    DEFAULT.get_or_init(Session::new).clone()
}

/// A session with `options`, over a client built for their transport.
///
/// ```
/// use std::time::Duration;
///
/// use yggdryl::http::{self, HttpOptions};
///
/// # fn main() -> yggdryl::Result<()> {
/// let session = http::session_with(HttpOptions::default().with_timeout(Duration::from_secs(5)))?;
/// assert_eq!(session.options().timeout(), Duration::from_secs(5));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// As [`Session::with_options`].
pub fn session_with(options: HttpOptions) -> Result<Session> {
    Session::with_options(options)
}

/// Hold the resource `url` names: a `GET` of it on a default session,
/// as [`Holder::HttpRequest`].
///
/// ```no_run
/// use yggdryl::{IOBase, http};
///
/// # fn main() -> yggdryl::Result<()> {
/// let resource = http::located("https://example.com/data/trades.parquet")?;
/// assert_eq!(resource.media_type().base(), &yggdryl::MimeType::PARQUET);
/// let footer = resource.read_range_bytes(resource.size() - 8, 8)?; // one HEAD, one ranged GET
/// assert_eq!(&footer[4..], b"PAR1");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// As [`Request::get`].
pub fn located(url: &str) -> Result<Holder> {
    located_with(url, HttpOptions::default())
}

/// Hold the resource `url` names, on a session with `options`.
///
/// # Errors
///
/// As [`Session::with_options`] and [`Session::get`].
pub fn located_with(url: &str, options: HttpOptions) -> Result<Holder> {
    Ok(Holder::HttpRequest(
        Session::with_options(options)?.get(url)?,
    ))
}

/// `GET url` on the default session, the body read whole.
///
/// # Errors
///
/// As [`Request::get`] and [`Request::send`].
pub fn get(url: &str) -> Result<Response> {
    session().get(url)?.send()
}

/// `HEAD url` on the default session.
///
/// # Errors
///
/// As [`Request::head`] and [`Request::send`].
pub fn head(url: &str) -> Result<Response> {
    session().head(url)?.send()
}

/// `POST body` to `url` on the default session, the answer read whole.
///
/// # Errors
///
/// As [`Request::post`] and [`Request::send`].
pub fn post(url: &str, body: Body) -> Result<Response> {
    session().post(url, body)?.send()
}

/// `PUT body` to `url` on the default session, the answer read whole.
///
/// # Errors
///
/// As [`Request::put`] and [`Request::send`].
pub fn put(url: &str, body: Body) -> Result<Response> {
    session().put(url, body)?.send()
}

/// `PATCH body` to `url` on the default session, the answer read whole.
///
/// # Errors
///
/// As [`Request::patch`] and [`Request::send`].
pub fn patch(url: &str, body: Body) -> Result<Response> {
    session().patch(url, body)?.send()
}

/// `DELETE url` on the default session.
///
/// # Errors
///
/// As [`Request::delete`] and [`Request::send`].
pub fn delete(url: &str) -> Result<Response> {
    session().delete(url)?.send()
}
