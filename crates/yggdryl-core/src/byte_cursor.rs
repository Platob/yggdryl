//! The [`ByteCursor`] — a stateful cursor over a [`ByteIo`].

use crate::buffer::Buffer;
use crate::byte_io::{ByteIo, IoError};
use crate::whence::Whence;

/// A read/write cursor over an inner [`ByteIo`]. It turns the positional primitives
/// into sequential [`read_bytes`](ByteCursor::read_bytes) /
/// [`write_bytes`](ByteCursor::write_bytes) that advance a mutable cursor, plus
/// [`seek`](ByteCursor::seek) — the stateful layer the positional [`ByteIo`]
/// deliberately leaves out.
///
/// ```
/// use yggdryl_core::{Buffer, ByteCursor, Whence};
///
/// let mut io = ByteCursor::new(Buffer::from_vec(b"hello world".to_vec()));
/// assert_eq!(io.read_bytes(5).unwrap().as_slice(), b"hello");
/// assert_eq!(io.position(), 5);
///
/// io.seek(0, Whence::Start).unwrap();
/// io.write_bytes(b"HELLO").unwrap();
/// assert_eq!(io.get_ref().as_slice(), b"HELLO world");
/// ```
#[derive(Clone, Debug, Default)]
pub struct ByteCursor<T: ByteIo> {
    io: T,
    cursor: u64,
}

impl<T: ByteIo> ByteCursor<T> {
    /// A cursor over `io`, positioned at the start.
    pub fn new(io: T) -> Self {
        Self { io, cursor: 0 }
    }

    /// The current cursor position (a byte offset from the start).
    pub fn position(&self) -> u64 {
        self.cursor
    }

    /// Moves the cursor to the absolute byte `position`.
    pub fn set_position(&mut self, position: u64) {
        self.cursor = position;
    }

    /// The inner io.
    pub fn get_ref(&self) -> &T {
        &self.io
    }

    /// Consumes the cursor, returning the inner io.
    pub fn into_inner(self) -> T {
        self.io
    }

    /// Moves the cursor to `offset` relative to `whence`, returning the new
    /// position. Leaves the cursor unchanged when the target falls outside
    /// `0..=len`.
    pub fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base = match whence {
            Whence::Start => 0,
            Whence::Current => i64::try_from(self.cursor).map_err(|_| IoError::OutOfBounds)?,
            Whence::End => i64::try_from(self.io.byte_len()?).map_err(|_| IoError::OutOfBounds)?,
        };
        let position = base
            .checked_add(offset)
            .filter(|resolved| *resolved >= 0)
            .ok_or(IoError::OutOfBounds)? as u64;
        if position > self.io.byte_len()? {
            return Err(IoError::OutOfBounds);
        }
        self.cursor = position;
        Ok(position)
    }

    /// Reads up to `len` bytes from the cursor, advancing it past the bytes read.
    pub fn read_bytes(&mut self, len: usize) -> Result<Buffer, IoError> {
        let bytes = self.io.positional_read_bytes(self.cursor, len)?;
        self.cursor += bytes.as_slice().len() as u64;
        Ok(bytes)
    }

    /// Writes `bytes` at the cursor, advancing it past the bytes written, and
    /// returns the number written.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        let written = self.io.positional_write_bytes(self.cursor, bytes)?;
        self.cursor += written as u64;
        Ok(written)
    }
}
