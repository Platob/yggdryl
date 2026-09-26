//! One test file per file under `rust/src/soap/`, under `tests/soap/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the SOAP 1.1
//! envelope and its HTTP binding. A test reaches the crate through
//! `yggdryl::` and nothing else.

#[path = "soap/http.rs"]
mod http;
#[path = "soap/mod_.rs"]
mod mod_;
