//! `memory` — the **abstract byte / memory-access layer**.
//!
//! This module owns the abstraction over *where the bytes live*: the traits that define
//! positioned and cursor access to a byte region, independent of whether that region is an
//! in-heap allocation or a memory-mapped file. A concrete **source** implements them, so
//! everything above reads and writes through one contract:
//!
//! - [`IOBase`] — positioned access: `pread_byte_array` / `pwrite_byte_array` primitives, the
//!   typed `byte` / `bit` / `i32` / `i64` accessors, `pread_into` transfers, and `Vec`-like
//!   `capacity` / `reserve`.
//! - [`IOCursor`] — cursor access built on [`IOBase`]: `read` / `write` advancing a position,
//!   [`Whence`]-relative `seek`, typed `read_byte` / `read_i32` / `read_i64`, and the bounded
//!   bulk readers (`read_exact_vec`, …).
//! - [`IOSlice`] — a bounded sub-range view over an [`IOBase`].
//! - [`Whence`] — the seek anchor (`Start` / `Current` / `End`).
//! - [`IoError`] — the guided failures the byte-access methods return.
//!
//! The concrete in-heap source is [`Heap`] (an owned byte `Vec` + cursor + capacity). A
//! memory-mapped source plugs in against the same traits.

mod base;
mod cursor;
mod error;
mod heap;
mod slice;
mod whence;

pub use base::IOBase;
pub use cursor::IOCursor;
pub use error::IoError;
pub use heap::Heap;
pub use slice::IOSlice;
pub use whence::Whence;
