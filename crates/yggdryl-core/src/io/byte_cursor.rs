//! [`ByteCursor`] — a positioned cursor over a [`ByteBuffer`].

use arrow_buffer::MutableBuffer;

use crate::{ByteBuffer, IOBase, IOCursor, IoError, TypedIOBase, Whence};

/// The cursor's backing: a read-only share of the source buffer until the first
/// write copies it out (copy-on-write), so the source [`ByteBuffer`] stays intact.
/// The owned copy is a growable Arrow [`MutableBuffer`], so writes stay within the
/// Arrow allocator.
#[derive(Debug)]
enum Backing {
    Shared(ByteBuffer),
    Owned(MutableBuffer),
}

impl Clone for Backing {
    fn clone(&self) -> Self {
        match self {
            // Arrow `MutableBuffer` is not `Clone`, so copy its bytes into a fresh one
            // (preserving spare capacity). `Shared` is a cheap Arrow refcount bump.
            Self::Shared(buffer) => Self::Shared(buffer.clone()),
            Self::Owned(bytes) => {
                let mut copy = MutableBuffer::with_capacity(bytes.capacity().max(bytes.len()));
                copy.extend_from_slice(bytes.as_slice());
                Self::Owned(copy)
            }
        }
    }
}

/// A positioned, advancing cursor over a [`ByteBuffer`] — the `std::io::Cursor`
/// analogue. Reads and writes happen at the cursor and advance it; a write copies
/// the shared bytes out first, leaving the source buffer untouched.
///
/// Implements [`IOBase`] and [`IOCursor`], and `TypedIOBase<u8>` /
/// `TypedIOCursor<u8>`. Obtain one from [`ByteBuffer::byte_cursor`].
///
/// ```
/// use yggdryl_core::{ByteBuffer, IOBase, Whence};
///
/// let buffer = ByteBuffer::from_bytes(b"abcdef");
/// let mut cursor = buffer.byte_cursor();
/// assert_eq!(cursor.pread_byte_array(3, Whence::Start).unwrap(), b"abc");
/// cursor.pwrite_byte_array(b"XYZ", Whence::Current).unwrap(); // copy-on-write
/// assert_eq!(buffer.as_bytes(), b"abcdef"); // source intact
/// ```
#[derive(Debug, Clone)]
pub struct ByteCursor {
    backing: Backing,
    position: u64,
}

// `Backing`'s `Clone` (above) deep-copies the owned `MutableBuffer`, so the derived
// `ByteCursor: Clone` works even though Arrow's `MutableBuffer` is not itself `Clone`.

impl ByteCursor {
    /// Creates a cursor over `buffer`, positioned at the start.
    pub fn new(buffer: ByteBuffer) -> Self {
        Self {
            backing: Backing::Shared(buffer),
            position: 0,
        }
    }

    /// Borrows the cursor's current bytes, including any writes it has made.
    pub fn as_bytes(&self) -> &[u8] {
        self.slice()
    }

    /// Adjusts the backing allocation to hold `capacity` bytes, returning the new
    /// capacity. Growing **reserves** headroom (no reallocation until it is exceeded);
    /// a `capacity` **below the current length truncates the content** to `capacity`
    /// (reducing the inner buffer) and clamps the cursor to the new end.
    ///
    /// Because this can mutate the content it materialises the copy-on-write owned
    /// buffer, leaving any source [`ByteBuffer`] intact.
    ///
    /// ```
    /// use yggdryl_core::{ByteBuffer, IOBase, Whence};
    ///
    /// let mut cursor = ByteBuffer::from_bytes(b"abcdef").byte_cursor();
    /// cursor.set_byte_capacity(3); // below the length -> reduce the inner buffer
    /// assert_eq!(cursor.as_bytes(), b"abc");
    ///
    /// cursor.set_byte_capacity(64); // above -> reserve headroom, content unchanged
    /// assert_eq!(cursor.as_bytes(), b"abc");
    /// assert!(cursor.byte_capacity().unwrap() >= 64);
    /// ```
    pub fn set_byte_capacity(&mut self, capacity: usize) -> usize {
        let bytes = self.owned_mut();
        if capacity < bytes.len() {
            bytes.truncate(capacity);
        } else if capacity > bytes.capacity() {
            bytes.reserve(capacity - bytes.len());
        }
        let len = bytes.len() as u64;
        let new_capacity = bytes.capacity();
        if self.position > len {
            self.position = len;
        }
        new_capacity
    }

    /// Adjusts the backing allocation to hold `capacity` bits (rounded up to whole
    /// bytes), returning the new byte capacity. See
    /// [`set_byte_capacity`](ByteCursor::set_byte_capacity).
    pub fn set_bit_capacity(&mut self, capacity: usize) -> usize {
        self.set_byte_capacity(capacity.div_ceil(8))
    }

