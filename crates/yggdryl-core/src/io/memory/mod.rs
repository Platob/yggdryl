//! `io::memory` — the **generic byte / memory-access layer**.
//!
//! This module owns the abstraction over *where the bytes live*: the traits that define
//! positioned and cursor access to a byte region, independent of whether that region is an
//! in-heap allocation or (a future backing) a memory-mapped file. Every physical backing
//! implements them, so the typed layer above reads and writes through one contract:
//!
//! - [`IOBase`] — positioned access: `pread` / `pwrite` at an explicit offset (no cursor).
//! - [`IOCursor`] — cursor access built on [`IOBase`]: `read` / `write` advancing a position,
//!   with [`Whence`]-relative `seek`, plus the bounded bulk readers (`read_exact_vec`, …).
//! - [`IOSlice`] — a zero-copy sub-range view over an [`IOBase`].
//! - [`Whence`] — the seek anchor (`Start` / `Current` / `End`).
//!
//! DESIGN: the memory layer is being generalized so a column's bytes can come from either an
//! owned in-heap allocation or a memory-mapped region behind the same traits. The abstract
//! contracts live here; the concrete backings (the Arrow-backed heap `Buffer`, and mmap) plug in
//! against them.

mod base;
mod cursor;
mod slice;
mod whence;

pub use base::IOBase;
pub use cursor::IOCursor;
pub use slice::IOSlice;
pub use whence::Whence;
