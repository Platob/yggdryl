//! [`ByteSlice`] — a bounded, non-growing window over a [`ByteBuffer`].

use crate::{ByteBuffer, ByteCursor, IOBase, IOCursor, IOSlice, IoError, TypedIOBase, Whence};

/// A fixed-length **window** `[offset, offset + len)` over a [`ByteBuffer`]'s bytes —
/// the bounded sibling of [`ByteCursor`]. Reads and writes are confined to the window
/// (clamped at its end; it never grows), positions `0..len` are relative to the window
/// start, and a write copies the shared bytes out first (copy-on-write), leaving the
/// source buffer intact.
///
/// Implements [`IOBase`], [`IOCursor`], [`IOSlice`], and `TypedIOBase<u8>` /
/// `TypedIOSlice<u8>`. Obtain one from [`ByteBuffer::byte_slice`].
///
/// ```
/// use yggdryl_buffer::{ByteBuffer, IOBase, IOSlice, Whence};
///
/// let buffer = ByteBuffer::from_bytes(b"hello world");
/// let mut slice = buffer.byte_slice(6, 5);
/// assert_eq!(slice.pread_byte_array(100, Whence::Start).unwrap(), b"world"); // clamped
/// assert_eq!(slice.byte_size().unwrap(), 0); // fully read
/// assert_eq!(buffer.as_bytes(), b"hello world"); // source intact
/// ```
#[derive(Debug, Clone)]
pub struct ByteSlice {
    inner: ByteCursor,
    offset: u64,
    len: usize,
}

impl ByteSlice {
    /// Creates a window `[offset, offset + len)` over `buffer`, clamped to the buffer's
    /// bytes (so the window never extends past the end).
    pub fn new(buffer: ByteBuffer, offset: u64, len: usize) -> Self {
        Self::from_byte_cursor(buffer.byte_cursor(), offset, len)
    }

    /// Wraps an existing [`ByteCursor`] as a window `[offset, offset + len)` over its
    /// bytes, clamped to them.
    pub fn from_byte_cursor(inner: ByteCursor, offset: u64, len: usize) -> Self {
        let total = inner.as_bytes().len() as u64;
        let offset = offset.min(total);
        let len = len.min((total - offset) as usize);
        let mut slice = Self { inner, offset, len };
        slice.inner.set_position(offset); // window position 0
        slice
    }

    /// Borrows the window's bytes, including any writes it has made.
    pub fn as_bytes(&self) -> &[u8] {
        let start = self.offset as usize;
        &self.inner.as_bytes()[start..start + self.len]
    }

    /// Freezes the window's bytes into a new [`ByteBuffer`].
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer::from_bytes(self.as_bytes())
    }

    /// The current position relative to the window start.
    fn window_position(&self) -> u64 {
        self.inner.position().saturating_sub(self.offset)
    }
}

impl IOBase for ByteSlice {
    fn with_byte_capacity(capacity: usize) -> Self {
        // A fresh, writable window of `capacity` zeroed bytes (a slice's length is its
        // capacity — it does not grow).
        Self::new(ByteBuffer::from_vec(vec![0u8; capacity]), 0, capacity)
    }

    fn byte_tell(&self) -> Result<u64, IoError> {
        Ok(self.window_position())
    }

    fn byte_seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base: i128 = match whence {
            Whence::Start => 0,
            Whence::Current => i128::from(self.window_position()),
            Whence::End => self.len as i128,
        };
        let absolute = base + i128::from(offset);
        if !(0..=i128::from(u64::MAX)).contains(&absolute) {
            return Err(IoError::InvalidSeek { offset, whence });
        }
        let window = absolute as u64;
        let inner_abs = self.offset.saturating_add(window);
        let inner_offset =
            i64::try_from(inner_abs).map_err(|_| IoError::InvalidSeek { offset, whence })?;
        self.inner.byte_seek(inner_offset, Whence::Start)?;
        Ok(window)
    }

    fn byte_size(&self) -> Result<usize, IoError> {
        Ok(self.len.saturating_sub(self.window_position() as usize))
    }

    fn byte_capacity(&self) -> Result<usize, IoError> {
        Ok(self.len)
    }

    fn pread_byte_array(&mut self, size: usize, whence: Whence) -> Result<Vec<u8>, IoError> {
        let start = self.byte_seek(0, whence)?; // positions the inner cursor at the window start
        let available = self.len.saturating_sub(start as usize);
        self.inner
            .pread_byte_array(size.min(available), Whence::Current)
    }

    fn pread_into(&mut self, buf: &mut [u8], whence: Whence) -> Result<usize, IoError> {
        let start = self.byte_seek(0, whence)?;
        let available = self.len.saturating_sub(start as usize);
        let n = buf.len().min(available);
        self.inner.pread_into(&mut buf[..n], Whence::Current)
    }

    fn pwrite_byte_array(&mut self, data: &[u8], whence: Whence) -> Result<usize, IoError> {
        let start = self.byte_seek(0, whence)?;
        let available = self.len.saturating_sub(start as usize);
        let n = data.len().min(available); // clamp to the window — a slice never grows
        self.inner.pwrite_byte_array(&data[..n], Whence::Current)
    }
}

impl IOCursor for ByteSlice {
    fn position(&self) -> u64 {
        self.window_position()
    }

    fn set_position(&mut self, position: u64) {
        self.inner
            .set_position(self.offset.saturating_add(position));
    }
}

impl IOSlice for ByteSlice {
    fn slice_offset(&self) -> u64 {
        self.offset
    }

    fn slice_len(&self) -> usize {
        self.len
    }
}

impl TypedIOBase<u8> for ByteSlice {
    fn pread_one(&mut self, whence: Whence) -> Result<u8, IoError> {
        let bytes = self.pread_byte_array(1, whence)?;
        bytes.first().copied().ok_or(IoError::UnexpectedEof {
            needed: 1,
            available: 0,
        })
    }

    fn pwrite_one(&mut self, value: u8, whence: Whence) -> Result<usize, IoError> {
        self.pwrite_byte_array(&[value], whence)
    }

    fn pread_array(&mut self, count: usize, whence: Whence) -> Result<Vec<u8>, IoError> {
        self.pread_byte_array(count, whence)
    }

    fn pwrite_array(&mut self, data: &[u8], whence: Whence) -> Result<usize, IoError> {
        self.pwrite_byte_array(data, whence)
    }
}
