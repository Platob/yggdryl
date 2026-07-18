//! `io` — the byte / memory-access layer, and the cross-cutting value types it is built on.
//!
//! The `io` root holds what every part of the layer shares:
//!
//! - [`IoError`](crate::io::IoError) — the guided failures the byte-access methods return.
//! - [`Whence`](crate::io::Whence) — the seek anchor (`Start` / `Current` / `End`).
//! - [`Serializable`](crate::io::Serializable) — the root byte-codec trait
//!   (`serialize_bytes` / `deserialize_bytes`).
//! - [`IOMode`](crate::io::IOMode) — how a source may be accessed (read / write / append / …).
//! - [`IOKind`](crate::io::IOKind) — what a source is (missing / file / directory / heap).
//!
//! Below it, one module per concern:
//!
//! - [`memory`](crate::io::memory) — the abstract byte-access contract ([`IOBase`](crate::io::memory::IOBase)) with its concrete
//!   [`IOCursor`](crate::io::memory::IOCursor) / [`IOSlice`](crate::io::memory::IOSlice) wrappers, and the sources that implement it
//!   (the in-heap [`Heap`](crate::io::memory::Heap); a memory-mapped source plugs in against the same trait).
//!   `IOBase` is the **central access path**: besides bytes it carries the addressing
//!   ([`uri`](crate::io::memory::IOBase::uri)) and the whole filesystem-graph surface
//!   (`ls` / `ls_recursive` / `children` / `name` / `parent` / `rm` family), so every
//!   source — in-memory, local, object store — is a node of one uniform IO graph.

mod any;
mod error;
mod kind;
mod meminfo;
mod mode;
mod serializable;
mod whence;

#[cfg(feature = "amd")]
pub mod amd;
pub mod local;
pub mod memory;

pub use any::{open, open_str, AnyIO};
pub use error::IoError;
pub use kind::IOKind;
pub(crate) use meminfo::disk_memory;
pub use meminfo::MemoryInfo;
pub use mode::IOMode;
pub use serializable::Serializable;
pub use whence::Whence;
