//! The [`ByteBufferSlice`] alias: a byte window over an owned [`ByteBuffer`].

use super::{ByteBuffer, RawIOSlice};

/// A [`RawIOSlice`] window over an owned [`ByteBuffer`] — the concrete byte-slice
/// resource: one buffer, bounded to `[start, end)`, with the whole positioned-IO
/// surface reading relative to the window.
///
/// It is the shape a bounded byte value takes when it moves as a Rust native
/// value (the data layer's `binary` scalars hand it out), and the same concrete
/// pairing the bindings expose as their `ByteBufferSlice` class.
///
/// ```
/// use yggdryl_core::{ByteBuffer, ByteBufferSlice, RawIOBase, Whence};
///
/// let window: ByteBufferSlice = ByteBuffer::from_bytes(vec![1, 2, 3, 4]).slice(1, 3);
/// assert_eq!(window.byte_size(), 2);
/// assert_eq!(window.pread_byte_one(0, Whence::Start)?, 2); // window-relative
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
pub type ByteBufferSlice = RawIOSlice<ByteBuffer>;
