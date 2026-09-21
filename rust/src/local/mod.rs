//! Local file system resources as [`IOBase`](crate::IOBase) handles.
//!
//! Three implementations cover a local tree, and every file system backend is
//! expected to supply the same three roles:
//!
//! - [`Path`] is the generic location. It answers
//!   [`IOBase::kind`](crate::IOBase::kind) by looking at what is actually
//!   there, and runs every operation through the specialized implementation
//!   that fits, so a caller who does not yet know what a location is can still
//!   use it.
//! - [`Folder`] is the container: it lists and resolves children.
//! - [`File`] is the leaf: its bytes are a memory mapping of one file.
//!
//! All three follow the shared [`IOBase`](crate::IOBase) laziness contract:
//! constructing one touches nothing, reading something that does not exist
//! yields nothing, and writing creates it.
//!
//! Grouping them here is deliberate. A remote store (S3, GCS, Azure) is the
//! same three ideas, so a new backend is a sibling module supplying the same
//! three roles rather than a change to anything here.
//!
//! ```no_run
//! use yggdryl::IOBase;
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/local/mod_.rs` pins and a caller cannot reach.
    //!
    //! The home container resolves from `HOME` and `USERPROFILE`. Taking the
    //! two values as arguments is the only way to name a home without
    //! touching the developer's real one, so the resolution is reached here
    //! rather than made API; everything a caller can observe is pinned
    //! through `yggdryl::` like any other test.
    use std::ffi::OsString;

    use crate::Result;
    use crate::local::Folder;

    /// Resolve the home container from the two environment values, `HOME`
    /// first, an empty value counting as unset.
    pub fn home_from(home: Option<OsString>, profile: Option<OsString>) -> Result<Folder> {
        Folder::home_from(home, profile)
    }
}
