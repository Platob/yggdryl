//! Foreign Arrow filesystems as [`IOBase`](crate::io::IOBase) handles.
//!
//! [`ArrowFileSystem`] is a small synchronous vtable modeled on Arrow's
//! `FileSystem` API - the contract `pyarrow.fs`, Arrow C++, and Arrow Java
//! share - and the three roles here put any value implementing it behind the
//! crate's one storage abstraction:
//!
//! - [`Path`] is the generic location. It answers
//!   [`IOBase::kind`](crate::io::IOBase::kind) by asking the filesystem what
//!   is actually there, and runs every operation through the specialized
//!   implementation that fits.
//! - [`Folder`] is the container: it lists and resolves children. On an
//!   object store a directory is a prefix, and existence is what the
//!   filesystem itself reports.
//! - [`File`] is the leaf. Reads map straight onto ranged fetches; writes
//!   stage in memory and publish as one whole-value replacement on flush or
//!   close, because an Arrow filesystem replaces files rather than writing
//!   ranges.
//!
//! All three are lazy in the same way everything in [`crate::io`] is:
//! constructing one performs no filesystem call, reading something that does
//! not exist yields nothing, and writing creates it. Because the vtable adds
//! no dependency the module is unconditional, and every wrapper and record
//! method - [`Coded`](crate::io::Coded), globs, Hive partitions, IPC,
//! Parquet, Iceberg tables - is inherited rather than reimplemented.
//!
//! Two [`ArrowFileSystem`] implementations ship in-tree:
//! [`MemoryFileSystem`], the "memory" filesystem the tests and benchmarks
//! run on, and [`LocalFileSystem`], a thin `std::fs` mapping that proves the
//! vtable against a real OS filesystem without replacing [`crate::local`].
//!
//! ```
//! use std::sync::Arc;
//!
//! use yggdryl::arrowfs::{Folder, MemoryFileSystem};
//! use yggdryl::io::IOBase;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let filesystem = Arc::new(MemoryFileSystem::new());
//! let lake = Folder::from_location(filesystem, "lake")?;
//!
//! let mut leaf = lake.child_by("trades.bin")?;
//! leaf.write_all_bytes(b"AAPL")?;
//! leaf.close()?;
//!
//! assert_eq!(lake.ls(false, false)?.len(), 1);
//! assert_eq!(lake.child_by("trades.bin")?.read_all_bytes()?, b"AAPL");
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use crate::IOKind;
use crate::generic::Holder;

mod file;
mod folder;
mod path;
mod system;

pub use file::File;
pub use folder::Folder;
pub use path::Path;
pub use system::{ArrowFileSystem, FileInfo, LocalFileSystem, MemoryFileSystem, location_url};

/// Hold the resource `url` names on `filesystem`.
///
/// An existing directory becomes [`Holder::ArrowFolder`]; anything else,
/// including a location that does not exist yet, becomes an
/// [`Holder::ArrowFile`] leaf - the mirror of [`Holder::local`], so a caller
/// that knows which role it wants says so by constructing it explicitly.
pub fn located(filesystem: Arc<dyn ArrowFileSystem>, url: crate::Url) -> Holder {
    let folder = Folder::new(filesystem.clone(), url);
    if matches!(
        filesystem.file_info(folder.location()),
        Ok(info) if info.kind == IOKind::Directory
    ) {
        return Holder::ArrowFolder(folder);
    }
    Holder::ArrowFile(File::new(filesystem, folder.url().clone()))
}

#[cfg(test)]
mod tests;
