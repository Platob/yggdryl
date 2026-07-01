//! The [`ByteSlice`] bounded window over a [`ByteIo`].

use crate::buffer::Buffer;
use crate::byte_io::{ByteIo, IoError};

/// A bounded window `[start, end)` over an inner [`ByteIo`], itself a [`ByteIo`].
///
/// Offsets are relative to the window `start`, and reads and writes are clamped to
/// the window `end`, so a slice never escapes its bounds. It copies nothing itself
/// — [`positional_read_bytes`](ByteIo::positional_read_bytes) delegates to the inner
/// io, so a slice of a [`Buffer`] is a zero-copy view.
///
/// ```
/// use yggdryl_core::{Buffer, ByteIo, ByteSlice};
///
/// let buf = Buffer::from_vec(b"hello world".to_vec());
/// let world = ByteSlice::new(buf, 6, 11).unwrap();
/// assert_eq!(world.byte_len().unwrap(), 5);
/// assert_eq!(world.positional_read_bytes(0, 9).unwrap().as_slice(), b"world");
/// ```
#[derive(Clone, Debug)]
pub struct ByteSlice<T: ByteIo> {
    io: T,
    start: u64,
    end: u64,
}

impl<T: ByteIo> ByteSlice<T> {
    /// A window over `io` spanning the absolute byte range `[start, end)`. Errors
    /// [`OutOfBounds`](IoError::OutOfBounds) if `start > end` or `end` runs past the
    /// inner length.
    pub fn new(io: T, start: u64, end: u64) -> Result<Self, IoError> {
        if start > end || end > io.byte_len()? {
            return Err(IoError::OutOfBounds);
        }
        Ok(Self { io, start, end })
    }

    /// The window's absolute start offset in the inner io.
    pub fn start(&self) -> u64 {
        self.start
    }

    /// The window's absolute end offset in the inner io.
    pub fn end(&self) -> u64 {
        self.end
    }

    /// The inner io.
    pub fn get_ref(&self) -> &T {
        &self.io
    }

    /// Consumes the slice, returning the inner io.
    pub fn into_inner(self) -> T {
        self.io
    }
}

impl<T: ByteIo> ByteIo for ByteSlice<T> {
    fn byte_len(&self) -> Result<u64, IoError> {
        Ok(self.end - self.start)
    }

    fn positional_read_bytes(&self, offset: u64, len: usize) -> Result<Buffer, IoError> {
        let window = self.end - self.start;
        if offset > window {
            return Err(IoError::OutOfBounds);
        }
        // Clamp the read to what remains inside the window.
        let len = (len as u64).min(window - offset) as usize;
        self.io.positional_read_bytes(self.start + offset, len)
    }

    fn positional_write_bytes(&mut self, offset: u64, bytes: &[u8]) -> Result<usize, IoError> {
        let window = self.end - self.start;
        if offset > window {
            return Err(IoError::OutOfBounds);
        }
        // A window has a fixed end, so write only what fits inside it.
        let fits = (bytes.len() as u64).min(window - offset) as usize;
        self.io
            .positional_write_bytes(self.start + offset, &bytes[..fits])
    }
}
