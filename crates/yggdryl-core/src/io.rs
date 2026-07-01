//! The [`Io`] byte-source abstraction and its [`IoError`].

use crate::buffer::Buffer;
use crate::whence::Whence;

/// An error raised by an [`Io`] operation.
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

/// A seekable source of bytes — memory, a local file, a cloud object, an HTTP body
/// — behind one interface, so code that consumes bytes never cares where they
/// live.
///
/// An implementor supplies the cursor ([`position`](Io::position) /
/// [`set_position`](Io::set_position)), the total [`len`](Io::len) and the two
/// positional (random-access) primitives
/// ([`positional_read_bytes`](Io::positional_read_bytes) /
/// [`positional_write_bytes`](Io::positional_write_bytes)); the sequential
/// [`read_bytes`](Io::read_bytes) / [`write_bytes`](Io::write_bytes) and
/// [`seek`](Io::seek) are provided on top of them.
///
/// Positional reads hand back a zero-copy [`Buffer`] view when the source is
/// memory-resident, so nothing is copied on the read hot path.
///
/// ```
/// use yggdryl_core::{BytesIo, Io, Whence};
///
/// let mut io = BytesIo::from_bytes(b"hello world".to_vec());
///
/// // A positional read at an offset leaves the cursor untouched.
/// assert_eq!(io.positional_read_bytes(6, 5).unwrap().as_slice(), b"world");
/// assert_eq!(io.position(), 0);
///
/// // A sequential read advances the cursor.
/// assert_eq!(io.read_bytes(5).unwrap().as_slice(), b"hello");
/// assert_eq!(io.position(), 5);
///
/// // Seek back and overwrite in place.
/// io.seek(0, Whence::Start).unwrap();
/// io.write_bytes(b"HELLO").unwrap();
/// assert_eq!(io.positional_read_bytes(0, 11).unwrap().as_slice(), b"HELLO world");
/// ```
pub trait Io {
    /// The total number of bytes in the source.
    fn len(&self) -> Result<u64, IoError>;

    /// The current cursor position (a byte offset from the start).
    fn position(&self) -> u64;

    /// Moves the cursor to the absolute byte `position`.
    fn set_position(&mut self, position: u64);

    /// Reads up to `len` bytes at the absolute byte `offset`, returning a [`Buffer`]
    /// of the bytes actually available there (fewer than `len` near the end, empty
    /// at EOF). A memory-resident source returns a zero-copy view. Does not move the
    /// cursor. Errors [`OutOfBounds`](IoError::OutOfBounds) if `offset` is past the
    /// end.
    fn positional_read_bytes(&self, offset: u64, len: usize) -> Result<Buffer, IoError>;

    /// Writes `bytes` at the absolute byte `offset` — overwriting, and extending the
    /// source when the write runs past the end — returning the number of bytes
    /// written. Does not move the cursor. Errors [`ReadOnly`](IoError::ReadOnly) on a
    /// read-only source, or [`OutOfBounds`](IoError::OutOfBounds) if `offset` is past
    /// the end.
    fn positional_write_bytes(&mut self, offset: u64, bytes: &[u8]) -> Result<usize, IoError>;

    /// Whether the source holds no bytes.
    fn is_empty(&self) -> Result<bool, IoError> {
        Ok(self.len()? == 0)
    }

    /// Resolves a `whence`-relative `offset` to an absolute position, erroring if it
    /// falls outside `0..=len`.
    fn resolve_offset(&self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base = match whence {
            Whence::Start => 0,
            Whence::Current => i64::try_from(self.position()).map_err(|_| IoError::OutOfBounds)?,
            Whence::End => i64::try_from(self.len()?).map_err(|_| IoError::OutOfBounds)?,
        };
        let position = base
            .checked_add(offset)
            .filter(|resolved| *resolved >= 0)
            .ok_or(IoError::OutOfBounds)? as u64;
        if position > self.len()? {
            return Err(IoError::OutOfBounds);
        }
        Ok(position)
    }

    /// Moves the cursor to `offset` relative to `whence`, returning the new position.
    /// The cursor is left unchanged when the target is out of bounds.
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let position = self.resolve_offset(offset, whence)?;
        self.set_position(position);
        Ok(position)
    }

    /// Reads up to `len` bytes from the cursor, advancing it past the bytes read.
    fn read_bytes(&mut self, len: usize) -> Result<Buffer, IoError> {
        let bytes = self.positional_read_bytes(self.position(), len)?;
        self.set_position(self.position() + bytes.len() as u64);
        Ok(bytes)
    }

    /// Writes `bytes` at the cursor, advancing it past the bytes written, and
    /// returns the number written.
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        let written = self.positional_write_bytes(self.position(), bytes)?;
        self.set_position(self.position() + written as u64);
        Ok(written)
    }
}
