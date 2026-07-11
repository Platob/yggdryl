//! [`BooleanBuffer`] — a contiguous, bit-packed buffer of `bool` values.

use core::fmt;

use arrow_buffer::Buffer;
use yggdryl_core::ByteBuffer;
use yggdryl_field::BooleanField;
use yggdryl_http::{Headers, HeadersBased};

use crate::BufferError;

/// The mask selecting the valid low bits of a bit buffer's final byte (`0xFF` when
/// the length is a whole number of bytes, so the last byte is fully used).
const fn trailing_mask(len: usize) -> u8 {
    match len % 8 {
        0 => 0xFF,
        rem => (1u8 << rem) - 1,
    }
}

/// An immutable, cheaply-shared buffer of `bool` values, **bit-packed** LSB-first
/// (8 values per byte) exactly like an Arrow validity bitmap — it *is* an Arrow
/// [`BooleanBuffer`](arrow_buffer::BooleanBuffer).
///
/// Cloning shares the allocation. Unused high bits of the final byte are kept zero,
/// so equality and hashing (by bit content) agree with
/// [`serialize_bytes`](BooleanBuffer::serialize_bytes) — two buffers are equal iff
/// their `serialize_bytes` are equal. It wraps an Arrow `BooleanBuffer` **zero-copy**
/// (see [`from_arrow`](BooleanBuffer::from_arrow)).
///
/// ```
/// use yggdryl_buffer::BooleanBuffer;
///
/// let buffer = BooleanBuffer::from_bits(&[true, false, true, true]);
/// assert_eq!(buffer.len(), 4);
/// assert_eq!(buffer.get(2), Some(true));
/// assert_eq!(buffer.count_set_bits(), 3);
/// let bytes = buffer.serialize_bytes();
/// assert_eq!(BooleanBuffer::deserialize_bytes(&bytes).unwrap(), buffer);
/// ```
#[derive(Clone)]
pub struct BooleanBuffer {
    // Always byte-aligned (offset 0), so `as_bytes` is a straight slice.
    data: arrow_buffer::BooleanBuffer,
    headers: Option<Headers>,
}

impl BooleanBuffer {
    /// Wraps packed `bytes` (already `ceil(len / 8)` long, trailing bits zeroed) as a
    /// buffer of `len` bits.
    fn from_packed(bytes: Vec<u8>, len: usize) -> Self {
        Self {
            data: arrow_buffer::BooleanBuffer::new(Buffer::from_vec(bytes), 0, len),
            headers: None,
        }
    }

    /// Builds the matching [`BooleanField`] named `name` (nullable `nullable`), carrying
    /// this buffer's headers.
    pub fn field(&self, name: impl Into<String>, nullable: bool) -> BooleanField {
        let field = BooleanField::new(name, nullable);
        match &self.headers {
            Some(headers) => HeadersBased::with_headers(field, headers.clone()),
            None => field,
        }
    }

    /// Creates an empty buffer.
    pub fn new() -> Self {
        Self::from_packed(Vec::new(), 0)
    }

    /// Packs `bits` LSB-first into a new buffer.
    pub fn from_bits(bits: &[bool]) -> Self {
        let len = bits.len();
        let mut bytes = vec![0u8; len.div_ceil(8)];
        for (index, &bit) in bits.iter().enumerate() {
            if bit {
                bytes[index / 8] |= 1 << (index % 8);
            }
        }
        Self::from_packed(bytes, len)
    }

    /// Wraps `bytes` (LSB-first packed bits) as a buffer of `len` bits, zeroing any
    /// unused high bits of the final byte.
    ///
    /// # Errors
    /// [`BufferError::InvalidBitLength`] if `bytes.len()` is not `ceil(len / 8)`.
    pub fn from_bytes(bytes: &[u8], len: usize) -> Result<Self, BufferError> {
        let expected = len.div_ceil(8);
        if bytes.len() != expected {
            return Err(BufferError::InvalidBitLength {
                bytes: bytes.len(),
                expected,
                len,
            });
        }
        let mut owned = bytes.to_vec();
        if expected > 0 {
            owned[expected - 1] &= trailing_mask(len);
        }
        Ok(Self::from_packed(owned, len))
    }

    /// The number of bits (boolean values) held.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the buffer holds no bits.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The boolean at `index`, or `None` if out of bounds.
    pub fn get(&self, index: usize) -> Option<bool> {
        if index >= self.len() {
            return None;
        }
        Some(self.data.value(index))
    }

    /// Unpacks the bits into an owned `Vec<bool>`.
    pub fn to_vec(&self) -> Vec<bool> {
        (0..self.len())
            .map(|index| self.data.value(index))
            .collect()
    }

