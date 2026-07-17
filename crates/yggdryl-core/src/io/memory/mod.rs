//! `memory` — the **abstract byte / memory-access contract**, and its concrete pieces.
//!
//! This module owns the abstraction over *where the bytes live*: the [`IOBase`] contract that
//! defines positioned access to a byte region (independent of whether that region is an
//! in-heap allocation or a memory-mapped file), plus the concrete pieces built over it. A
//! concrete **source** implements [`IOBase`], and everything above reads and writes through
//! the one contract:
//!
//! - [`IOBase`] — the source contract: `pread_byte_array` / `pwrite_byte_array` primitives, the
//!   typed `byte` / `bit` / `i32` / `i64` accessors, `pread_into` transfers, `Vec`-like
//!   `capacity` / `reserve`, an addressing [`uri`](IOBase::uri), and the
//!   [`cursor`](IOBase::cursor) / [`window`](IOBase::window) builders.
//! - [`IOCursor`] — a concrete **cursor** (a moving position) over any source: `read` / `write`
//!   advance it, [`Whence`]-relative `seek`, typed `read_byte` / `read_i32` / `read_i64`, and the
//!   bounded bulk readers (`read_to_end`, `read_exact_vec`).
//! - [`IOSlice`] — a concrete bounded **window** over any source, addressed from its own `0`.
//!
//! The seek anchor [`Whence`] and the guided [`IoError`] live at the [`io`](crate::io) root and
//! are re-exported here for convenience. The concrete in-memory source is [`Heap`] (an owned
//! byte `Vec` + built-in cursor + capacity); the local-filesystem family (`LocalIO` over the
//! raw `Mmap`) lives in [`local`](crate::io::local) and implements the same contract.
//! `IOBase` also carries the **graph surface** (`ls` streaming children of the same type,
//! `rm` CRUD, the memory-tree container reads); the in-memory sources are **leaves**
//! ([`NoChildren`]).

mod base;
mod cursor;
mod heap;
mod slice;

pub(crate) use base::{
    stage_pread_i32_array, stage_pread_i64_array, stage_pwrite_byte_repeat, stage_pwrite_i32_array,
    stage_pwrite_i32_repeat, stage_pwrite_i64_array, stage_pwrite_i64_repeat,
};
pub(crate) use cursor::cursor_methods;

pub use crate::io::{IoError, Whence};

pub use base::{IOBase, NoChildren};
pub use cursor::IOCursor;
pub use heap::Heap;
pub use slice::IOSlice;
