//! [`ByteBuffer`] — an immutable, cheaply-shared byte store (no cursor).

use std::hash::{Hash, Hasher};

use arrow_buffer::{Buffer, MutableBuffer};

use crate::ByteCursor;

/// An immutable, reference-counted byte store backed by an Apache Arrow `Buffer` —
/// pure storage with **no cursor**.
///
/// Positioned IO is done through a [`ByteCursor`] (`std::io::Cursor`-style): a
/// cursor holds a share of the buffer plus its own position and does the reading
/// and writing, leaving the buffer intact. Cloning a `ByteBuffer` (and creating a
/// cursor) is cheap — an Arrow refcount bump — and a cursor only copies the bytes if
/// it **writes** (copy-on-write), so the base buffer is never mutated.
///
/// The Arrow `Buffer` is the storage itself (not an optional interop layer), so
/// [`from_arrow_byte_buffer`](ByteBuffer::from_arrow_byte_buffer) /
/// [`to_arrow_byte_buffer`](ByteBuffer::to_arrow_byte_buffer) hand the allocation
/// across **zero-copy**.
///
/// Equality, hashing, and [`serialize_bytes`](ByteBuffer::serialize_bytes) concern
/// the byte content.
///
/// ```
/// use yggdryl_core::{ByteBuffer, IOBase, Whence};
///
/// let buffer = ByteBuffer::from_bytes(b"hello world");
/// let mut cursor = buffer.byte_cursor();
/// assert_eq!(cursor.pread_byte_array(5, Whence::Start).unwrap(), b"hello");
/// assert_eq!(cursor.byte_tell().unwrap(), 5); // the cursor advanced; buffer intact
/// assert_eq!(buffer.byte_size(), 11);
/// ```
#[derive(Debug, Clone)]
pub struct ByteBuffer {
    data: Buffer,
}

impl ByteBuffer {
    /// Creates an empty buffer.
    pub fn new() -> Self {
        Self::with_byte_capacity(0)
    }

    /// Creates an empty buffer that can hold `capacity` bytes without reallocating.
    pub fn with_byte_capacity(capacity: usize) -> Self {
        Self {
            data: MutableBuffer::with_capacity(capacity).into(),
        }
    }

    /// Creates an empty buffer that can hold `capacity` bits without reallocating.
    pub fn with_bit_capacity(capacity: usize) -> Self {
        Self::with_byte_capacity(capacity.div_ceil(8))
    }

