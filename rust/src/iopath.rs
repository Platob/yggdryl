//! The unresolved-location role every storage backend implements.

use crate::{IOBase, IOKind, MediaType, MimeType, Url};

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

    /// The media type of the location, including an explicit declaration.
    fn path_media_type(&self) -> MediaType {
        self.media_type().clone()
    }

    /// Whether the location is one whole byte value.
    ///
    /// The handle owns filename inference and any explicit media declaration,
    /// so the shape is derived from that one canonical answer. A directory is
    /// neither an atomic byte value nor a tabular value of its own.
    fn path_is_atomic(&self) -> bool {
        let media_type = self.media_type();
        media_type.base() != &MimeType::DIRECTORY && !media_type.is_tabular()
    }

    /// Whether the location holds rows and columns.
    ///
    /// A container reads as the table beneath it; a leaf is answered by the
    /// handle's inferred or explicitly declared representation.
    fn path_is_tabular(&self) -> bool {
        let media_type = self.media_type();
        if media_type.base() == &MimeType::DIRECTORY {
            return crate::iobase::container_is_tabular(self);
        }
        media_type.is_tabular()
    }
}
