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

impl Heap {
    /// The exact [`IoError::UnexpectedEof`] the default trait helpers would report for a read
    /// of `requested` bytes at `offset` — kept identical so the fast overrides below never
    /// change an error a caller sees.
    fn eof(&self, offset: u64, requested: usize) -> IoError {
        let start = (offset as usize).min(self.data.len());
        let got = (self.data.len() - start).min(requested);
        IoError::UnexpectedEof {
            offset: offset + got as u64,
            requested,
            available: got,
        }
    }

    /// Grows the buffer so `start..end` is addressable, **writing the grown region only
    /// once**: the gap before `start` (if any) is zero-filled, but the region a caller is
    /// about to overwrite is *not* pre-zeroed. Kept out of line so the hot in-place write
    /// paths stay tiny and inline; returns `true` when `start..end` was already in place.
    #[inline]
    fn grow_for_write(&mut self, start: usize, end: usize) -> bool {
        if end <= self.data.len() {
            return true; // pure in-place overwrite
        }
        self.grow_slow(start, end);
        false
    }

    /// The cold growth half of [`grow_for_write`](Heap::grow_for_write).
    fn grow_slow(&mut self, start: usize, end: usize) {
        self.data.reserve(end - self.data.len());
        if start > self.data.len() {
            // A gap between the old end and the write: zero-fill exactly the gap.
            self.data.resize(start, 0);
        }
        // The tail is extended by the caller in one pass. Using `resize` here would zero-fill
        // bytes the caller immediately overwrites (a measured 2-3x penalty on appends).
    }

    /// The growing half of the byte-write primitive: zero-fills any gap, overwrites the
    /// existing overlap, and **extends** with the rest — the grown region is written exactly
    /// once (never zero-filled first).
    fn pwrite_grow(&mut self, start: usize, end: usize, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0; // an empty write never grows the buffer
        }
        self.data.reserve(end - self.data.len());
        if start > self.data.len() {
            self.data.resize(start, 0); // zero-fill exactly the gap
        }
        let overlap = self.data.len() - start;
        self.data[start..].copy_from_slice(&data[..overlap]);
        self.data.extend_from_slice(&data[overlap..]);
        data.len()
    }

    /// Fills `start..end` (already sized) with the little-endian bytes of one repeated value
    /// by writing the first element and then **doubling** the filled region with
    /// `copy_within` — `log2(n)` bulk memcpys instead of `n` scalar stores.
    #[inline]
    fn fill_repeat(&mut self, start: usize, end: usize, pattern: &[u8]) {
        let total = end - start;
        if total == 0 {
            return;
        }
        self.data[start..start + pattern.len()].copy_from_slice(pattern);
        let mut filled = pattern.len();
        while filled < total {
            let take = filled.min(total - filled);
            self.data.copy_within(start..start + take, start + filled);
            filled += take;
        }
    }
}

impl IOBase for Heap {
    #[inline]
    fn byte_size(&self) -> u64 {
        self.data.len() as u64
    }

    #[inline]
    fn capacity(&self) -> u64 {
        self.data.capacity() as u64
    }

    #[inline]
    fn reserve(&mut self, additional: u64) {
        self.data.reserve(additional as usize);
    }