    /// Creates a buffer holding a copy of `bytes`.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            data: Buffer::from(bytes),
        }
    }

    /// Creates a buffer that takes ownership of `bytes` without copying.
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            data: Buffer::from_vec(bytes),
        }
    }

    /// The number of bytes held.
    pub fn byte_size(&self) -> usize {
        self.data.len()
    }

    /// The number of bits held (`byte_size * 8`).
    pub fn bit_size(&self) -> usize {
        self.byte_size().saturating_mul(8)
    }

    /// Whether the buffer holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The number of bytes that can be held without reallocating.
    pub fn byte_capacity(&self) -> usize {
        self.data.capacity()
    }

    /// The number of bits that can be held without reallocating.
    pub fn bit_capacity(&self) -> usize {
        self.byte_capacity().saturating_mul(8)
    }

    /// Borrows the backing bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.data.as_slice()
    }

    /// Serialises the buffer to its byte content.
    ///
    /// ```
    /// use yggdryl_core::ByteBuffer;
    ///
    /// let buffer = ByteBuffer::from_bytes(b"payload");
    /// assert_eq!(buffer.serialize_bytes(), b"payload");
    /// assert_eq!(ByteBuffer::deserialize_bytes(&buffer.serialize_bytes()), buffer);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }

    /// Reconstructs a buffer from its byte content.
    pub fn deserialize_bytes(bytes: &[u8]) -> Self {
        Self::from_bytes(bytes)
    }

    /// Opens a [`ByteCursor`] over this buffer (positioned at the start). The
    /// buffer stays intact; the cursor copies only if it writes.
    ///
    /// The returned cursor implements both the raw [`IOCursor`](crate::IOCursor) and
    /// the `u8` [`TypedIOCursor`](crate::TypedIOCursor) surfaces.
    pub fn byte_cursor(&self) -> ByteCursor {
        ByteCursor::new(self.clone())
    }

    /// Opens a [`TypedCursor<T>`](crate::TypedCursor) over this buffer's bytes, for
    /// any [`IoPrimitive`](crate::IoPrimitive) element type — including the wide
    /// integers (`crate::i96` / `i128` / `crate::i256`), which have no dedicated
    /// typed buffer. `T = u8` coincides with [`byte_cursor`](ByteBuffer::byte_cursor).
    pub fn cursor<T: crate::IoPrimitive>(&self) -> crate::TypedCursor<T> {
        crate::TypedCursor::new(self.clone())
    }

    /// Opens a [`ByteSlice`](crate::ByteSlice) over the **byte** window
    /// `[offset, offset + len)` (clamped to the buffer's bytes) — a fixed-length,
    /// non-growing view. The buffer stays intact; the slice copies only if it writes.
    pub fn byte_slice(&self, offset: u64, len: usize) -> crate::ByteSlice {
        crate::ByteSlice::new(self.clone(), offset, len)
    }

    /// Opens a [`TypedSlice<T>`](crate::TypedSlice) over the **byte** window
    /// `[offset, offset + len)` for any [`IoPrimitive`](crate::IoPrimitive) element
    /// type. `T = u8` coincides with [`byte_slice`](ByteBuffer::byte_slice).
    pub fn slice<T: crate::IoPrimitive>(&self, offset: u64, len: usize) -> crate::TypedSlice<T> {
        crate::TypedSlice::new(self.clone(), offset, len)
    }

    /// Wraps an Arrow `Buffer` **zero-copy** — the buffer and this `ByteBuffer`
    /// share the same underlying allocation (reference-counted).
    ///
    /// ```
    /// use yggdryl_core::ByteBuffer;
    /// use yggdryl_core::arrow_buffer::Buffer;
    ///
    /// let arrow = Buffer::from_vec(b"payload".to_vec());
    /// let buffer = ByteBuffer::from_arrow_byte_buffer(arrow);
    /// assert_eq!(buffer.as_bytes(), b"payload");
    /// ```
    pub fn from_arrow_byte_buffer(buffer: Buffer) -> Self {
        Self { data: buffer }
    }

    /// Wraps an Arrow **bitmap** `Buffer` (LSB-first packed bits) zero-copy. Arrow
    /// bitmaps are byte buffers of packed bits, so this shares the allocation just
    /// like [`from_arrow_byte_buffer`](ByteBuffer::from_arrow_byte_buffer); the
    /// distinction is that the bytes are meant to be read as bits.
    pub fn from_arrow_bit_buffer(buffer: Buffer) -> Self {
        Self::from_arrow_byte_buffer(buffer)
    }

    /// Exports the content as an Arrow `Buffer` — **zero-copy** (an Arrow refcount
    /// bump), since the storage already is an Arrow `Buffer`.
    pub fn to_arrow_byte_buffer(&self) -> Buffer {
        self.data.clone()
    }

    /// Exports the packed bytes as an Arrow **bitmap** `Buffer` — **zero-copy**, the
    /// bit-oriented counterpart of
    /// [`to_arrow_byte_buffer`](ByteBuffer::to_arrow_byte_buffer).
    pub fn to_arrow_bit_buffer(&self) -> Buffer {
        self.data.clone()
    }

    /// Clones the backing bytes out into a growable Arrow [`MutableBuffer`] for a
    /// cursor's copy-on-write path, preserving the buffer's spare capacity so a
    /// preallocated buffer (`with_byte_capacity`) keeps its headroom and the cursor's
    /// first writes avoid reallocating.
    pub(crate) fn to_owned_mutable(&self) -> MutableBuffer {
        let bytes = self.as_bytes();
        let mut owned = MutableBuffer::with_capacity(self.data.capacity().max(bytes.len()));
        owned.extend_from_slice(bytes);
        owned
    }
}

impl Default for ByteBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// Value semantics per the byte content only.
impl PartialEq for ByteBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for ByteBuffer {}

impl Hash for ByteBuffer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}
