//! The typed [`IOSlice`] adapter over [`RawIOSlice`].

use super::{IOBase, IOError, RawIOBase, RawIOSlice, Whence};

/// A bounded window over an [`IOBase<T>`] resource: like [`RawIOSlice`] it
/// restricts every access to a byte range `[start, end)`, and it additionally
/// offers the typed [`IOBase`] surface so `T` values within the window are written
/// through it.
///
/// It layers the [`IOBase`] surface over a [`RawIOSlice`], forwarding the raw
/// surface to it, so the windowing logic lives in one place. The wrapped resource is
/// reached with [`get_ref`](IOSlice::get_ref), [`get_mut`](IOSlice::get_mut) or
/// [`into_inner`](IOSlice::into_inner). [`size`](IOBase::size) counts the whole `T`
/// items in the window, inferring the item width from the inner as
/// `byte_size / size`; over an empty inner the width is unknown and `size` reports
/// `0` (prefer a [`RawIOSlice`] there).
///
/// ```
/// use yggdryl_core::{IOBase, IOError, IOSlice, RawIOBase, Whence};
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
/// let mut store = Store::default();
/// store.pwrite_array(0, Whence::Start, &[1, 2, 3, 4]).unwrap(); // four u32s, 16 bytes
///
/// // Slice the middle two u32s: bytes [4, 12).
/// let slice = IOSlice::new(store, 4, 12);
/// assert_eq!(slice.size(), 2); // two u32 items in the window
/// assert_eq!(slice.byte_size(), 8);
/// assert_eq!(
///     slice.pread_byte_array(0, Whence::Start, 8).unwrap(),
///     vec![2, 0, 0, 0, 3, 0, 0, 0],
/// );
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IOSlice<I> {
    raw: RawIOSlice<I>,
}

impl<I> IOSlice<I> {
    /// Wrap `inner`, restricting access to the byte window `[start, end)`.
    pub fn new(inner: I, start: usize, end: usize) -> Self {
        Self {
            raw: RawIOSlice::new(inner, start, end),
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

    /// Consume the slice, returning the wrapped resource.
    pub fn into_inner(self) -> I {
        self.raw.into_inner()
    }

    /// The window's start byte offset into the wrapped resource.
    pub fn start(&self) -> usize {
        self.raw.start()
    }

    /// The window's end byte offset (exclusive) into the wrapped resource.
    pub fn end(&self) -> usize {
        self.raw.end()
    }
}

impl<I: RawIOBase> RawIOBase for IOSlice<I> {
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

impl<I, T> IOBase<T> for IOSlice<I>
where
    I: IOBase<T>,
{
    fn value_to_bytes(&self, value: &T) -> Vec<u8> {
        self.raw.get_ref().value_to_bytes(value)
    }

    fn size(&self) -> usize {
        let width = element_width::<T>(self.raw.get_ref());
        if width == 0 {
            0
        } else {
            self.raw.byte_size() / width
        }
    }

    fn resize(&mut self, size: usize) -> Result<(), IOError> {
        let width = element_width::<T>(self.raw.get_ref());
        self.raw.resize_bytes(size.saturating_mul(width))
    }
}

/// The fixed byte width of a `T` in `inner`, inferred as `byte_size / size` — the
/// inverse of how [`IOBase::size`] counts items. Returns `0` when the inner holds no
/// items (the width is then unknown).
fn element_width<T>(inner: &(impl IOBase<T> + ?Sized)) -> usize {
    match inner.size() {
        0 => 0,
        items => inner.byte_size() / items,
    }
}
