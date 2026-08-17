//! The three roles every storage backend supplies.
//!
//! [`IOBase`] says how to move bytes. These traits say what a resource *is*,
//! and each one pre-implements the parts of [`IOBase`] that follow from that:
//!
//! - [`IOFolder`] is a container. It holds no bytes, so reads yield nothing,
//!   byte writes are refused with a reason, and `truncate(0)` is what brings it
//!   into being. An implementation supplies its location, how to create it, and
//!   how to list it.
//! - [`IOFile`] is a leaf. Its size and bytes are its own; the role supplies
//!   the container answers - it is not a container, it lists nothing.
//! - [`IOPath`] is a location that has not been resolved yet. It answers by
//!   looking: an existing directory behaves as a folder, an existing file as a
//!   file, and a location that does not exist yet stays undecided until a write
//!   settles it.
//!
//! A backend implements the role and gets the boilerplate. `local` is the
//! reference implementation; a remote store is the same three roles over a
//! different transport.

use crate::generic::Holder;
use crate::io::IOBase;
use crate::{Error, IOKind, MediaType, MimeType, Result, Url};

/// The media type every container reports.
static DIRECTORY_MEDIA_TYPE: std::sync::LazyLock<MediaType> =
    std::sync::LazyLock::new(|| MediaType::from(MimeType::DIRECTORY));

/// A resource that holds other resources.
///
/// Implement the four required members and the byte half of [`IOBase`] follows:
/// a container reads as empty, refuses byte writes, and is created by
/// truncating it to zero.
pub trait IOFolder: IOBase {
    /// The container's location.
    fn folder_url(&self) -> &Url;

    /// Return whether the container exists yet.
    fn folder_exists(&self) -> bool;

    /// Create the container and every missing parent.
    ///
    /// # Errors
    ///
    /// Returns the backing store's creation failure.
    fn create_folder(&self) -> Result<()>;

    /// List the container's entries, optionally descending.
    ///
    /// A container that does not exist lists nothing rather than failing, and
    /// private entries - those whose name begins with a dot - are excluded
    /// unless `include_private` asks for them.
    ///
    /// # Errors
    ///
    /// Returns the backing store's listing failure.
    fn list_folder(&self, recursive: bool, include_private: bool) -> Result<Vec<Holder>>;

    /// List a container, expanding a glob location into what it names.
    ///
    /// A container's location can be a pattern - `lake/**/*.parquet` - because
    /// a pattern is how a caller says "these children" in one string. Listing
    /// one walks up to the fixed root it was split from and expands the pattern
    /// there, so `ls` answers with the entries the pattern selects instead of
    /// looking for a directory literally named `**`. A plain location lists
    /// normally.
    ///
    /// # Errors
    ///
    /// Returns the backing store's listing failure.
    fn folder_ls(&self, recursive: bool, include_private: bool) -> Result<Vec<Holder>> {
        let (_, pattern) = self.folder_url().glob_parts()?;
        let Some(pattern) = pattern else {
            return self.list_folder(recursive, include_private);
        };
        // Walk up one level per pattern segment to reach the fixed root.
        let depth = pattern.split('/').filter(|part| !part.is_empty()).count();
        let mut root = self.parent();
        for _ in 1..depth {
            root = root.and_then(|holder| holder.parent());
        }
        match root {
            Some(root) => root.glob(&pattern, include_private),
            // Nothing above the pattern, so there is nowhere to expand it.
            None => Ok(Vec::new()),
        }
    }

    /// Read nothing: a container holds no bytes of its own.
    fn folder_pread(&self) -> Result<usize> {
        Ok(0)
    }

    /// Refuse a byte write, naming the container that was addressed.
    ///
    /// # Errors
    ///
    /// Always returns [`std::io::ErrorKind::IsADirectory`].
    fn folder_pwrite(&self, bytes: usize) -> Result<usize> {
        Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::IsADirectory,
            format!(
                "expected a file to write {bytes} bytes into, got the directory {}",
                self.folder_url()
            ),
        )))
    }

    /// Truncate a container, which means creating it when `size` is zero.
    ///
    /// # Errors
    ///
    /// Returns an error for any non-zero size, or the creation failure.
    fn folder_truncate(&self, size: u64) -> Result<()> {
        if size != 0 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::IsADirectory,
                format!(
                    "expected a truncation to 0 for the directory {}, got {size}",
                    self.folder_url()
                ),
            )));
        }
        self.create_folder()
    }

    /// The media type of a container, which is always `inode/directory`.
    fn folder_media_type(&self) -> &'static MediaType {
        &DIRECTORY_MEDIA_TYPE
    }

    /// The kind of a container, which is always [`IOKind::Directory`].
    fn folder_kind(&self) -> IOKind {
        IOKind::Directory
    }
}

/// A resource that holds bytes.
///
/// The role supplies the container half of [`IOBase`]: a leaf contains nothing,
/// so listing it is empty rather than an error, and its kind follows from
/// whether it exists yet.
pub trait IOFile: IOBase {
    /// The leaf's location.
    fn file_url(&self) -> &Url;

    /// Return whether the leaf exists yet.
    fn file_exists(&self) -> bool;

    /// List nothing: a leaf contains no other resources.
    fn file_ls(&self) -> Result<Vec<Holder>> {
        Ok(Vec::new())
    }

    /// A leaf that exists is a file; one that does not has not been decided.
    fn file_kind(&self) -> IOKind {
        if self.file_exists() {
            IOKind::File
        } else {
            IOKind::Unknown
        }
    }

    /// Refuse to resolve a child, naming the leaf that was addressed.
    ///
    /// # Errors
    ///
    /// Always returns [`std::io::ErrorKind::NotADirectory`].
    fn file_child_by(&self, name: &str) -> Result<Holder> {
        Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!(
                "expected a container to resolve {name:?} against, got the file {}",
                self.file_url()
            ),
        )))
    }
}

/// A location whose role is decided by what is actually there.
///
/// This is the entry point a caller reaches for when a path arrives from
/// outside - an argument, a listing, a configuration value - and nothing yet
/// says whether it names a container or a leaf.
pub trait IOPath: IOBase {
    /// The location.
    fn path_url(&self) -> &Url;

    /// Return whether the location names an existing container.
    fn is_folder(&self) -> bool;

    /// Return whether the location names an existing leaf.
    fn is_file(&self) -> bool;

    /// Return whether anything is there yet.
    fn path_exists(&self) -> bool {
        self.is_folder() || self.is_file()
    }

    /// Resolve what the location is, without touching its contents.
    ///
    /// A glob answers first and answers container: a pattern names a set of
    /// children, so everything that asks "can this hold others?" - a listing, a
    /// walk, a generic handle picking an implementation - gets the right answer
    /// before anything touches the file system.
    fn path_kind(&self) -> IOKind {
        if self.path_url().is_glob() || self.is_folder() {
            IOKind::Directory
        } else if self.is_file() {
            IOKind::File
        } else {
            // Nothing is there, so nothing has decided; a write settles it.
            IOKind::Unknown
        }
    }

    /// The media type of the location: its container type, or its name's.
    fn path_media_type(&self) -> MediaType {
        if self.path_url().is_glob() || self.is_folder() {
            return DIRECTORY_MEDIA_TYPE.clone();
        }
        self.path_url().media_type()
    }
}