    /// Freezes the cursor's current bytes into a new [`ByteBuffer`].
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        match &self.backing {
            Backing::Shared(buffer) => buffer.clone(),
            // Freeze the owned `MutableBuffer` into an immutable Arrow `Buffer`.
            Backing::Owned(bytes) => {
                ByteBuffer::from_arrow_byte_buffer(arrow_buffer::Buffer::from(bytes.as_slice()))
            }
        }
    }

    /// Borrows the backing bytes (shared or owned).
    fn slice(&self) -> &[u8] {
        match &self.backing {
            Backing::Shared(buffer) => buffer.as_bytes(),
            Backing::Owned(bytes) => bytes.as_slice(),
        }
    }

    /// Materialises an owned, mutable copy (copy-on-write) and returns it.
    fn owned_mut(&mut self) -> &mut MutableBuffer {
        if let Backing::Shared(buffer) = &self.backing {
            self.backing = Backing::Owned(buffer.to_owned_mutable());
        }
        match &mut self.backing {
            Backing::Owned(bytes) => bytes,
            Backing::Shared(_) => unreachable!("just converted to owned"),
        }
    }

    /// Resolves `whence` (offset 0) to an in-bounds start index.
    fn resolve(&self, whence: Whence) -> Result<usize, IoError> {
        let len = self.slice().len() as u64;
        let absolute = whence.resolve(0, self.position, len)?;
        usize::try_from(absolute).map_err(|_| IoError::InvalidSeek { offset: 0, whence })
    }
}

impl IOBase for ByteCursor {
    fn with_byte_capacity(capacity: usize) -> Self {
        Self {
            backing: Backing::Owned(MutableBuffer::with_capacity(capacity)),
            position: 0,
        }
    }

    fn byte_tell(&self) -> Result<u64, IoError> {
        Ok(self.position)
    }

    fn byte_seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let len = self.slice().len() as u64;
        self.position = whence.resolve(offset, self.position, len)?;
        Ok(self.position)
    }

    fn byte_size(&self) -> Result<usize, IoError> {
        // The bytes remaining from the current position to the end (0 if the cursor
        // is at or past the end); the total extent lives in the backing slice.
        Ok(self.slice().len().saturating_sub(self.position as usize))
    }

    fn byte_capacity(&self) -> Result<usize, IoError> {
        Ok(match &self.backing {
            Backing::Shared(buffer) => buffer.byte_capacity(),
            Backing::Owned(bytes) => bytes.capacity(),
        })
    }

    fn pread_byte_array(&mut self, size: usize, whence: Whence) -> Result<Vec<u8>, IoError> {
        let start = self.resolve(whence)?;
        let data = self.slice();
        let end = start.saturating_add(size).min(data.len());
        // `start > end` when the cursor is past the end; `get` yields `None` -> empty.
        let out = data.get(start..end).unwrap_or(&[]).to_vec();
        // Advance by what was actually read (never jump backward when past the end).
        self.position = (start + out.len()) as u64;
        Ok(out)
    }

    fn pread_into(&mut self, buf: &mut [u8], whence: Whence) -> Result<usize, IoError> {
        let start = self.resolve(whence)?;
        let data = self.slice();
        let end = start.saturating_add(buf.len()).min(data.len());
        // `start > end` when the cursor is past the end; `get` yields `None` -> empty,
        // so the read returns 0 instead of panicking on `data[start..end]`.
        let chunk = data.get(start..end).unwrap_or(&[]);
        let n = chunk.len();
        buf[..n].copy_from_slice(chunk);
        self.position = (start + n) as u64;
        Ok(n)
    }

    fn pwrite_byte_array(&mut self, data: &[u8], whence: Whence) -> Result<usize, IoError> {
        let start = self.resolve(whence)?;
        let end = start
            .checked_add(data.len())
            .ok_or(IoError::InvalidSeek { offset: 0, whence })?;
        let bytes = self.owned_mut();
        if end > bytes.len() {
            bytes.resize(end, 0);
        }
        bytes.as_slice_mut()[start..end].copy_from_slice(data);
        self.position = end as u64;
        Ok(data.len())
    }
}

impl IOCursor for ByteCursor {
    fn position(&self) -> u64 {
        self.position
    }

    fn set_position(&mut self, position: u64) {
        self.position = position;
    }
}

impl TypedIOBase<u8> for ByteCursor {
    fn pread_one(&mut self, whence: Whence) -> Result<u8, IoError> {
        let start = self.resolve(whence)?;
        let byte = self
            .slice()
            .get(start)
            .copied()
            .ok_or(IoError::UnexpectedEof {
                needed: 1,
                available: 0,
            })?;
        self.position = (start + 1) as u64;
        Ok(byte)
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
