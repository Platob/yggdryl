//! The [`BitSlice`] bounded bit-window over a [`ByteIo`].

use crate::bit_io::BitIo;
use crate::byte_io::{ByteIo, IoError};

/// A bounded bit-window `[start, end)` over an inner [`ByteIo`], addressed in bits.
///
/// It is the bit analog of [`ByteSlice`](crate::ByteSlice): offsets are relative to
/// the window `start`, and reads and writes are clamped to the window `end`, so a
/// slice never escapes its bounds. Because its bounds are bit-granular (not
/// byte-aligned) it is a [`BitIo`], not a [`ByteIo`] — it delegates each bit to the
/// inner io and copies nothing itself.
///
/// ```
/// use yggdryl_core::{BitIo, BitSlice, Buffer};
///
/// // byte 0 = 0b1010_0101.
/// let buf = Buffer::from_vec(vec![0b1010_0101]);
/// let high = BitSlice::new(buf, 4, 8).unwrap(); // bits 4..8
/// assert_eq!(high.bit_len(), 4);
/// // Bit 0 of the window is bit 4 of the buffer.
/// assert!(!high.read_bit(0).unwrap());
/// assert!(high.read_bit(1).unwrap());
/// ```
#[derive(Clone, Debug)]
pub struct BitSlice<T: ByteIo> {
    io: T,
    start: u64,
    end: u64,
}

impl<T: ByteIo> BitSlice<T> {
    /// A window over `io` spanning the absolute bit range `[start, end)`. Errors
    /// [`OutOfBounds`](IoError::OutOfBounds) if `start > end` or `end` runs past the
    /// inner bit length (`byte_len * 8`).
    pub fn new(io: T, start: u64, end: u64) -> Result<Self, IoError> {
        if start > end || end > io.byte_len()?.saturating_mul(8) {
            return Err(IoError::OutOfBounds);
        }
        Ok(Self { io, start, end })
    }

    /// The window's absolute start bit offset in the inner io.
    pub fn start(&self) -> u64 {
        self.start
    }

    /// The window's absolute end bit offset in the inner io.
    pub fn end(&self) -> u64 {
        self.end
    }

    /// The number of bits in the window.
    pub fn bit_len(&self) -> u64 {
        self.end - self.start
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

impl<T: ByteIo> BitIo for BitSlice<T> {
    fn read_bit(&self, offset: u64) -> Result<bool, IoError> {
        if offset >= self.bit_len() {
            return Err(IoError::OutOfBounds);
        }
        self.io.read_bit(self.start + offset)
    }

    fn write_bit(&mut self, offset: u64, value: bool) -> Result<(), IoError> {
        if offset >= self.bit_len() {
            return Err(IoError::OutOfBounds);
        }
        self.io.write_bit(self.start + offset, value)
    }
}
