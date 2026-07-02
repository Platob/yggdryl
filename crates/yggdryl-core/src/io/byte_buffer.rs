//! The [`ByteBuffer`] in-memory resource.

use super::bits;
use super::{IOError, RawIOBase, Whence};

/// A growable, byte-granular in-memory buffer implementing [`RawIOBase`].
///
/// Positions resolve from [`Whence::Start`] (absolute) or [`Whence::End`] (the
/// length); writes past the end grow the buffer with zeroes. The buffer keeps no
/// cursor, so [`Whence::Current`] is measured from the start — wrap it in a
/// [`RawIOCursor`](super::RawIOCursor) for a position that advances on each access.
/// Its [`bit_size`](RawIOBase::bit_size) is always eight times its
/// [`byte_size`](RawIOBase::byte_size), and capacity tracks the underlying
/// allocation.
///
/// ```
/// use yggdryl_core::{ByteBuffer, RawIOBase, Whence};
///
/// let mut buf = ByteBuffer::new();
/// buf.pwrite_byte_array(0, Whence::Start, &[1, 2, 3])?;
/// assert_eq!(buf.byte_size(), 3);
/// assert_eq!(buf.pread_byte_one(1, Whence::Start)?, 2);
/// buf.pwrite_byte_array(0, Whence::End, &[4])?; // append at the end
///
/// buf.resize_bytes(5)?; // zero-fill up to five bytes
/// assert_eq!(buf.as_bytes(), &[1, 2, 3, 4, 0]);
/// assert!(buf.resize_byte_capacity(64)? >= 64);
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ByteBuffer {
    data: Vec<u8>,
}

impl ByteBuffer {
    /// An empty buffer.
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    /// A buffer over `data`.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// The buffer's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the buffer, returning its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    fn byte_offset(&self, position: usize, whence: Whence) -> Result<usize, IOError> {
        let base = match whence {
            Whence::End => self.data.len(),
            Whence::Current => {
                crate::log_event!(
                    warn,
                    "ByteBuffer has no cursor; Whence::Current measured from 0 — wrap it in a RawIOCursor"
                );
                0
            }
            _ => 0,
        };
        bits::offset(base, position)
    }

    fn bit_offset(&self, position: usize, whence: Whence) -> Result<usize, IOError> {
        let base = match whence {
            Whence::End => self.data.len().saturating_mul(8),
            Whence::Current => {
                crate::log_event!(
                    warn,
                    "ByteBuffer has no cursor; Whence::Current measured from 0 — wrap it in a RawIOCursor"
                );
                0
            }
            _ => 0,
        };
        bits::offset(base, position)
    }
}

impl RawIOBase for ByteBuffer {
    fn byte_size(&self) -> usize {
        self.data.len()
    }

    fn byte_capacity(&self) -> usize {
        self.data.capacity()
    }

    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        if capacity > self.data.capacity() {
            self.data.reserve_exact(capacity - self.data.len());
        } else {
            self.data.shrink_to(capacity);
        }
        crate::log_event!(
            debug,
            "ByteBuffer::resize_byte_capacity -> {}",
            self.data.capacity()
        );
        Ok(self.data.capacity())
    }

    fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
        self.data.resize(size, 0);
        crate::log_event!(debug, "ByteBuffer::resize_bytes -> {size}");
        Ok(())
    }

    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        let start = self.byte_offset(position, whence)?;
        let end = bits::checked_end(start, size, self.data.len())?;
        crate::log_event!(
            trace,
            "ByteBuffer::pread_byte_array start={start} size={size}"
        );
        Ok(self.data[start..end].to_vec())
    }

    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError> {
        if values.is_empty() {
            return Ok(());
        }
        let start = self.byte_offset(position, whence)?;
        let end = bits::offset(start, values.len())?;
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[start..end].copy_from_slice(values);
        crate::log_event!(
            trace,
            "ByteBuffer::pwrite_byte_array start={start} len={}",
            values.len()
        );
        Ok(())
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError> {
        let start = self.bit_offset(position, whence)?;
        bits::checked_end(start, size, self.data.len() * 8)?;
        Ok(bits::read_bits(&self.data, start, size))
    }

    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError> {
        if values.is_empty() {
            return Ok(());
        }
        let start = self.bit_offset(position, whence)?;
        let end = bits::offset(start, values.len())?;
        let needed = end.div_ceil(8);
        if needed > self.data.len() {
            self.data.resize(needed, 0);
        }
        bits::write_bits(&mut self.data, start, values);
        Ok(())
    }
}
