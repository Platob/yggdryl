//! One test file per file under `rust/src/http/`, mirrored file for file.
//!
//! The whole module is behind the `http` feature, so every module here
//! carries that cfg; the ones that pin something a caller cannot reach - the
//! retry schedule and its budget - carry the `internals` cfg beside it and
//! reach the crate through `yggdryl::internals`. `headers` declares the four
//! helper suites under `http/headers/` itself.
//!
//! [`http_server`] is not a suite: it is the in-process HTTP/1.1 server every
//! suite over a socket runs against, declared here once so they share one
//! fixture.

#[cfg(feature = "http")]
#[path = "support/http_server.rs"]
mod http_server;

#[cfg(feature = "http")]
#[path = "http/authorization.rs"]
mod authorization;
#[cfg(feature = "http")]
#[path = "http/cookie.rs"]
mod cookie;
#[cfg(feature = "http")]
#[path = "http/headers.rs"]
mod headers;
#[cfg(feature = "http")]
#[path = "http/method.rs"]
mod method;
#[cfg(feature = "http")]
#[path = "http/options.rs"]
mod options;
#[cfg(feature = "http")]
#[path = "http/pagination.rs"]
mod pagination;
#[cfg(all(feature = "http", feature = "internals"))]
#[path = "http/retry.rs"]
mod retry;
#[cfg(feature = "http")]
#[path = "http/status.rs"]
mod status;
#[cfg(feature = "http")]
#[path = "http/wire.rs"]
mod wire;
