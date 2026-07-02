//! The [`BitBuffer`] in-memory resource.

use super::bits;
use super::{IOError, RawIOBase, Seekable, Whence};

/// A growable, bit-granular in-memory buffer implementing [`Seekable`] and
/// [`RawIOBase`].
///
/// Unlike [`ByteBuffer`](super::ByteBuffer), it tracks an exact bit length, so its
/// [`bit_size`](RawIOBase::bit_size) need not be a multiple of eight and its
/// [`byte_size`](RawIOBase::byte_size) rounds up to the enclosing byte. Bits are
/// MSB-first; writes past the end grow the buffer with zeroes.
///
/// ```
/// use yggdryl_core::{BitBuffer, RawIOBase, Whence};
///
/// let mut buf = BitBuffer::new();
/// buf.pwrite_bit_array(0, Whence::Start, &[true, false, true])?;
/// assert_eq!(buf.bit_size(), 3);
/// assert_eq!(buf.byte_size(), 1); // three bits still occupy one byte
/// assert_eq!(buf.pread_bit_array(0, Whence::Start, 3)?, vec![true, false, true]);
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BitBuffer {
    // Invariant: `data.len() == bit_len.div_ceil(8)`.
    data: Vec<u8>,
    bit_len: usize,
    cursor: usize,
}

impl BitBuffer {
    /// An empty buffer with its cursor at the start.
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bit_len: 0,
            cursor: 0,
        }
    }

    /// A buffer over `data` (a whole number of bytes), with its cursor at the start.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        let bit_len = data.len() * 8;
        Self {
            data,
            bit_len,
            cursor: 0,
        }
    }

    /// The buffer's backing bytes (the final byte may hold trailing padding bits).
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the buffer, returning its backing bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    fn byte_offset(&self, position: usize, whence: Whence) -> usize {
        match whence {
            Whence::Current => self.cursor + position,
            Whence::End => self.data.len() + position,
            _ => position,
        }
    }

    fn bit_offset(&self, position: usize, whence: Whence) -> usize {
        match whence {
            Whence::Current => self.cursor * 8 + position,
            Whence::End => self.bit_len + position,
            _ => position,
        }
    }
}

impl Seekable for BitBuffer {
    fn tell(&self) -> usize {
        self.cursor
    }

    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IOError> {
        self.cursor = self.byte_offset(position, whence);
        crate::log_event!(debug, "BitBuffer::seek -> {}", self.cursor);
        Ok(self.cursor)
    }
}

impl RawIOBase for BitBuffer {
    fn byte_size(&self) -> usize {
        self.data.len()
    }

    fn bit_size(&self) -> usize {
        self.bit_len
    }

    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        let start = self.byte_offset(position, whence);
        let end = bits::checked_end(start, size, self.data.len())?;
        Ok(self.data[start..end].to_vec())
    }

    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError> {
        let start = self.byte_offset(position, whence);
        let end = start
            .checked_add(values.len())
            .ok_or(IOError::OutOfBounds {
                offset: start,
                len: self.data.len(),
            })?;
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[start..end].copy_from_slice(values);
        self.bit_len = self.bit_len.max(end * 8);
        Ok(())
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError> {
        let start = self.bit_offset(position, whence);
        let end = bits::checked_end(start, size, self.bit_len)?;
        Ok((start..end).map(|i| bits::get_bit(&self.data, i)).collect())
    }

    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError> {
        let start = self.bit_offset(position, whence);
        let end = start
            .checked_add(values.len())
            .ok_or(IOError::OutOfBounds {
                offset: start,
                len: self.bit_len,
            })?;
        let needed = end.div_ceil(8);
        if needed > self.data.len() {
            self.data.resize(needed, 0);
        }
        for (i, &bit) in values.iter().enumerate() {
            bits::set_bit(&mut self.data, start + i, bit);
        }
        self.bit_len = self.bit_len.max(end);
        Ok(())
    }
}
