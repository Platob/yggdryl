//! The [`ByteIo`] positional byte-source abstraction, its [`IoError`], and the
//! [`Buffer`] leaf implementation.

use crate::buffer::Buffer;

/// An error raised by a [`ByteIo`] (or [`BitIo`](crate::BitIo)) operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// A positional offset resolved outside the valid `0..=len` range.
    OutOfBounds,
    /// The source is read-only and cannot be written.
    ReadOnly,
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IoError::OutOfBounds => {
                f.write_str("offset out of bounds — expected a position within `0..=len`")
            }
            IoError::ReadOnly => f.write_str("source is read-only — writing is not supported"),
        }
    }
}

impl std::error::Error for IoError {}

/// A random-access source of bytes — memory, a local file, a cloud object, an HTTP
/// body — addressed purely by absolute byte offset, with no cursor of its own.
///
/// An implementor supplies [`byte_len`](ByteIo::byte_len) and the two positional
/// primitives ([`positional_read_bytes`](ByteIo::positional_read_bytes) /
/// [`positional_write_bytes`](ByteIo::positional_write_bytes)). Layer the other
/// concerns on top: a moving cursor with [`ByteCursor`](crate::ByteCursor), a
/// bounded window with [`ByteSlice`](crate::ByteSlice), and bit addressing with
/// [`BitIo`](crate::BitIo) (blanket-implemented for every `ByteIo`).
///
/// Reads hand back a zero-copy [`Buffer`] view when the source is memory-resident,
/// so nothing is copied on the read hot path.
///
/// ```
/// use yggdryl_core::{Buffer, ByteIo};
///
/// let buf = Buffer::from_vec(b"hello world".to_vec());
/// assert_eq!(buf.byte_len().unwrap(), 11);
/// assert_eq!(buf.positional_read_bytes(6, 5).unwrap().as_slice(), b"world");
/// ```
pub trait ByteIo {
    /// The total number of bytes in the source.
    ///
    /// Named `byte_len` (not `len`) so it never collides with an inherent `len` on
    /// a leaf type such as [`Buffer`].
    fn byte_len(&self) -> Result<u64, IoError>;

    /// Reads up to `len` bytes at the absolute byte `offset`, returning a [`Buffer`]
    /// of the bytes actually available there (fewer than `len` near the end, empty
    /// at EOF). A memory-resident source returns a zero-copy view. Errors
    /// [`OutOfBounds`](IoError::OutOfBounds) if `offset` is past the end.
    fn positional_read_bytes(&self, offset: u64, len: usize) -> Result<Buffer, IoError>;

    /// Writes `bytes` at the absolute byte `offset` — overwriting, and extending the
    /// source when the write runs past the end — returning the number written.
    /// Errors [`ReadOnly`](IoError::ReadOnly) on a read-only source or
    /// [`OutOfBounds`](IoError::OutOfBounds) if `offset` is past the end.
    fn positional_write_bytes(&mut self, offset: u64, bytes: &[u8]) -> Result<usize, IoError>;
}

/// [`Buffer`] is the in-memory leaf: reads are zero-copy slices of the backing
/// bytes; a write copies the (shared, immutable) buffer, patches the window
/// (zero-filling any gap it opens past the end) and re-wraps it, so other clones
/// are untouched.
impl ByteIo for Buffer {
    fn byte_len(&self) -> Result<u64, IoError> {
        Ok(self.as_slice().len() as u64)
    }

    fn positional_read_bytes(&self, offset: u64, len: usize) -> Result<Buffer, IoError> {
        let offset = usize::try_from(offset).map_err(|_| IoError::OutOfBounds)?;
        let total = self.as_slice().len();
        if offset > total {
            return Err(IoError::OutOfBounds);
        }
        crate::log_event!(
            trace,
            "Buffer::positional_read_bytes offset={offset} len={len}"
        );
        let end = offset.saturating_add(len).min(total);
        Ok(self.slice(offset..end))
    }

    fn positional_write_bytes(&mut self, offset: u64, bytes: &[u8]) -> Result<usize, IoError> {
        let offset = usize::try_from(offset).map_err(|_| IoError::OutOfBounds)?;
        let total = self.as_slice().len();
        if offset > total {
            return Err(IoError::OutOfBounds);
        }
        crate::log_event!(
            trace,
            "Buffer::positional_write_bytes offset={offset} len={}",
            bytes.len()
        );
        let mut vec = self.as_slice().to_vec();
        let end = offset + bytes.len();
        if end > vec.len() {
            vec.resize(end, 0);
        }
        vec[offset..end].copy_from_slice(bytes);
        *self = Buffer::from_vec(vec);
        Ok(bytes.len())
    }
}