    fn reserve_exact(&mut self, additional: u64) {
        self.data.reserve_exact(additional as usize);
    }

    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        // Checked: a request past `usize` or one the allocator refuses is a guided error,
        // never an abort. (`u64 -> usize` is lossless on 64-bit; on 32-bit an oversized
        // request fails the same way through the `usize::MAX` clamp.)
        let want = usize::try_from(additional).unwrap_or(usize::MAX);
        self.data
            .try_reserve(want)
            .map_err(|_| IoError::CapacityOverflow {
                additional,
                capacity: self.data.capacity() as u64,
            })
    }

    fn try_reserve_exact(&mut self, additional: u64) -> Result<(), IoError> {
        let want = usize::try_from(additional).unwrap_or(usize::MAX);
        self.data
            .try_reserve_exact(want)
            .map_err(|_| IoError::CapacityOverflow {
                additional,
                capacity: self.data.capacity() as u64,
            })
    }

    fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
    }

    fn shrink_to(&mut self, min_capacity: u64) {
        self.data
            .shrink_to(usize::try_from(min_capacity).unwrap_or(usize::MAX));
    }

    // `uri()` is deliberately NOT overridden: a heap stores no address, so every heap reports
    // the trait's stable synthetic `mem://heap`.

    #[inline]
    fn headers(&self) -> &Headers {
        &self.headers
    }

    #[inline]
    fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    #[inline]
    fn mode(&self) -> IOMode {
        self.mode
    }

    #[inline]
    fn kind(&self) -> IOKind {
        IOKind::Heap
    }

    #[inline]
    fn exists(&self) -> bool {
        true // a live in-memory buffer always exists (it is neither file nor directory)
    }

    #[inline]
    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        let start = offset as usize;
        if start >= self.data.len() {
            return 0;
        }
        let n = buf.len().min(self.data.len() - start);
        buf[..n].copy_from_slice(&self.data[start..start + n]);
        n
    }

    #[inline]
    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        let start = offset as usize;
        // Hot path first: a pure in-place overwrite is one bounds check + one copy.
        if let Some(end) = start.checked_add(data.len()) {
            if end <= self.data.len() {
                self.data[start..end].copy_from_slice(data);
                return data.len();
            }
            return self.pwrite_grow(start, end, data);
        }
        // An offset so large the write would overflow the address space is a no-op (0 bytes
        // written) — `pwrite_all` then reports the shortfall as a guided error.
        0
    }

    // ---- Heap fast paths over the trait defaults ---------------------------------------
    // The defaults stage every typed/bulk op through the byte primitives (a stack chunk +
    // a second copy); the heap owns contiguous bytes, so it converts in a single pass. Each
    // override keeps the default's exact semantics — same results, same error values —
    // and the benchmark compares them against the defaults through a minimal source.

    #[inline]
    fn pread_byte(&self, offset: u64) -> Result<u8, IoError> {
        self.data
            .get(offset as usize)
            .copied()
            .ok_or_else(|| self.eof(offset, 1))
    }

    #[inline]
    fn pread_i32(&self, offset: u64) -> Result<i32, IoError> {
        let start = offset as usize;
        match self.data.get(start..start.saturating_add(4)) {
            Some(raw) => Ok(i32::from_le_bytes(raw.try_into().expect("4-byte slice"))),
            None => Err(self.eof(offset, 4)),
        }
    }

    #[inline]
    fn pread_i64(&self, offset: u64) -> Result<i64, IoError> {
        let start = offset as usize;
        match self.data.get(start..start.saturating_add(8)) {
            Some(raw) => Ok(i64::from_le_bytes(raw.try_into().expect("8-byte slice"))),
            None => Err(self.eof(offset, 8)),
        }
    }

    fn pread_i32_array(&self, offset: u64, dst: &mut [i32]) -> Result<(), IoError> {
        let start = offset as usize;
        let need = dst.len() * 4;
        let Some(src) = self.data.get(start..start.saturating_add(need)) else {
            return Err(self.eof(offset, need));
        };
        // One dense, branch-free conversion pass straight off the stored bytes.
        for (value, raw) in dst.iter_mut().zip(src.chunks_exact(4)) {
            *value = i32::from_le_bytes(raw.try_into().expect("chunks_exact yields 4"));
        }
        Ok(())
    }

    fn pread_i64_array(&self, offset: u64, dst: &mut [i64]) -> Result<(), IoError> {
        let start = offset as usize;
        let need = dst.len() * 8;
        let Some(src) = self.data.get(start..start.saturating_add(need)) else {
            return Err(self.eof(offset, need));
        };
        for (value, raw) in dst.iter_mut().zip(src.chunks_exact(8)) {
            *value = i64::from_le_bytes(raw.try_into().expect("chunks_exact yields 8"));
        }
        Ok(())
    }

    fn pwrite_i32_array(&mut self, offset: u64, src: &[i32]) -> Result<(), IoError> {
        let start = offset as usize;
        let Some(end) = start.checked_add(src.len() * 4) else {
            return Err(self.eof(offset, src.len() * 4));
        };
        if !self.grow_for_write(start, end) {
            self.data.resize(end, 0); // rare grow: sized once, then densely overwritten
        }
        for (raw, value) in self.data[start..end].chunks_exact_mut(4).zip(src) {
            raw.copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }

    fn pwrite_i64_array(&mut self, offset: u64, src: &[i64]) -> Result<(), IoError> {
        let start = offset as usize;
        let Some(end) = start.checked_add(src.len() * 8) else {
            return Err(self.eof(offset, src.len() * 8));
        };
        if !self.grow_for_write(start, end) {
            self.data.resize(end, 0);
        }
        for (raw, value) in self.data[start..end].chunks_exact_mut(8).zip(src) {
            raw.copy_from_slice(&value.to_le_bytes());
        }
        Ok(())
    }

    fn pwrite_byte_repeat(&mut self, offset: u64, value: u8, count: usize) -> Result<(), IoError> {
        let start = offset as usize;
        let Some(end) = start.checked_add(count) else {
            return Err(self.eof(offset, count));
        };
        let old_len = self.data.len();
        if !self.grow_for_write(start, end) {
            // The grown tail is filled directly with `value` — one pass, a plain memset.
            self.data.resize(end, value);
        }
        // Fill only the pre-existing overlap; the grown tail already holds `value`.
        let overlap_end = end.min(old_len);
        if overlap_end > start {
            self.data[start..overlap_end].fill(value);
        }
        Ok(())
    }

    fn pwrite_i32_repeat(&mut self, offset: u64, value: i32, count: usize) -> Result<(), IoError> {
        let start = offset as usize;
        let Some(end) = start.checked_add(count * 4) else {
            return Err(self.eof(offset, count * 4));
        };
        if !self.grow_for_write(start, end) {
            self.data.resize(end, 0);
        }
        self.fill_repeat(start, end, &value.to_le_bytes());
        Ok(())
    }

    fn pwrite_i64_repeat(&mut self, offset: u64, value: i64, count: usize) -> Result<(), IoError> {
        let start = offset as usize;
        let Some(end) = start.checked_add(count * 8) else {
            return Err(self.eof(offset, count * 8));
        };
        if !self.grow_for_write(start, end) {
            self.data.resize(end, 0);
        }
        self.fill_repeat(start, end, &value.to_le_bytes());
        Ok(())
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
