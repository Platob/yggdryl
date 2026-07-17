//! [`Heap`] — the **in-heap** source for the memory-access traits: an owned byte `Vec` with a
//! read/write cursor and `Vec`-like capacity. It is the reference implementor of [`IOBase`] /
//! [`IOCursor`] / [`IOSlice`] — the "memory" side of the layer; a memory-mapped source plugs in
//! against the same traits.

use super::{IOBase, IOCursor, IOSlice, IoError};

/// An in-heap byte buffer with a read/write cursor and amortized capacity — the concrete
/// in-memory implementor of the [`IOBase`] / [`IOCursor`] / [`IOSlice`] contracts.
///
/// It grows like a [`Vec`]: [`with_capacity`](Heap::with_capacity) pre-allocates,
/// [`capacity`](IOBase::capacity) reports the current allocation, and
/// [`reserve`](IOBase::reserve) amortizes future writes.
///
/// DESIGN: equality is over the **stored bytes only** — the cursor is transient I/O state (a read
/// position, like a file offset), so two heaps holding the same bytes compare equal regardless of
/// where their cursors sit. `Heap` is a mutable buffer (like `bytearray`), so it is intentionally
/// **not** `Hash`.
///
/// ```
/// use yggdryl_core::memory::{Heap, IOCursor};
///
/// let mut h = Heap::new();
/// h.write_all(b"hello ").unwrap();
/// h.write_all(b"world").unwrap();
/// assert_eq!(h.as_slice(), b"hello world");
///
/// h.rewind();
/// let mut head = [0u8; 5];
/// h.read_exact(&mut head).unwrap();
/// assert_eq!(&head, b"hello");
/// ```
#[derive(Clone, Debug, Default)]
pub struct Heap {
    data: Vec<u8>,
    /// The cursor — bytes from the start; may sit past the end after a seek.
    position: u64,
}

impl Heap {
    /// An empty buffer with the cursor at `0` and no allocation.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty buffer that can hold `capacity` bytes before reallocating — like
    /// [`Vec::with_capacity`]. Cursor at `0`.
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOBase};
    ///
    /// let h = Heap::with_capacity(64);
    /// assert!(h.is_empty());
    /// assert!(h.capacity() >= 64);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            position: 0,
        }
    }

    /// A buffer owning a **copy** of `data`, cursor at `0`.
    pub fn from_slice(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            position: 0,
        }
    }

    /// A buffer taking ownership of `data` **without copying**, cursor at `0`.
    pub fn from_vec(data: Vec<u8>) -> Self {
        Self { data, position: 0 }
    }

    /// The stored bytes as a slice — zero-copy.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// The owned byte vector (consuming the buffer).
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }
}

impl IOBase for Heap {
    fn byte_size(&self) -> u64 {
        self.data.len() as u64
    }

    fn capacity(&self) -> u64 {
        self.data.capacity() as u64
    }

    fn reserve(&mut self, additional: u64) {
        self.data.reserve(additional as usize);
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        let start = offset as usize;
        if start >= self.data.len() {
            return 0;
        }
        let n = buf.len().min(self.data.len() - start);
        buf[..n].copy_from_slice(&self.data[start..start + n]);
        n
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0;
        }
        let start = offset as usize;
        let end = start + data.len();
        if end > self.data.len() {
            self.data.resize(end, 0); // grow, zero-filling any gap
        }
        self.data[start..end].copy_from_slice(data);
        data.len()
    }
}

impl IOCursor for Heap {
    fn position(&self) -> u64 {
        self.position
    }

    fn set_position(&mut self, position: u64) {
        self.position = position;
    }
}

impl IOSlice for Heap {
    fn slice(&self, offset: u64, len: u64) -> Result<Self, IoError> {
        let available = self.data.len() as u64;
        let end = offset.checked_add(len).filter(|&e| e <= available).ok_or(
            IoError::SliceOutOfBounds {
                offset,
                len,
                available,
            },
        )?;
        Ok(Self::from_slice(&self.data[offset as usize..end as usize]))
    }
}

// Value equality over the stored bytes only — the cursor is transient (see the type's DESIGN
// note). `Heap` is mutable, so it is deliberately not `Hash`.
impl PartialEq for Heap {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl Eq for Heap {}
