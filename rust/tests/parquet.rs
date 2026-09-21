//! One test file per file under `rust/src/parquet/`, under `tests/parquet/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the Parquet
//! codec's own files. The whole folder is behind the `parquet` feature, so
//! every module here carries that cfg. A test reaches the crate through
//! `yggdryl::`; where what it pins is not reachable that way, it reaches
//! `yggdryl::internals`, which exists only under the `internals` feature, and
//! the file that reaches it is declared behind that feature too.

#[cfg(feature = "parquet")]
#[path = "parquet/metadata.rs"]
mod metadata;
#[cfg(all(feature = "parquet", feature = "internals"))]
#[path = "parquet/mod_.rs"]
mod mod_;
