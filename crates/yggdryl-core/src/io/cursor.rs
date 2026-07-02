//! The typed [`IOCursor`] adapter over [`RawIOCursor`].

use super::{IOBase, IOError, RawIOBase, RawIOCursor, Seekable, Whence};

/// A moving cursor over an [`IOBase<T>`] resource: like [`RawIOCursor`] every read
/// and write advances the position, and it additionally offers the typed
/// [`IOBase`] writes, so `T` values stream out one after another.
///
/// It layers the [`IOBase`] surface over a [`RawIOCursor`], forwarding the raw and
/// [`Seekable`] surfaces to it, so the cursor logic lives in one place. The wrapped
/// resource is reached with [`get_ref`](IOCursor::get_ref),
/// [`get_mut`](IOCursor::get_mut) or [`into_inner`](IOCursor::into_inner).
///
/// ```
/// use yggdryl_core::{IOBase, IOCursor, IOError, RawIOBase, Seekable, Whence};
///
/// // A minimal resource holding `u32`s, four little-endian bytes each.
/// #[derive(Default)]
/// struct Store {
///     data: Vec<u8>,
/// }
///
/// impl RawIOBase for Store {
///     fn byte_size(&self) -> usize {
///         self.data.len()
///     }
///     fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
///         self.data.resize(size, 0);
///         Ok(())
///     }
///     fn pread_byte_array(&self, position: usize, _w: Whence, size: usize) -> Result<Vec<u8>, IOError> {
///         self.data.get(position..position + size).map(<[u8]>::to_vec).ok_or(
///             IOError::OutOfBounds { offset: position + size, len: self.data.len() },
///         )
///     }
///     fn pwrite_byte_array(&mut self, position: usize, _w: Whence, values: &[u8]) -> Result<(), IOError> {
///         let end = position + values.len();
///         if end > self.data.len() {
///             self.data.resize(end, 0);
///         }
///         self.data[position..end].copy_from_slice(values);
///         Ok(())
///     }
///     fn pread_bit_array(&self, position: usize, _w: Whence, size: usize) -> Result<Vec<bool>, IOError> {
///         (0..size)
///             .map(|i| {
///                 let idx = position + i;
///                 self.data.get(idx / 8).map(|b| (b >> (7 - idx % 8)) & 1 == 1).ok_or(
///                     IOError::OutOfBounds { offset: idx, len: self.data.len() * 8 },
///                 )
///             })
///             .collect()
///     }
///     fn pwrite_bit_array(&mut self, position: usize, _w: Whence, values: &[bool]) -> Result<(), IOError> {
///         let needed = (position + values.len()).div_ceil(8);
///         if needed > self.data.len() {
///             self.data.resize(needed, 0);
///         }
///         for (i, &bit) in values.iter().enumerate() {
///             let idx = position + i;
///             let mask = 1u8 << (7 - idx % 8);
///             if bit {
///                 self.data[idx / 8] |= mask;
///             } else {
///                 self.data[idx / 8] &= !mask;
///             }
///         }
///         Ok(())
///     }
/// }
///
/// impl IOBase<u32> for Store {
///     fn value_to_bytes(&self, value: &u32) -> Vec<u8> {
///         value.to_le_bytes().to_vec()
///     }
///     fn size(&self) -> usize {
///         self.byte_size() / 4
///     }
///     fn resize(&mut self, size: usize) -> Result<(), IOError> {
///         self.resize_bytes(size * 4)
///     }
/// }
///
/// let mut cursor = IOCursor::new(Store::default());
/// cursor.pwrite_one(0, Whence::Start, &1)?;
/// cursor.pwrite_one(0, Whence::Current, &2)?; // continues after the first value
/// assert_eq!(cursor.tell(), 8); // two u32s written
/// assert_eq!(cursor.size(), 2);
/// assert_eq!(cursor.get_ref().data, vec![1, 0, 0, 0, 2, 0, 0, 0]);
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IOCursor<I> {
    raw: RawIOCursor<I>,
}

impl<I> IOCursor<I> {
    /// Wrap `inner`, with the cursor at the start.
    pub fn new(inner: I) -> Self {
        Self {
            raw: RawIOCursor::new(inner),
        }
    }

    /// A shared reference to the wrapped resource.
    pub fn get_ref(&self) -> &I {
        self.raw.get_ref()
    }

    /// A mutable reference to the wrapped resource.
    pub fn get_mut(&mut self) -> &mut I {
        self.raw.get_mut()
    }

    /// Consume the cursor, returning the wrapped resource.
    pub fn into_inner(self) -> I {
        self.raw.into_inner()
    }
}

impl<I: RawIOBase> Seekable for IOCursor<I> {
    fn tell(&self) -> usize {
        self.raw.tell()
    }

    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IOError> {
        self.raw.seek(position, whence)
    }
}

impl<I: RawIOBase> RawIOBase for IOCursor<I> {
    fn byte_size(&self) -> usize {
        self.raw.byte_size()
    }

    fn bit_size(&self) -> usize {
        self.raw.bit_size()
    }

    fn byte_capacity(&self) -> usize {
        self.raw.byte_capacity()
    }

    fn bit_capacity(&self) -> usize {
        self.raw.bit_capacity()
    }

    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        self.raw.resize_byte_capacity(capacity)
    }

    fn resize_bit_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        self.raw.resize_bit_capacity(capacity)
    }

    fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
        self.raw.resize_bytes(size)
    }

    fn resize_bits(&mut self, size: usize) -> Result<(), IOError> {
        self.raw.resize_bits(size)
    }

    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        self.raw.pread_byte_array(position, whence, size)
    }

    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError> {
        self.raw.pwrite_byte_array(position, whence, values)
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError> {
        self.raw.pread_bit_array(position, whence, size)
    }

    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError> {
        self.raw.pwrite_bit_array(position, whence, values)
    }
}

impl<I, T> IOBase<T> for IOCursor<I>
where
    I: IOBase<T>,
{
    fn value_to_bytes(&self, value: &T) -> Vec<u8> {
        self.raw.get_ref().value_to_bytes(value)
    }

    fn size(&self) -> usize {
        self.raw.get_ref().size()
    }

    fn capacity(&self) -> usize {
        self.raw.get_ref().capacity()
    }

    fn resize_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        self.raw.get_mut().resize_capacity(capacity)
    }

    fn resize(&mut self, size: usize) -> Result<(), IOError> {
        self.raw.get_mut().resize(size)
    }

    // `pwrite_one` / `pwrite_array` come from the trait defaults: they serialize
    // through this type's `value_to_bytes` and `pwrite_byte_array`, so each typed
    // write advances the cursor via the raw byte layer.
}
