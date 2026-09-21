//! One test file per file under `rust/src/toml/`, under `tests/toml/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the TOML codec's
//! own files. A test reaches the crate through `yggdryl::`; where what it pins
//! is not reachable that way, it reaches `yggdryl::internals`, which exists
//! only under the `internals` feature, and the file that reaches it is
//! declared behind that feature here.

#[path = "toml/mod_.rs"]
mod mod_;
#[cfg(feature = "internals")]
#[path = "toml/wire.rs"]
mod wire;
