//! The container role every storage backend implements.

use crate::{Error, IOBase, IOKind, Listing, MediaType, MimeType, Result, Url};

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

    /// List the container's entries, one at a time, optionally descending.
    ///
    /// A container that does not exist lists nothing rather than failing, and
    /// private entries - those whose name begins with a dot - are excluded
    /// unless `include_private` asks for them.
    ///
    /// The walk is lazy and the failure arrives as an entry, so an
    /// implementation never builds the result it is describing; whatever it
    /// does hold - one directory's entry names, say - says what bounds it.
    fn list_folder(&self, recursive: bool, include_private: bool) -> Listing;

    /// Delete the container itself, leaving whatever is inside it alone.
    ///
    /// This is the one mechanic a backend supplies for the lifecycle pair; the
    /// role assembles [`Self::folder_clear`] and [`Self::folder_remove`] from
    /// it. Two rules bind the implementation, both from [`IOBase::remove`]:
    ///
    /// - **Absence is success.** Issue the delete and map the store's own
    ///   not-found answer to `Ok(())` through [`skip_absent`](crate::iobase::skip_absent). Never probe
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
        for child in self.list_folder(false, true) {
            child?.remove(true)?;
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
                Err(crate::iobase::not_empty(self.folder_url()))
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
    fn folder_ls(&self, recursive: bool, include_private: bool) -> Listing {
        let pattern = match self.folder_url().glob_parts() {
            Ok((_, pattern)) => pattern,
            Err(error) => return Listing::failing(error),
        };
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
            Some(root) => match root.glob(&pattern, include_private) {
                Ok(listing) => listing,
                Err(error) => Listing::failing(error),
            },
            // Nothing above the pattern, so there is nowhere to expand it.
            None => Listing::empty(),
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
        crate::iobase::container_is_tabular(self)
    }
}
