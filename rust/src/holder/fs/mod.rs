//! Foreign filesystems as [`IOBase`](crate::IOBase) handles.
//!
//! [`FileSystem`] is a small synchronous vtable modeled on Arrow's
//! `FileSystem` API - the contract `pyarrow.fs`, Arrow C++, and Arrow Java
//! share - and the three roles here put any value implementing it behind the
//! crate's one storage abstraction:
//!
//! - [`Path`] is the generic location. It answers
//!   [`IOBase::kind`](crate::IOBase::kind) by asking the filesystem what
//!   is actually there, and runs every operation through the specialized
//!   implementation that fits.
//! - [`Folder`] is the container: it lists and resolves children. On an
//!   object store a directory is a prefix, and existence is what the
//!   filesystem itself reports.
//! - [`File`] is the leaf. Reads map straight onto ranged fetches; writes
//!   stage in memory and publish as one whole-value replacement on flush or
//!   close, matching the shared Arrow-compatible filesystem contract.
//!
//! All three follow the shared [`IOBase`](crate::IOBase) laziness contract:
//! constructing one performs no filesystem call, reading something that does
//! not exist yields nothing, and writing creates it. Because the vtable adds
//! no dependency the module is unconditional, and every wrapper and record
//! method - [`Coding`](crate::coding::Coding), globs, Hive partitions, IPC,
//! Parquet, Iceberg tables - is inherited rather than reimplemented.
//!
//! Two [`FileSystem`] implementations ship in-tree:
//! [`MemoryFileSystem`], the "memory" filesystem the tests and benchmarks
//! run on, and [`LocalFileSystem`], a thin `std::fs` mapping that proves the
//! vtable against a real OS filesystem without replacing [`crate::holder::local`].
//!
//! ```
//! use std::sync::Arc;
//!
//! use yggdryl::holder::fs::{Folder, MemoryFileSystem};
//! use yggdryl::IOBase;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let filesystem = Arc::new(MemoryFileSystem::new());
//! let lake = Folder::from_location(filesystem, "lake")?;
//!
//! let mut leaf = lake.child_by_path("trades.bin")?;
//! leaf.write_all_bytes(b"AAPL")?;
//! leaf.close()?;
//!
//! assert_eq!(lake.ls(false, false).count(), 1);
//! assert_eq!(lake.child_by_path("trades.bin")?.read_all_bytes()?, b"AAPL");
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use crate::holder::Holder;

mod file;
mod folder;
mod path;
mod system;

pub use file::File;
pub use folder::Folder;
pub use path::Path;
pub use system::{
    FileInfo, FileInfos, FileSystem, LocalFileSystem, MemoryFileSystem, location_url,
};

/// Hold the resource `url` names on `filesystem`.
///
/// The returned [`Holder::FsPath`] performs no filesystem call during
/// construction and resolves only when an operation needs the resource's
/// role. A caller that knows the role can construct [`Folder`] or [`File`]
/// explicitly.
pub fn located(filesystem: Arc<dyn FileSystem>, url: crate::Url) -> Holder {
    Holder::FsPath(Path::new(filesystem, url))
}

#[cfg(test)]
mod tests;
