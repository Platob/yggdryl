//! `io` — the byte / memory-access layer, and the cross-cutting value types it is built on.
//!
//! The `io` root holds what every part of the layer shares:
//!
//! - [`IoError`](crate::io::IoError) — the guided failures the byte-access methods return.
//! - [`Whence`](crate::io::Whence) — the seek anchor (`Start` / `Current` / `End`).
//! - [`Serializable`](crate::io::Serializable) — the root byte-codec trait
//!   (`serialize_bytes` / `deserialize_bytes`).
//!   case-insensitive, multi-value); every source carries one.
//! - [`IOMode`](crate::io::IOMode) — how a source may be accessed (read / write / append / …).
//! - [`IOKind`](crate::io::IOKind) — what a source is (missing / file / directory / heap).
//!
//! Below it, one module per concern:
//!
//! - [`memory`](crate::io::memory) — the abstract byte-access contract ([`IOBase`](crate::io::memory::IOBase)) with its concrete
//!   [`IOCursor`](crate::io::memory::IOCursor) / [`IOSlice`](crate::io::memory::IOSlice) wrappers, and the sources that implement it
//!   (the in-heap [`Heap`](crate::io::memory::Heap); a memory-mapped source plugs in against the same trait).

mod error;
mod kind;
mod mode;
mod path;
mod serializable;
mod whence;

pub mod local;
pub mod memory;

pub use error::IoError;
pub use kind::IOKind;
pub use mode::IOMode;
pub use path::Path;
pub use serializable::Serializable;
pub use whence::Whence;
