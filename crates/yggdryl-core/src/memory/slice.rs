//! [`IOSlice`] — bounded sub-views of an [`IOBase`].

use super::IOBase;
use super::IoError;

/// An [`IOBase`] that can hand out a **bounded window** of itself — a sub-range addressed
/// from its own offset `0`, as an independent, owned value (no borrow of the parent, so the
/// bindings can hold it and it carries no lifetime).
///
/// The window is a full `IOBase` (and, where the source is one, a full
/// [`IOCursor`](super::IOCursor)), so it can itself be sliced again. For the in-heap
/// [`Heap`](super::Heap) source the window owns a **copy** of the range; a source that can share
/// its backing (e.g. a memory-map) may return a zero-copy view — either way the returned value is
/// independent of the parent.
pub trait IOSlice: IOBase {
    /// The window `[offset, offset + len)` as an owned value whose offset `0` maps to
    /// `offset` here. Errors with [`IoError::SliceOutOfBounds`] if it runs past the end.
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOBase, IOSlice};
    ///
    /// let data = Heap::from_slice(b"hello world");
    /// let world = data.slice(6, 5).unwrap();
    /// assert_eq!(world.byte_size(), 5);
    /// assert_eq!(world.as_slice(), b"world"); // addressed from its own 0
    /// assert!(data.slice(6, 6).is_err());     // 6 + 6 > 11
    /// ```
    fn slice(&self, offset: u64, len: u64) -> Result<Self, IoError>
    where
        Self: Sized;
}
