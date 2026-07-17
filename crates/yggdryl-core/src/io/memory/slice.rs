//! [`IOSlice`] — bounded sub-views of an [`IOBase`].

use super::IOBase;
use crate::io::IoError;

/// An [`IOBase`] that can hand out a **bounded window** of itself — a sub-range addressed
/// from its own offset `0`, as an independent, owned value (no borrow of the parent, so the
/// bindings can hold it and it carries no lifetime).
///
/// Where the physical layer allows, the window is **zero-copy** — [`Bytes`](super::Bytes)
/// shares the underlying Arrow allocation, so slicing is an `Arc` bump, and a later write to
/// either side copies-on-write so they never alias. The window is a full `IOBase` (and, for
/// `Bytes`, a full [`IOCursor`](super::IOCursor)), so it can itself be sliced again.
pub trait IOSlice: IOBase {
    /// The window `[offset, offset + len)` as an owned value whose offset `0` maps to
    /// `offset` here. Errors with [`IoError::SliceOutOfBounds`] if it runs past the end.
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOBase, IOSlice};
    ///
    /// let data = Bytes::from_slice(b"hello world");
    /// let world = data.slice(6, 5).unwrap();
    /// assert_eq!(world.len(), 5);
    /// assert_eq!(world.as_slice(), b"world"); // addressed from its own 0
    /// assert!(data.slice(6, 6).is_err());     // 6 + 6 > 11
    /// ```
    fn slice(&self, offset: u64, len: u64) -> Result<Self, IoError>
    where
        Self: Sized;
}
