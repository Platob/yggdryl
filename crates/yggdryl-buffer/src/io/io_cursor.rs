//! [`IOCursor`] — an [`IOBase`] that tracks a position over an inner resource.

use crate::IOBase;

/// An [`IOBase`] backed by an inner byte resource plus a **position** — the
/// `std::io::Cursor`-style pairing. Reads and writes happen at, and advance, this
/// position; the inner resource is only copied if the cursor writes (so the source
/// buffer stays intact). [`ByteCursor`](crate::ByteCursor) is the concrete one.
///
/// ```
/// use yggdryl_buffer::{ByteBuffer, IOCursor, IOBase, Whence};
///
/// let mut cursor = ByteBuffer::from_bytes(b"abcdef").byte_cursor();
/// cursor.set_position(2);
/// assert_eq!(cursor.position(), 2);
/// assert_eq!(cursor.pread_byte_array(3, Whence::Current).unwrap(), b"cde");
/// assert_eq!(cursor.position(), 5); // advanced by the read
/// ```
#[allow(clippy::upper_case_acronyms)] // `IO` matches the project's IO-trait naming.
pub trait IOCursor: IOBase {
    /// The current position, in bytes from the start (the infallible mirror of
    /// [`byte_tell`](IOBase::byte_tell)).
    fn position(&self) -> u64;

    /// Sets the current position to `position` bytes from the start.
    fn set_position(&mut self, position: u64);
}
