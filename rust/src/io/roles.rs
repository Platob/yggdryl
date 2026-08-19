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

    /// Delete the container itself, leaving whatever is inside it alone.
    ///
    /// This is the one mechanic a backend supplies for the lifecycle pair; the
    /// role assembles [`Self::folder_clear`] and [`Self::folder_remove`] from
    /// it. Two rules bind the implementation, both from [`IOBase::remove`]:
    ///
    /// - **Absence is success.** Issue the delete and map the store's own
    ///   not-found answer to `Ok(())` through [`skip_absent`](crate::io::skip_absent). Never probe
    ///   first to decide whether to proceed.
    /// - **A non-empty container is refused, not recursed.** Return the store's
    ///   own "directory not empty" failure; the role turns it into an error
    ///   naming the location.
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure.
    fn delete_folder(&mut self) -> Result<()>;

    /// Remove every child, keeping the container - a container's `clear`.
    ///
    /// The listing here is the work, not a probe: a container that does not
    /// exist lists nothing and the call becomes the no-op success the contract
    /// promises without a separate existence check. A backend able to empty a
    /// prefix in one call overrides this.
    ///
    /// # Errors
    ///
    /// Returns the backing store's listing or delete failure.
    fn folder_clear(&mut self) -> Result<()> {
        for mut child in self.list_folder(false, true)? {
            child.remove(true)?;
        }
        Ok(())
    }

    /// Delete the container, per [`IOBase::remove`]'s contract.
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure, or a refusal naming the
    /// location when it still has children and `recursive` is not set.
    fn folder_remove(&mut self, recursive: bool) -> Result<()> {
        if recursive {
            self.folder_clear()?;
        }
        match self.delete_folder() {
            // Only the store's own "still has children" answer is translated;
            // every other failure stays the typed error it is.
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                Err(crate::io::not_empty(self.folder_url()))
            }
            other => other,
        }
    }

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

    /// A container is never one whole byte value, so this needs no probe.
    fn folder_is_atomic(&self) -> bool {
        false
    }

    /// A container reads as the table beneath it, so its leaves decide.
    ///
    /// The role knows what [`IOBase::is_tabular`] would have to establish
    /// first - that this is a container - so the probe starts at the listing
    /// instead of at a kind that is already settled.
    fn folder_is_tabular(&self) -> bool {
        crate::io::container_is_tabular(self)
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

    /// Discard every byte, without creating the leaf when it is absent.
    ///
    /// A separate mechanic from `truncate(0)` because a truncation *writes*,
    /// and per [`IOBase::clear`] emptying a resource that does not exist must
    /// do nothing rather than bring it into being. Map the store's own
    /// not-found answer to `Ok(())` through [`skip_absent`](crate::io::skip_absent); never probe first.
    ///
    /// # Errors
    ///
    /// Returns the backing store's resize failure.
    fn clear_file(&mut self) -> Result<()>;

    /// Delete the leaf, per [`IOBase::remove`]'s contract.
    ///
    /// Complete removal for a leaf means the bytes *and* anything the handle
    /// holds over them - a live mapping, a staged write - so an implementation
    /// releases those first and a later flush cannot recreate what was deleted.
    /// Map the store's own not-found answer to `Ok(())` through
    /// [`skip_absent`](crate::io::skip_absent); never probe first.
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure.
    fn delete_file(&mut self) -> Result<()>;

    /// Delete the leaf, ignoring `recursive` - a leaf contains nothing.
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure.
    fn file_remove(&mut self, recursive: bool) -> Result<()> {
        // A leaf has no children, so there is nothing for the flag to reach.
        let _ = recursive;
        self.delete_file()
    }

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

    /// A leaf is one whole byte value unless its representation holds rows.
    ///
    /// A leaf is never a container, so the role answers from the media type
    /// alone where [`IOBase::is_atomic`] would also ask whether the resource
    /// exists - and asking that is a call into the backing store.
    fn file_is_atomic(&self) -> bool {
        !self.media_type().is_tabular()
    }

    /// A leaf holds rows exactly when its representation does.
    fn file_is_tabular(&self) -> bool {
        self.media_type().is_tabular()
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

    /// Whether the location is one whole byte value.
    ///
    /// A location's *handle* reports `inode/file` or `inode/directory` as its
    /// media type - a borrowed answer cannot be derived per call - so the shape
    /// questions read the location's name instead, which is what actually says
    /// what would be there. The role settles container-or-not once, through
    /// [`Self::path_kind`], and reads the name only after: one look at the file
    /// system, no resolution, and nothing opened.
    fn path_is_atomic(&self) -> bool {
        if self.path_kind().is_container() {
            return false;
        }
        !self.path_url().media_type().is_tabular()
    }

    /// Whether the location holds rows and columns.
    ///
    /// The same one look as [`Self::path_is_atomic`]: a container reads as the
    /// table beneath it, and anything else is answered by its name.
    fn path_is_tabular(&self) -> bool {
        if self.path_kind().is_container() {
            return crate::io::container_is_tabular(self);
        }
        self.path_url().media_type().is_tabular()
    }
}
