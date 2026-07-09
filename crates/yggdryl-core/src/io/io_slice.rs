//! [`IOSlice`] — an [`IOBase`] that is a bounded window over an inner resource.

use crate::IOBase;

/// An [`IOBase`] that presents a fixed-length **window** `[offset, offset + len)` over
/// an inner byte resource — the bounded, non-growing sibling of
/// [`IOCursor`](crate::IOCursor). Its own byte positions `0..len` map onto the window;
/// reads and writes are confined to it (a read past the window end yields fewer bytes,
/// a write past it writes only what fits), and it never grows.
/// [`ByteSlice`](crate::ByteSlice) is the concrete one.
///
/// ```
/// use yggdryl_core::{ByteBuffer, IOBase, IOSlice, Whence};
///
/// let buffer = ByteBuffer::from_bytes(b"hello world");
/// let mut slice = buffer.byte_slice(6, 5); // the "world" window
/// assert_eq!(slice.slice_offset(), 6);
/// assert_eq!(slice.slice_len(), 5);
/// // Reads are clamped to the window, so an over-request stops at its end.
/// assert_eq!(slice.pread_byte_array(100, Whence::Start).unwrap(), b"world");
/// ```
#[allow(clippy::upper_case_acronyms)] // `IO` matches the project's IO-trait naming.
pub trait IOSlice: IOBase {
    /// The window's start offset within the origin resource, in bytes.
    fn slice_offset(&self) -> u64;

    /// The window's length in bytes (its fixed extent — also its capacity).
    fn slice_len(&self) -> usize;
}
