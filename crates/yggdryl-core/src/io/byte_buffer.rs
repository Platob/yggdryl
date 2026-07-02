//! The [`ByteBuffer`] in-memory resource.

use super::bits;
use super::{IOError, RawIOBase, Seekable, Whence};

/// A growable, byte-granular in-memory buffer implementing [`Seekable`] and
/// [`RawIOBase`].
///
/// Positions resolve from [`Whence::Start`] (absolute), [`Whence::Current`] (the
/// cursor) or [`Whence::End`] (the length); writes past the end grow the buffer with
/// zeroes. Its [`bit_size`](RawIOBase::bit_size) is always eight times its
/// [`byte_size`](RawIOBase::byte_size).
///
/// ```
/// use yggdryl_core::{ByteBuffer, RawIOBase, Seekable, Whence};
///
/// let mut buf = ByteBuffer::new();
/// buf.pwrite_byte_array(0, Whence::Start, &[1, 2, 3])?;
/// assert_eq!(buf.byte_size(), 3);
/// assert_eq!(buf.pread_byte_one(1, Whence::Start)?, 2);
///
/// buf.seek(1, Whence::Start)?;
/// assert_eq!(buf.pread_byte_one(0, Whence::Current)?, 2); // relative to the cursor
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ByteBuffer {
    data: Vec<u8>,
    cursor: usize,
}

impl ByteBuffer {
    /// An empty buffer with its cursor at the start.
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            cursor: 0,
        }
    }

    /// A buffer over `data`, with its cursor at the start.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self { data, cursor: 0 }
    }

    /// The buffer's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the buffer, returning its bytes.
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
            Whence::End => self.data.len() * 8 + position,
            _ => position,
        }
    }
}

impl Seekable for ByteBuffer {
    fn tell(&self) -> usize {
        self.cursor
    }

    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IOError> {
        self.cursor = self.byte_offset(position, whence);
        crate::log_event!(debug, "ByteBuffer::seek -> {}", self.cursor);
        Ok(self.cursor)
    }
}

impl RawIOBase for ByteBuffer {
    fn byte_size(&self) -> usize {
        self.data.len()
    }

    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        let start = self.byte_offset(position, whence);
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
        let start = self.bit_offset(position, whence);
        let end = bits::checked_end(start, size, self.data.len() * 8)?;
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
                len: self.data.len() * 8,
            })?;
        let needed = end.div_ceil(8);
        if needed > self.data.len() {
            self.data.resize(needed, 0);
        }
        for (i, &bit) in values.iter().enumerate() {
            bits::set_bit(&mut self.data, start + i, bit);
        }
        Ok(())
    }
}
