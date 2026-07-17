//! [`Heap`] — the **in-heap** source for the memory-access traits: an owned byte `Vec` with a
//! built-in read/write cursor, `Vec`-like capacity, and an addressing [`Uri`]. It is the
//! reference implementor of [`IOBase`] — the "memory" side of the layer; a memory-mapped source
//! plugs in against the same trait.

use super::cursor::cursor_methods;
use super::{IOBase, IoError, Whence};
use crate::io::uri::Uri;

/// An in-heap byte buffer with a **built-in cursor**, amortized capacity, and an addressing
/// [`Uri`] — the concrete in-memory implementor of [`IOBase`]. Its stream methods (`read` /
/// `write` / `seek` / the typed `read_byte` / `read_i32` / …) are the same ones an
/// [`IOCursor`](super::IOCursor) adds to any source; `Heap` carries them inherently so a heap is
/// usable as a cursor without wrapping. You can still wrap it — [`cursor`](IOBase::cursor) /
/// [`window`](IOBase::window) give an independent [`IOCursor`](super::IOCursor) /
/// [`IOSlice`](super::IOSlice) over any source, including a heap.
///
/// It grows like a [`Vec`]: [`with_capacity`](Heap::with_capacity) pre-allocates,
/// [`capacity`](IOBase::capacity) reports the current allocation, and
/// [`reserve`](IOBase::reserve) amortizes future writes. Its [`uri`](IOBase::uri) addresses it
/// (empty by default; set with [`with_uri`](Heap::with_uri) / [`set_uri`](Heap::set_uri)).
///
/// DESIGN: equality is over the **stored bytes only** — the cursor position and the address
/// [`Uri`] are transient/metadata, so two heaps holding the same bytes compare equal regardless
/// of where their cursors sit or how they are addressed. `Heap` is a mutable buffer (like
/// `bytearray`), so it is intentionally **not** `Hash`.
///
/// ```
/// use yggdryl_core::io::memory::{Heap, IOBase};
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
    /// The built-in cursor — bytes from the start; may sit past the end after a seek.
    position: u64,
    /// The address of this source (empty by default).
    uri: Uri,
}

impl Heap {
    /// An empty buffer with the cursor at `0`, no allocation, and no address.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty buffer that can hold `capacity` bytes before reallocating — like
    /// [`Vec::with_capacity`]. Cursor at `0`.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let h = Heap::with_capacity(64);
    /// assert!(h.is_empty());
    /// assert!(h.capacity() >= 64);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            position: 0,
            uri: Uri::default(),
        }
    }

    /// A buffer owning a **copy** of `data`, cursor at `0`.
    pub fn from_slice(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            position: 0,
            uri: Uri::default(),
        }
    }

    /// A buffer taking ownership of `data` **without copying**, cursor at `0`.
    pub fn from_vec(data: Vec<u8>) -> Self {
        Self {
            data,
            position: 0,
            uri: Uri::default(),
        }
    }

    /// The stored bytes as a slice — zero-copy.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// The owned byte vector (consuming the buffer).
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    /// An explicit copy of this heap — the cross-language name for a clone (bytes, cursor, and
    /// address all copied).
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Sets the addressing [`Uri`] in place.
    pub fn set_uri(&mut self, uri: Uri) {
        self.uri = uri;
    }

    /// Returns this heap with its addressing [`Uri`] set.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    /// use yggdryl_core::io::uri::Uri;
    ///
    /// let h = Heap::from_slice(b"x").with_uri(Uri::parse_str("mem://buf/1").unwrap());
    /// assert_eq!(h.uri().host(), Some("buf"));
    /// ```
    pub fn with_uri(mut self, uri: Uri) -> Self {
        self.uri = uri;
        self
    }

    /// The window `[offset, offset + len)` as a fresh, independent `Heap` owning a **copy** of the
    /// range (addressed from its own `0`). Errors with [`IoError::SliceOutOfBounds`] if it runs
    /// past the end. For a zero-copy *view* that borrows the source instead, use
    /// [`window`](IOBase::window), which returns an [`IOSlice`](super::IOSlice).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello world");
    /// assert_eq!(data.slice(6, 5).unwrap().as_slice(), b"world");
    /// assert!(data.slice(6, 6).is_err()); // 6 + 6 > 11
    /// ```
    pub fn slice(&self, offset: u64, len: u64) -> Result<Self, IoError> {
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

    cursor_methods!();
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

    fn uri(&self) -> Uri {
        self.uri.clone()
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

// Value equality over the stored bytes only — the cursor and address `Uri` are transient/metadata
// (see the type's DESIGN note). `Heap` is mutable, so it is deliberately not `Hash`.
impl PartialEq for Heap {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl Eq for Heap {}
