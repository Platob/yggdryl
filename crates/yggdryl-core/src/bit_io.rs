//! The [`BitIo`] bit-addressed trait, layered on [`ByteIo`].

use crate::byte_io::{ByteIo, IoError};

/// Bit-level random access, layered on [`ByteIo`]. A bit is addressed by its
/// absolute bit offset: bit `n` is bit `n % 8` — counting from the
/// **least-significant** bit — of byte `n / 8`.
///
/// It is blanket-implemented for every [`ByteIo`], so any byte source — a
/// [`Buffer`](crate::Buffer), a [`ByteSlice`](crate::ByteSlice), a
/// [`ByteCursor`](crate::ByteCursor)'s inner io — reads and writes bits for free.
///
/// ```
/// use yggdryl_core::{BitIo, Buffer};
///
/// // 0b0000_0001 — only bit 0 of byte 0 is set.
/// let mut buf = Buffer::from_vec(vec![0b0000_0001]);
/// assert!(buf.read_bit(0).unwrap());
/// assert!(!buf.read_bit(1).unwrap());
///
/// // Setting bit 3 leaves the other bits of the byte untouched.
/// buf.write_bit(3, true).unwrap();
/// assert_eq!(buf.as_slice(), &[0b0000_1001]);
/// ```
pub trait BitIo {
    /// Reads the bit at absolute bit `offset` — bit `offset % 8` (from the LSB) of
    /// byte `offset / 8`. Errors [`OutOfBounds`](IoError::OutOfBounds) if the byte
    /// is past the end.
    fn read_bit(&self, offset: u64) -> Result<bool, IoError>;

    /// Sets (`value == true`) or clears the bit at absolute bit `offset`, leaving
    /// the other bits of its byte untouched.
    fn write_bit(&mut self, offset: u64, value: bool) -> Result<(), IoError>;
}

impl<T: ByteIo + ?Sized> BitIo for T {
    fn read_bit(&self, offset: u64) -> Result<bool, IoError> {
        let bit = (offset % 8) as u32;
        let byte = self
            .positional_read_bytes(offset / 8, 1)?
            .as_slice()
            .first()
            .copied()
            .ok_or(IoError::OutOfBounds)?;
        Ok((byte >> bit) & 1 == 1)
    }

    fn write_bit(&mut self, offset: u64, value: bool) -> Result<(), IoError> {
        let byte_index = offset / 8;
        let bit = (offset % 8) as u32;
        let mut byte = self
            .positional_read_bytes(byte_index, 1)?
            .as_slice()
            .first()
            .copied()
            .ok_or(IoError::OutOfBounds)?;
        if value {
            byte |= 1 << bit;
        } else {
            byte &= !(1 << bit);
        }
        self.positional_write_bytes(byte_index, &[byte])?;
        Ok(())
    }
}
