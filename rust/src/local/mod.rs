//! Local file system resources as [`IOBase`](crate::io::IOBase) handles.
//!
//! Three implementations cover a local tree, and every file system backend is
//! expected to supply the same three roles:
//!
//! - [`Path`] is the generic location. It answers
//!   [`IOBase::kind`](crate::io::IOBase::kind) by looking at what is actually
//!   there, and runs every operation through the specialized implementation
//!   that fits, so a caller who does not yet know what a location is can still
//!   use it.
//! - [`Folder`] is the container: it lists and resolves children.
//! - [`File`] is the leaf: its bytes are a memory mapping of one file.
//!
//! All three are lazy in the same way everything in [`crate::io`] is:
//! constructing one touches nothing, reading something that does not exist
//! yields nothing, and writing creates it.
//!
//! Grouping them here is deliberate. A remote store (S3, GCS, Azure) is the
//! same three ideas, so a new backend is a sibling module supplying the same
//! three roles rather than a change to anything here.
//!
//! ```no_run
//! use yggdryl::io::IOBase;
//! use yggdryl::local::Folder;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let root = Folder::temporary()?;
//!
//! // Children resolve by name, and a listing is stable in sort order.
//! let mut leaf = root.child_by_path("trades.arrows")?;
//! leaf.write_all_bytes(b"...")?;
//! # Ok(())
//! # }
//! ```

mod file;
mod folder;
mod path;

pub use file::File;
pub use folder::Folder;
pub use path::Path;

#[cfg(test)]
mod tests;
