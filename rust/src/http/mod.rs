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
//! What every request shares - the retry budget, the jittered backoff, the
//! `Retry-After` reading, and the two verdicts on whether a failure is worth
//! another attempt - is `retry`, crate-private, which the S3 backend draws on
//! as well.
//!
//! The module is laid out one file per type: [`Headers`] over the crate's
//! metadata map with the typed readers beside it, [`Method`], [`Status`] and
//! the [`wire`] grammar of RFC 9112, [`Cookie`] and [`CookieJar`],
//! [`Authorization`], [`HttpOptions`], [`Pagination`].

pub mod authorization;
pub mod cookie;
pub mod headers;
pub mod method;
pub mod options;
pub mod pagination;
pub(crate) mod retry;
pub mod status;
pub mod wire;

pub use authorization::Authorization;
pub use cookie::{Cookie, CookieJar};
pub use headers::{
    ContentRange, ETag, Headers, HeadersIntoIter, HeadersIter, Link, parse_http_date, parse_links,
    range_header, render_http_date,
};
pub use method::Method;
pub use options::HttpOptions;
pub use pagination::{NextPage, Pagination};
pub use status::Status;
pub use wire::{
    ChunkedReader, ChunkedWriter, HttpVersion, RequestHead, ResponseHead, decode_chunked,
    encode_chunked, parse_request, parse_response, render_request, render_response,
};
