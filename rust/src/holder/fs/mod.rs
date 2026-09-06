//! Arrow-compatible filesystems bound to the crate's one [`IOBase`](crate::IOBase).
//!
//! [`FileSystem`] is the public, object-safe seam. It represents selectors,
//! metadata, distinct directory/file lifecycle operations, native copy/move,
//! random and sequential inputs, and forwarding output/append streams without
//! whole-object staging. An unsupported backend capability returns
//! [`Error::Unsupported`](crate::Error::Unsupported).
//!
//! [`BoundLocation`] retains the filesystem equality domain, the exact opaque
//! filesystem path, the caller's optional URI spelling, and a masked
//! diagnostic form as separate facts. Injected paths are never parsed as URLs;
//! only [`ResolvedFileSystemUri`] crosses the URI boundary. [`Path`], [`File`],
//! [`Folder`], their parents and children, listings, and globs all carry that
//! same binding.
//!
//! [`MemoryFileSystem`] and [`LocalFileSystem`] are complete reference
//! implementations. Language bindings adapt their native Arrow filesystem to
//! this trait; storage behavior is not reimplemented in the binding.
//!
//! ```
//! use std::sync::Arc;
//!
//! use yggdryl::holder::fs::{FileSystem, Folder, MemoryFileSystem};
//! use yggdryl::IOBase;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
//! filesystem.create_dir("lake", true)?;
//! let lake = Folder::from_path(filesystem, "lake", None)?;
//!
//! let mut leaf = lake.child_by_path("trades.bin")?;
//! leaf.write_all_bytes(b"AAPL")?;
//!
//! assert_eq!(lake.ls(false, false).count(), 1);
//! assert_eq!(lake.child_by_path("trades.bin")?.read_all_bytes()?, b"AAPL");
//! # Ok(())
//! # }
//! ```

use crate::holder::Holder;

mod file;
mod folder;
mod local;
mod location;
mod memory;
mod path;
mod stream;
mod system;
mod transfer;
mod uri;

pub use file::File;
pub use folder::Folder;
pub use local::LocalFileSystem;
pub use location::{BoundLocation, BoundLocationIdentity, mask_uri};
pub use memory::MemoryFileSystem;
pub use path::Path;
pub use stream::{ByteReader, ByteWriter, RandomAccessReader};
pub use system::{FileInfo, FileInfos, FileSelector, FileSystem, OutputMetadata};
pub use transfer::{copy_bound, move_bound};
pub use uri::{ResolvedFileSystem, ResolvedFileSystemUri, S3AddressingStyle, S3FileSystemOptions};

/// Hold the resource `url` names on `filesystem`.
///
/// The returned [`Holder::FsPath`] performs no filesystem call during
/// construction and resolves only when an operation needs the resource's
/// role. A caller that knows the role can construct [`Folder`] or [`File`]
/// explicitly.
pub fn located(location: BoundLocation) -> Holder {
    Holder::FsPath(Path::new(location))
}

#[cfg(test)]
mod tests;
