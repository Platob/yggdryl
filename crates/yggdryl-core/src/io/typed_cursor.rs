//! [`TypedCursor<T>`] — a positioned cursor whose native unit is a `T` value.

use core::marker::PhantomData;
use std::borrow::Cow;

use crate::{ByteBuffer, ByteCursor, IOBase, IOCursor, IoError, IoPrimitive, TypedIOBase, Whence};

/// A positioned, advancing cursor whose native unit is a fixed-width primitive `T`,
/// layered over a [`ByteCursor`]. It is the `T`-typed analogue of [`ByteCursor`]:
/// [`pread_one`](TypedIOBase::pread_one) / [`pwrite_array`](TypedIOBase::pwrite_array)
/// move whole `T` values (little-endian), [`tell`](TypedIOBase::tell) /
/// [`seek`](TypedIOBase::seek) count in `T` units, and a write past the end fills the
/// gap with the `T` [`default_value`](TypedIOBase::default_value) (zero for every
/// native primitive). The underlying [`byte_tell`](IOBase::byte_tell) /
/// [`byte_seek`](IOBase::byte_seek) and [`bit_tell`](IOBase::bit_tell) /
/// [`bit_seek`](IOBase::bit_seek) byte/bit positions remain available.
///
/// Like [`ByteCursor`] it is copy-on-write over its source [`ByteBuffer`], so writes
/// leave the buffer intact. Obtain one from a typed buffer's `cursor` (in the
/// `yggdryl-buffer` crate) or [`TypedCursor::new`].
///
/// ```
/// use yggdryl_core::{ByteBuffer, IOBase, TypedCursor, TypedIOBase, Whence};
///
/// // Three little-endian i32 values as bytes.
/// let mut bytes = Vec::new();
/// for value in [10_i32, 20, 30] {
///     bytes.extend_from_slice(&value.to_le_bytes());
/// }
/// let mut cursor = TypedCursor::<i32>::new(ByteBuffer::from_vec(bytes));
/// assert_eq!(cursor.pread_one(Whence::Start).unwrap(), 10);
/// assert_eq!(cursor.tell().unwrap(), 1); // one i32 in
/// cursor.seek(2, Whence::Start).unwrap();
/// assert_eq!(cursor.pread_one(Whence::Current).unwrap(), 30);
/// ```
#[allow(clippy::upper_case_acronyms)] // matches the project's IO-type naming.
pub struct TypedCursor<T: IoPrimitive> {
    inner: ByteCursor,
    _marker: PhantomData<fn() -> T>,
}

impl<T: IoPrimitive> TypedCursor<T> {
    /// Creates a typed cursor over `buffer`, positioned at the start.
    pub fn new(buffer: ByteBuffer) -> Self {
        Self::from_byte_cursor(buffer.byte_cursor())
    }

    /// Wraps an existing [`ByteCursor`] as a `T`-typed cursor (sharing its position
    /// and bytes).
    pub fn from_byte_cursor(inner: ByteCursor) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }

    /// Borrows the cursor's current bytes, including any writes it has made.
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }

    /// Freezes the cursor's current bytes into a new [`ByteBuffer`].
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        self.inner.to_byte_buffer()
    }

    /// Borrows the underlying byte cursor.
    pub fn as_byte_cursor(&self) -> &ByteCursor {
        &self.inner
    }
}

impl<T: IoPrimitive> core::fmt::Debug for TypedCursor<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TypedCursor")
            .field("width", &T::WIDTH)
            .field("position", &self.inner.position())
            .finish()
    }
}

impl<T: IoPrimitive> Clone for TypedCursor<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: IoPrimitive> IOBase for TypedCursor<T> {
    fn with_byte_capacity(capacity: usize) -> Self {
        Self::from_byte_cursor(ByteCursor::with_byte_capacity(capacity))
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

impl<T: IoPrimitive> TypedIOBase<T> for TypedCursor<T> {
    fn pread_one(&mut self, whence: Whence) -> Result<T, IoError> {
        // Read into a stack scratch (no heap allocation) for the common `WIDTH <= 8`
        // native primitives, falling back to a heap read for any wider `T`.
        let mut stack = [0u8; 8];
        if T::WIDTH <= stack.len() {
            let n = self.inner.pread_into(&mut stack[..T::WIDTH], whence)?;
            if n < T::WIDTH {
                return Err(IoError::UnexpectedEof {
                    needed: T::WIDTH,
                    available: n,
                });
            }
            Ok(T::from_le_slice(&stack[..T::WIDTH]))
        } else {
            let bytes = self.inner.pread_byte_array(T::WIDTH, whence)?;
            if bytes.len() < T::WIDTH {
                return Err(IoError::UnexpectedEof {
                    needed: T::WIDTH,
                    available: bytes.len(),
                });
            }
            Ok(T::from_le_slice(&bytes))
        }
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
        // Little-endian byte view of `data`. When `T`'s in-memory bytes already are
        // its wire form on this target (the native integers/floats), borrow the slice
        // as bytes zero-copy; otherwise (the wide integers, or a big-endian target)
        // encode each value with `write_le`.
        let reinterpret = T::REINTERPRET_LE && cfg!(target_endian = "little");
        let encoded: Cow<'_, [u8]> = if reinterpret {
            // SAFETY: guarded by `REINTERPRET_LE` — `T` has no padding, its in-memory
            // bytes equal its little-endian wire form, and `u8` has alignment 1, so
            // viewing the value slice as bytes is sound; the borrow is tied to `data`.
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    data.as_ptr().cast::<u8>(),
                    core::mem::size_of_val(data),
                )
            };
            Cow::Borrowed(bytes)
        } else {
            let mut buf = Vec::with_capacity(data.len().saturating_mul(T::WIDTH));
            for &value in data {
                value.write_le(&mut buf);
            }
            Cow::Owned(buf)
        };
        let bytes: &[u8] = &encoded;

        // Resolve the write start, then the total extent (`byte_size` reports only
        // the *remaining* bytes, so read the total via an `End` seek). Only when the
        // write opens a gap past the end do we fill it — with the `T` default via
        // `default_byte_array` (zero for every native primitive), never leaving it
        // undefined; the common append/overwrite is a single write.
        let start = self.inner.byte_seek(0, whence)?;
        let total = self.inner.byte_seek(0, Whence::End)?; // cursor now at the end
        if start > total {
            let gap = (start - total) as usize;
            let mut fill = self.default_byte_array(gap.div_ceil(T::WIDTH.max(1)));
            fill.truncate(gap);
            // The cursor sits at `total`; writing the fill advances it to `start`.
            self.inner.pwrite_byte_array(&fill, Whence::Current)?;
        } else {
            self.inner.set_position(start);
        }
        // The cursor now sits at `start` (from the fill, or the reset above).
        self.inner.pwrite_byte_array(bytes, Whence::Current)?;
        Ok(data.len())
    }
}

impl<T: IoPrimitive> IOCursor for TypedCursor<T> {
    fn position(&self) -> u64 {
        self.inner.position()
    }

    fn set_position(&mut self, position: u64) {
        self.inner.set_position(position);
    }
}
