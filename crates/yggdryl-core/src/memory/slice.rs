//! [`IOSlice`] — a concrete bounded window over any [`IOBase`].

use super::{IOBase, IoError};

/// A **bounded window** over any [`IOBase`] source: it owns the source and presents the range
/// `[offset, offset + len)` addressed from its own `0`. Build one from any source with
/// [`IOBase::window`](super::IOBase::window).
///
/// `IOSlice<T>` is itself an [`IOBase`] (its [`byte_size`](super::IOBase::byte_size) is the window
/// length, reads/writes are shifted by the offset and clamped to the window, and its
/// [`uri`](super::IOBase::uri) is the source's), so a window can be re-windowed or cursored and
/// used anywhere a source is. Owning the source keeps the type lifetime-free, so the bindings can
/// hold it; to keep the original, wrap a clone.
///
/// DESIGN: the window is **fixed-length** — a write past its end is clamped (it can never grow the
/// underlying source beyond the window), unlike the growing primitive on the source itself.
///
/// ```
/// use yggdryl_core::memory::{Heap, IOBase};
///
/// let win = Heap::from_slice(b"hello world").window(6, 5).unwrap(); // IOSlice<Heap>
/// assert_eq!(win.byte_size(), 5);
/// assert_eq!(win.pread_vec(0, 5), b"world"); // addressed from its own 0
/// assert!(Heap::from_slice(b"hello world").window(6, 6).is_err()); // 6 + 6 > 11
/// ```
#[derive(Clone, Debug, Default)]
pub struct IOSlice<T: IOBase> {
    inner: T,
    offset: u64,
    len: u64,
}

impl<T: IOBase> IOSlice<T> {
    /// Wraps `inner` as the window `[offset, offset + len)`. Errors with
    /// [`IoError::SliceOutOfBounds`] if the window runs past the source's end.
    pub fn new(inner: T, offset: u64, len: u64) -> Result<Self, IoError> {
        let available = inner.byte_size();
        offset
            .checked_add(len)
            .filter(|&end| end <= available)
            .ok_or(IoError::SliceOutOfBounds {
                offset,
                len,
                available,
            })?;
        Ok(Self { inner, offset, len })
    }

    /// The window's start offset within the source.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Borrows the wrapped source.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Mutably borrows the wrapped source.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwraps into the source, discarding the window bounds.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: IOBase> IOBase for IOSlice<T> {
    fn byte_size(&self) -> u64 {
        self.len
    }

    fn capacity(&self) -> u64 {
        self.len
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        if offset >= self.len {
            return 0;
        }
        let room = (self.len - offset) as usize;
        let n = buf.len().min(room);
        self.inner
            .pread_byte_array(self.offset + offset, &mut buf[..n])
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        if offset >= self.len {
            return 0; // the window is fixed-length; a write past its end is clamped away
        }
        let room = (self.len - offset) as usize;
        let n = data.len().min(room);
        self.inner
            .pwrite_byte_array(self.offset + offset, &data[..n])
    }

    fn uri(&self) -> crate::uri::Uri {
        self.inner.uri()
    }
}
