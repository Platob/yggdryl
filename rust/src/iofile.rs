//! The byte-leaf role every storage backend implements.

use crate::generic::Holder;
use crate::{Error, IOBase, IOKind, Listing, Result, Url};

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
    /// not-found answer to `Ok(())` through [`skip_absent`](crate::iobase::skip_absent); never probe first.
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
    /// [`skip_absent`](crate::iobase::skip_absent); never probe first.
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
    fn file_ls(&self) -> Listing {
        Listing::empty()
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
    fn file_child_by_path(&self, name: &str) -> Result<Holder> {
        Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!(
                "expected a container to resolve {name:?} against, got the file {}",
                self.file_url()
            ),
        )))
    }
}