    /// Borrows the packed bytes (LSB-first, `ceil(len / 8)` of them).
    pub fn as_bytes(&self) -> &[u8] {
        &self.data.values()[..self.len().div_ceil(8)]
    }

    /// Counts the set (`true`) bits.
    pub fn count_set_bits(&self) -> usize {
        self.data.count_set_bits()
    }

    /// Serialises to an 8-byte little-endian bit length followed by the packed bytes.
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let len = self.len();
        let n = len.div_ceil(8);
        let mut out = Vec::with_capacity(8 + n);
        out.extend_from_slice(&(len as u64).to_le_bytes());
        if n > 0 {
            let bytes = self.as_bytes();
            out.extend_from_slice(&bytes[..n - 1]);
            out.push(bytes[n - 1] & trailing_mask(len));
        }
        out
    }

    /// Reconstructs a buffer from [`serialize_bytes`](BooleanBuffer::serialize_bytes).
    ///
    /// # Errors
    /// [`BufferError::Truncated`] if shorter than the 8-byte header, or
    /// [`BufferError::InvalidBitLength`] if the packed bytes don't match the header.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, BufferError> {
        if bytes.len() < 8 {
            return Err(BufferError::Truncated {
                needed: 8,
                available: bytes.len(),
            });
        }
        let len = u64::from_le_bytes(bytes[..8].try_into().expect("8 bytes")) as usize;
        Self::from_bytes(&bytes[8..], len)
    }

    /// Freezes the packed bytes into a [`ByteBuffer`] (the bit length is not carried;
    /// pair it with [`len`](BooleanBuffer::len) or use `serialize_bytes`).
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        let len = self.len();
        let n = len.div_ceil(8);
        if n == 0 {
            return ByteBuffer::new();
        }
        let mut bytes = self.as_bytes()[..n].to_vec();
        bytes[n - 1] &= trailing_mask(len);
        ByteBuffer::from_vec(bytes)
    }

    /// Reads `len` packed bits from a [`ByteBuffer`].
    ///
    /// # Errors
    /// As [`from_bytes`](BooleanBuffer::from_bytes).
    pub fn from_byte_buffer(buffer: &ByteBuffer, len: usize) -> Result<Self, BufferError> {
        Self::from_bytes(buffer.as_bytes(), len)
    }

    /// Wraps an Arrow `BooleanBuffer` **zero-copy** when it starts on a byte boundary
    /// (offset 0), otherwise materialises the offset bits into a fresh buffer so the
    /// wrapped bitmap is always byte-aligned.
    pub fn from_arrow(buffer: arrow_buffer::BooleanBuffer) -> Self {
        if buffer.offset() == 0 {
            Self {
                data: buffer,
                headers: None,
            }
        } else {
            let len = buffer.len();
            let mut bytes = vec![0u8; len.div_ceil(8)];
            for index in 0..len {
                if buffer.value(index) {
                    bytes[index / 8] |= 1 << (index % 8);
                }
            }
            Self::from_packed(bytes, len)
        }
    }

    /// Exports the bits as an Arrow `BooleanBuffer` — **zero-copy** (an Arrow refcount
    /// bump), since the storage already is an Arrow `BooleanBuffer`.
    pub fn to_arrow(&self) -> arrow_buffer::BooleanBuffer {
        self.data.clone()
    }
}

impl Default for BooleanBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for BooleanBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BooleanBuffer")
            .field("len", &self.len())
            .finish()
    }
}

// Value semantics by bit content (the final byte masked to its valid bits), so two
// buffers are equal iff their `serialize_bytes` are equal (`CLAUDE.md` rule 7).
impl PartialEq for BooleanBuffer {
    fn eq(&self, other: &Self) -> bool {
        let len = self.len();
        if len != other.len() {
            return false;
        }
        let n = len.div_ceil(8);
        if n == 0 {
            return true;
        }
        let (a, b) = (self.as_bytes(), other.as_bytes());
        if a[..n - 1] != b[..n - 1] {
            return false;
        }
        let mask = trailing_mask(len);
        (a[n - 1] & mask) == (b[n - 1] & mask)
    }
}

impl Eq for BooleanBuffer {}

impl core::hash::Hash for BooleanBuffer {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        let len = self.len();
        len.hash(state);
        let n = len.div_ceil(8);
        if n > 0 {
            let bytes = self.as_bytes();
            bytes[..n - 1].hash(state);
            (bytes[n - 1] & trailing_mask(len)).hash(state);
        }
    }
}

// Header get / add / update / delete + the `with_headers` builder come from the one
// shared trait; the buffer only supplies the slot.
impl HeadersBased for BooleanBuffer {
    fn headers(&self) -> Option<&Headers> {
        self.headers.as_ref()
    }

    fn headers_mut(&mut self) -> &mut Option<Headers> {
        &mut self.headers
    }
}
