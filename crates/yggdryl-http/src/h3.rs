//! HTTP/3 transport stub — falls back to HTTP/2 for all requests.
//!
//! Full QUIC/UDP support (via `quinn` + `h3`) is planned. The `http3` feature
//! currently implies `http2`, so enabling it turns on H2 with the
//! `h3 → h2 → h1.1` fallback semantics: the client advertises H3 preference
//! but uses the best version the server and network actually support.
//!
//! When a proper QUIC implementation lands it will live here, entirely behind
//! the `http3` feature gate, without changing the public API.
