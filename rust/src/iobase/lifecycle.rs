//! Shared lifecycle and size refusals for [`IOBase`](super::IOBase).

use crate::{Codec, Error, MimeType, Result, Url};

/// Treat a backend's own not-found answer as a completed removal.
///
/// This is the shape [`IOBase::clear`] and [`IOBase::remove`] require: issue the
/// delete, and let the store say whether there was anything there. A backend
/// with a different absence signal - a store's no-such-key, an HTTP 404 - maps
/// its own answer the same way; everything else stays the typed failure it is,
/// because a permission or network error is not an absence.
///
/// ```
/// use yggdryl::io::skip_absent;
///
/// # fn main() -> yggdryl::Result<()> {
/// let absent = std::io::Error::from(std::io::ErrorKind::NotFound);
/// skip_absent(Err::<(), _>(absent))?;
///
/// let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
/// assert!(skip_absent(Err::<(), _>(denied)).is_err());
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns every failure that is not [`std::io::ErrorKind::NotFound`].
pub fn skip_absent<T: Default>(result: std::io::Result<T>) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(Error::Io(error)),
    }
}

/// Refuse to delete a container that still holds children.
///
/// `remove(false)` on a populated container is an error naming the location and
/// the fact that it has children - never a silent success and never a silent
/// recursion.
pub fn not_empty(url: &Url) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::DirectoryNotEmpty,
        format!(
            "expected an empty container to remove, got {url}, which still has children; pass \
             recursive to delete them too"
        ),
    ))
}

/// Report a value too large for this platform's address space.
pub(crate) fn oversized(size: u64) -> Error {
    Error::Io(std::io::Error::other(format!(
        "expected a value addressable by usize, got {size} bytes"
    )))
}

/// The MIME type naming one content coding, for media-type bookkeeping.
pub(crate) fn coding_mime(codec: Codec) -> Option<MimeType> {
    match codec {
        Codec::Identity => None,
        Codec::Gzip => Some(MimeType::GZIP),
        Codec::Zlib | Codec::Deflate => Some(MimeType::ZLIB),
        Codec::Zstd => Some(MimeType::ZSTD),
    }
}
