//! `io` — the byte / memory-access layer, and the cross-cutting value types it is built on.
//!
//! The `io` root holds what every part of the layer shares:
//!
//! - [`IoError`](crate::io::IoError) — the guided failures the byte-access methods return.
//! - [`Whence`](crate::io::Whence) — the seek anchor (`Start` / `Current` / `End`).
//!
//! Below it, one module per concern:
//!
//! - [`memory`](crate::io::memory) — the abstract byte-access contract ([`IOBase`](crate::io::memory::IOBase)) with its concrete
//!   [`IOCursor`](crate::io::memory::IOCursor) / [`IOSlice`](crate::io::memory::IOSlice) wrappers, and the sources that implement it
//!   (the in-heap [`Heap`](crate::io::memory::Heap); a memory-mapped source plugs in against the same trait).
//! - [`uri`](crate::io::uri) — the addressing layer: RFC 3986 [`Uri`](crate::io::uri::Uri) / [`Url`](crate::io::uri::Url) / [`Authority`](crate::io::uri::Authority).

mod error;
mod whence;

pub mod memory;
pub mod uri;

pub use error::IoError;
pub use whence::Whence;
