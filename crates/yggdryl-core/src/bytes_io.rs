//! The in-memory [`BytesIo`] — the reference [`Io`] backend.

use crate::buffer::Buffer;
use crate::io::{Io, IoError};

/// An in-memory [`Io`] source backed by a [`Buffer`], with a read/write cursor.
///
/// Positional reads are zero-copy [`Buffer`] slices of the backing bytes; a write
/// copies the backing buffer on write. It is the reference backend that exercises
/// the [`Io`] surface and the trivial source everything else composes on.
///
/// ```
/// use yggdryl_core::{BytesIo, Io, Whence};
///
/// let mut io = BytesIo::from_bytes(b"abc".to_vec());
/// assert_eq!(io.len().unwrap(), 3);
///
/// // Append by seeking to the end first.
/// io.seek(0, Whence::End).unwrap();
/// io.write_bytes(b"def").unwrap();
/// assert_eq!(io.positional_read_bytes(0, 6).unwrap().as_slice(), b"abcdef");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BytesIo {
    data: Buffer,
    position: u64,
}

impl BytesIo {
    /// An empty, writable source.
    pub fn new() -> Self {
        Self::default()
    }

    /// A source over `bytes`, taking ownership without copying, cursor at the start.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::from_buffer(Buffer::from_vec(bytes))
    }

    /// A source sharing `buffer`'s allocation (zero-copy), cursor at the start.
    pub fn from_buffer(buffer: Buffer) -> Self {
        Self {
            data: buffer,
            position: 0,
        }
    }

    /// The backing bytes as a zero-copy [`Buffer`].
    pub fn buffer(&self) -> &Buffer {
        &self.data
    }

    /// The backing bytes, borrowed (zero-copy).
    pub fn as_slice(&self) -> &[u8] {
        self.data.as_slice()
    }
}

impl Io for BytesIo {
    fn len(&self) -> Result<u64, IoError> {
        Ok(self.data.len() as u64)
    }

    fn position(&self) -> u64 {
        self.position
    }

    fn set_position(&mut self, position: u64) {
        self.position = position;
    }

    fn positional_read_bytes(&self, offset: u64, len: usize) -> Result<Buffer, IoError> {
        let offset = usize::try_from(offset).map_err(|_| IoError::OutOfBounds)?;
        if offset > self.data.len() {
            return Err(IoError::OutOfBounds);
        }
        crate::log_event!(
            trace,
            "BytesIo::positional_read_bytes offset={offset} len={len}"
        );
        let end = offset.saturating_add(len).min(self.data.len());
        Ok(self.data.slice(offset..end))
    }

    fn positional_write_bytes(&mut self, offset: u64, bytes: &[u8]) -> Result<usize, IoError> {
        let offset = usize::try_from(offset).map_err(|_| IoError::OutOfBounds)?;
        if offset > self.data.len() {
            return Err(IoError::OutOfBounds);
        }
        crate::log_event!(
            trace,
            "BytesIo::positional_write_bytes offset={offset} len={}",
            bytes.len()
        );
        // The backing `Buffer` is immutable and shared, so a write copies it out,
        // patches the window (extending with zeros when it runs past the end) and
        // re-wraps it — reads stay zero-copy, only writes pay the copy.
        let mut vec = self.data.as_slice().to_vec();
        let end = offset + bytes.len();
        if end > vec.len() {
            vec.resize(end, 0);
        }
        vec[offset..end].copy_from_slice(bytes);
        self.data = Buffer::from_vec(vec);
        Ok(bytes.len())
    }
}
