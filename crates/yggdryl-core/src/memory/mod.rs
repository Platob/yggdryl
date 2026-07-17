//! `memory` — the **abstract byte / memory-access layer**.
//!
//! This module owns the abstraction over *where the bytes live*: the traits that define
//! positioned and cursor access to a byte region, independent of whether that region is an
//! in-heap allocation or a memory-mapped file. A concrete backing implements them, so everything
//! above reads and writes through one contract:
//!
//! - [`IOBase`] — positioned access: `pread` / `pwrite` at an explicit offset (no cursor).
//! - [`IOCursor`] — cursor access built on [`IOBase`]: `read` / `write` advancing a position,
//!   with [`Whence`]-relative `seek`, plus the bounded bulk readers (`read_exact_vec`, …).
//! - [`IOSlice`] — a zero-copy sub-range view over an [`IOBase`].
//! - [`Whence`] — the seek anchor (`Start` / `Current` / `End`).
//! - [`IoError`] — the guided failures the byte-access methods return.
//!
//! The concrete in-heap backing is [`Bytes`] (an owned byte `Vec` + cursor). A memory-mapped
//! backing plugs in against the same traits.

mod base;
mod bytes;
mod cursor;
mod error;
mod slice;
mod whence;

pub use base::IOBase;
pub use bytes::Bytes;
pub use cursor::IOCursor;
pub use error::IoError;
pub use slice::IOSlice;
pub use whence::Whence;
