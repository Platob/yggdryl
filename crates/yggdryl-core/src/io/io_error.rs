//! [`IoError`] — the failure modes of the positioned-IO surface.

use core::fmt;

use crate::Whence;

/// An error raised by an [`IOBase`](crate::IOBase) /
/// [`TypedIOBase`](crate::TypedIOBase) operation.
///
/// One enum covers seek, read, and write for the whole IO surface, so callers
/// handle failures uniformly regardless of the concrete resource. In the bindings
/// it surfaces as a Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::{IoError, Whence};
///
/// let err = IoError::InvalidSeek { offset: -1, whence: Whence::Start };
/// assert!(err.to_string().contains("start"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// A seek/position resolved to before the start of the resource (or beyond
    /// the addressable range). Pass an offset that lands within `0..=len`.
    InvalidSeek {
        /// The requested offset.
        offset: i64,
        /// The origin the offset was measured from.
        whence: Whence,
    },
    /// A read needed more data than the resource had left. Read within bounds or
    /// check the remaining length first.
    UnexpectedEof {
        /// The number of elements the read required.
        needed: usize,
        /// The number of elements actually available.
        available: usize,
    },
    /// A [`bit_seek`](crate::IOBase::bit_seek) was given a bit offset that is not a
    /// multiple of 8. This cursor addresses whole bytes (every seek origin is
    /// byte-aligned), so a non-multiple-of-8 offset can never land on a byte
    /// boundary; pass an offset that is a multiple of 8.
    UnalignedBitSeek {
        /// The offending bit offset (not a multiple of 8).
        offset: i64,
    },
    /// The underlying resource raised an I/O failure; the string is the source
    /// error's message.
    Io(String),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSeek { offset, whence } => {
                write!(f, "invalid seek: offset {offset} from {whence}")
            }
            Self::UnexpectedEof { needed, available } => write!(
                f,
                "unexpected end of resource: needed {needed}, {available} available"
            ),
            Self::UnalignedBitSeek { offset } => write!(
                f,
                "unaligned bit seek: offset {offset} is not a multiple of 8; seek to a byte-aligned bit offset (a multiple of 8)"
            ),
            Self::Io(message) => write!(f, "i/o error: {message}"),
        }
    }
}

impl std::error::Error for IoError {}

impl From<std::io::Error> for IoError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
