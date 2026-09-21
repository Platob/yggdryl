//! One test file per file at the crate root, under `tests/root/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the files the
//! crate root itself holds. A test reaches the crate through `yggdryl::`;
//! where what it pins is not reachable that way, it reaches
//! `yggdryl::internals`, which exists only under the `internals` feature.

#[cfg(feature = "internals")]
#[path = "root/utf8.rs"]
mod utf8;
