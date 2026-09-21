//! One test file per file under `rust/src/ipc/`, under `tests/ipc/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the Arrow IPC
//! media's own files. A test reaches the crate through `yggdryl::`; where what
//! it pins is not reachable that way, it reaches `yggdryl::internals`, which
//! exists only under the `internals` feature, and the file that reaches it is
//! declared behind that feature here.

#[cfg(feature = "internals")]
#[path = "ipc/mod_.rs"]
mod mod_;
