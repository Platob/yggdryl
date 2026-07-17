//! [`Heap`] — the **in-heap** source for the memory-access traits: an owned byte `Vec` with a
//! built-in read/write cursor and `Vec`-like capacity. It is the reference implementor of
//! [`IOBase`] — the "memory" side of the layer; a memory-mapped source plugs in against the
//! same trait.

use super::cursor::cursor_methods;
use super::{IOBase, IoError, Whence};
use crate::headers::Headers;
use crate::io::{IOKind, IOMode};

/// An in-heap byte buffer with a **built-in cursor** and amortized capacity — the concrete
/// in-memory implementor of [`IOBase`]. Its stream methods (`read` / `write` / `seek` / the
/// typed `read_byte` / `read_i32` / …) are the same ones an [`IOCursor`](super::IOCursor) adds
/// to any source; `Heap` carries them inherently so a heap is usable as a cursor without
/// wrapping. You can still wrap it — [`cursor`](IOBase::cursor) / [`window`](IOBase::window)
/// give an independent [`IOCursor`](super::IOCursor) / [`IOSlice`](super::IOSlice) over any
/// source, including a heap.
///
/// It grows like a [`Vec`]: [`with_capacity`](Heap::with_capacity) pre-allocates,
/// [`capacity`](IOBase::capacity) reports the current allocation, and
/// [`reserve`](IOBase::reserve) amortizes future writes.
///
/// DESIGN: a heap stores **no address** — its [`uri`](IOBase::uri) is always the trait's
/// stable synthetic `mem://heap` (an anonymous in-memory buffer has no other identity; a
/// source with a real address, like a future file source, stores and reports its own).
///
/// DESIGN: equality is over the **stored bytes only** — the cursor position, [`Headers`], and
/// [`IOMode`] are transient/metadata, so two heaps holding the same bytes compare equal
/// regardless of cursor or metadata. `Heap` is a mutable buffer (like `bytearray`), so it is
/// intentionally **not** `Hash`.
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
#[derive(Clone, Debug)]
pub struct Heap {
    data: Vec<u8>,
    /// The built-in cursor — bytes from the start; may sit past the end after a seek.
    position: u64,
    /// The source's metadata map — initialized **empty** (an empty `Headers` allocates
    /// nothing, so an untouched heap stays allocation-free).
    headers: Headers,
    /// How this source may be accessed (`ReadWrite` by default — it is in-memory).
    mode: IOMode,
}

impl Default for Heap {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            position: 0,
            headers: Headers::new(),
            mode: IOMode::ReadWrite,
        }
    }
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
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let h = Heap::with_capacity(64);
    /// assert!(h.is_empty());
    /// assert!(h.capacity() >= 64);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            ..Self::default()
        }
    }

    /// A buffer owning a **copy** of `data`, cursor at `0`.
    pub fn from_slice(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            ..Self::default()
        }
    }

    /// A buffer taking ownership of `data` **without copying**, cursor at `0`.
    pub fn from_vec(data: Vec<u8>) -> Self {
        Self {
            data,
            ..Self::default()
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

    /// An explicit copy of this heap — the cross-language name for a clone (bytes, cursor,
    /// headers, and mode all copied).
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Sets the access [`IOMode`] in place.
    pub fn set_mode(&mut self, mode: IOMode) {
        self.mode = mode;
    }

    /// Returns this heap with its access [`IOMode`] set.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    /// use yggdryl_core::io::IOMode;
    ///
    /// let h = Heap::new().with_mode(IOMode::Read);
    /// assert_eq!(h.mode(), IOMode::Read);
    /// ```
    pub fn with_mode(mut self, mode: IOMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets the whole [`Headers`] metadata map in place (use
    /// [`headers_mut`](IOBase::headers_mut) for entry-level edits).
    pub fn set_headers(&mut self, headers: Headers) {
        self.headers = headers;
    }

    /// Returns this heap with its [`Headers`] metadata replaced.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    /// use yggdryl_core::headers::Headers;
    ///
    /// let h = Heap::new().with_headers(Headers::new().with("Content-Type", "text/plain"));
    /// assert_eq!(h.headers().content_type(), Some("text/plain"));
    /// ```
    pub fn with_headers(mut self, headers: Headers) -> Self {
        self.set_headers(headers);
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

    // `uri()` is deliberately NOT overridden: a heap stores no address, so every heap reports
    // the trait's stable synthetic `mem://heap`.

    fn headers(&self) -> &Headers {
        &self.headers
    }

    fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    fn mode(&self) -> IOMode {
        self.mode
    }

    fn kind(&self) -> IOKind {
        IOKind::Heap
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
        // An offset so large the write would overflow the address space is a no-op (0 bytes
        // written) — `pwrite_all` then reports the shortfall as a guided error.
        let Some(end) = start.checked_add(data.len()) else {
            return 0;
        };
        if end > self.data.len() {
            self.data.resize(end, 0); // grow, zero-filling any gap
        }
        self.data[start..end].copy_from_slice(data);
        data.len()
    }
}

// Value equality over the stored bytes only — the cursor, address `Uri`, `Headers`, and `IOMode`
// are transient/metadata
// (see the type's DESIGN note). `Heap` is mutable, so it is deliberately not `Hash`.
impl PartialEq for Heap {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl Eq for Heap {}

/// The value form of a heap is its stored bytes — the same identity its equality uses (the
/// cursor, headers, and mode are transient metadata and are not serialized).
impl crate::io::Serializable for Heap {
    type Error = IoError;

    fn serialize_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }

    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Ok(Heap::from_slice(bytes))
    }
}
