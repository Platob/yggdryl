//! [`TypedSlice<T>`] — a bounded, element-typed window over a [`ByteBuffer`].

use core::marker::PhantomData;

use crate::{
    ByteBuffer, ByteSlice, IOBase, IOCursor, IOSlice, IoError, IoPrimitive, TypedIOBase, Whence,
};

/// A fixed-length **window** whose native unit is a `T` value — the bounded sibling of
/// [`TypedCursor<T>`](crate::TypedCursor), layered over a [`ByteSlice`].
/// [`pread_one`](TypedIOBase::pread_one) / [`pwrite_array`](TypedIOBase::pwrite_array)
/// move whole `T` values (little-endian) confined to the window; [`tell`](TypedIOBase::tell)
/// / [`seek`](TypedIOBase::seek) count in `T` units, and a write past the window end
/// writes only the whole values that fit — the window never grows. The underlying byte
/// and bit positions ([`byte_tell`](IOBase::byte_tell) / [`bit_tell`](IOBase::bit_tell))
/// and the window bounds ([`slice_offset`](IOSlice::slice_offset) /
/// [`slice_len`](IOSlice::slice_len), in bytes) remain available.
///
/// Copy-on-write over its source [`ByteBuffer`], so writes leave the buffer intact.
/// Obtain one from a typed buffer's `slice` (e.g.
/// [`I64Buffer::slice`](crate::I64Buffer::slice)) or [`TypedSlice::new`].
///
/// ```
/// use yggdryl_core::{I32Buffer, IOBase, TypedIOBase, Whence};
///
/// let buffer = I32Buffer::from_slice(&[10, 20, 30, 40, 50]);
/// let mut slice = buffer.slice(1, 3); // the [20, 30, 40] window (3 i32)
/// assert_eq!(slice.size().unwrap(), 3);
/// assert_eq!(slice.pread_array(100, Whence::Start).unwrap(), vec![20, 30, 40]); // clamped
/// ```
#[allow(clippy::upper_case_acronyms)] // matches the project's IO-type naming.
pub struct TypedSlice<T: IoPrimitive> {
    inner: ByteSlice,
    _marker: PhantomData<fn() -> T>,
}

impl<T: IoPrimitive> TypedSlice<T> {
    /// Creates a `T`-typed window over `buffer` spanning the **byte** range
    /// `[offset, offset + len)`, clamped to the buffer's bytes.
    pub fn new(buffer: ByteBuffer, offset: u64, len: usize) -> Self {
        Self::from_byte_slice(ByteSlice::new(buffer, offset, len))
    }

    /// Wraps an existing [`ByteSlice`] as a `T`-typed window (sharing its bytes and
    /// bounds).
    pub fn from_byte_slice(inner: ByteSlice) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }

    /// Borrows the window's bytes, including any writes it has made.
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Freezes the window's bytes into a new [`ByteBuffer`].
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        self.inner.to_byte_buffer()
    }

    /// Borrows the underlying byte slice.
    pub fn as_byte_slice(&self) -> &ByteSlice {
        &self.inner
    }
}

impl<T: IoPrimitive> core::fmt::Debug for TypedSlice<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TypedSlice")
            .field("width", &T::WIDTH)
            .field("offset", &self.inner.slice_offset())
            .field("len", &self.inner.slice_len())
            .finish()
    }
}

impl<T: IoPrimitive> Clone for TypedSlice<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: IoPrimitive> IOBase for TypedSlice<T> {
    fn with_byte_capacity(capacity: usize) -> Self {
        Self::from_byte_slice(ByteSlice::with_byte_capacity(capacity))
    }

    fn byte_tell(&self) -> Result<u64, IoError> {
        self.inner.byte_tell()
    }

    fn byte_seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        self.inner.byte_seek(offset, whence)
    }

    fn byte_size(&self) -> Result<usize, IoError> {
        self.inner.byte_size()
    }

    fn byte_capacity(&self) -> Result<usize, IoError> {
        self.inner.byte_capacity()
    }

    fn pread_byte_array(&mut self, size: usize, whence: Whence) -> Result<Vec<u8>, IoError> {
        self.inner.pread_byte_array(size, whence)
    }

    fn pread_into(&mut self, buf: &mut [u8], whence: Whence) -> Result<usize, IoError> {
        self.inner.pread_into(buf, whence)
    }

    fn pwrite_byte_array(&mut self, data: &[u8], whence: Whence) -> Result<usize, IoError> {
        self.inner.pwrite_byte_array(data, whence)
    }
}

impl<T: IoPrimitive> TypedIOBase<T> for TypedSlice<T> {
    fn pread_one(&mut self, whence: Whence) -> Result<T, IoError> {
        let bytes = self.inner.pread_byte_array(T::WIDTH, whence)?;
        if bytes.len() < T::WIDTH {
            return Err(IoError::UnexpectedEof {
                needed: T::WIDTH,
                available: bytes.len(),
            });
        }
        Ok(T::from_le_slice(&bytes))
    }

    fn pwrite_one(&mut self, value: T, whence: Whence) -> Result<usize, IoError> {
        self.pwrite_array(core::slice::from_ref(&value), whence)
    }

    fn pread_array(&mut self, count: usize, whence: Whence) -> Result<Vec<T>, IoError> {
        let bytes = self
            .inner
            .pread_byte_array(count.saturating_mul(T::WIDTH), whence)?;
        Ok(bytes.chunks_exact(T::WIDTH).map(T::from_le_slice).collect())
    }

    fn pwrite_array(&mut self, data: &[T], whence: Whence) -> Result<usize, IoError> {
        // Position at the write start, then write only the whole `T` values that fit in
        // the remaining window — a slice never grows.
        self.inner.byte_seek(0, whence)?;
        let fits = self.inner.byte_size()? / T::WIDTH.max(1);
        let count = fits.min(data.len());
        let mut buf = Vec::with_capacity(count.saturating_mul(T::WIDTH));
        for value in &data[..count] {
            value.write_le(&mut buf);
        }
        self.inner.pwrite_byte_array(&buf, Whence::Current)?;
        Ok(count)
    }
}

impl<T: IoPrimitive> IOCursor for TypedSlice<T> {
    fn position(&self) -> u64 {
        self.inner.position()
    }

    fn set_position(&mut self, position: u64) {
        self.inner.set_position(position);
    }
}

impl<T: IoPrimitive> IOSlice for TypedSlice<T> {
    fn slice_offset(&self) -> u64 {
        self.inner.slice_offset()
    }

    fn slice_len(&self) -> usize {
        self.inner.slice_len()
    }
}
