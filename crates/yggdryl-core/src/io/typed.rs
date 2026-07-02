//! The typed [`IOBase`] layer over [`RawIOBase`](super::RawIOBase).

use super::{IOError, RawIOBase, Whence};

/// A typed view over a [`RawIOBase`] resource: writes values of type `T` by
/// converting them to bytes.
///
/// An implementor says how a `T` becomes bytes with
/// [`value_to_bytes`](IOBase::value_to_bytes), how many items the resource holds
/// with [`size`](IOBase::size), and how to change that count with
/// [`resize`](IOBase::resize); the typed writes [`pwrite_one`](IOBase::pwrite_one) /
/// [`pwrite_array`](IOBase::pwrite_array) then come for free — they serialize
/// through it and delegate to the raw byte methods.
///
/// ```
/// use yggdryl_core::{IOBase, IOError, RawIOBase, Whence};
///
/// #[derive(Default)]
/// struct Mem {
///     data: Vec<u8>,
/// }
///
/// impl RawIOBase for Mem {
///     fn byte_size(&self) -> usize {
///         self.data.len()
///     }
///
///     fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
///         self.data.resize(size, 0);
///         Ok(())
///     }
///
///     fn pread_byte_array(&self, position: usize, _whence: Whence, size: usize) -> Result<Vec<u8>, IOError> {
///         self.data.get(position..position + size).map(<[u8]>::to_vec).ok_or_else(|| {
///             IOError::OutOfBounds { offset: position + size, len: self.data.len() }
///         })
///     }
///     fn pwrite_byte_array(&mut self, position: usize, _whence: Whence, values: &[u8]) -> Result<(), IOError> {
///         let end = position + values.len();
///         if end > self.data.len() {
///             self.data.resize(end, 0);
///         }
///         self.data[position..end].copy_from_slice(values);
///         Ok(())
///     }
///     fn pread_bit_array(&self, position: usize, _whence: Whence, size: usize) -> Result<Vec<bool>, IOError> {
///         (0..size)
///             .map(|i| {
///                 let idx = position + i;
///                 self.data.get(idx / 8).map(|b| (b >> (7 - idx % 8)) & 1 == 1).ok_or_else(|| {
///                     IOError::OutOfBounds { offset: idx, len: self.data.len() * 8 }
///                 })
///             })
///             .collect()
///     }
///     fn pwrite_bit_array(&mut self, position: usize, _whence: Whence, values: &[bool]) -> Result<(), IOError> {
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
/// // The typed layer: say how a u32 becomes bytes, count and resize in items, and
/// // typed writes come for free.
/// impl IOBase<u32> for Mem {
///     fn value_to_bytes(&self, value: &u32) -> Vec<u8> {
///         value.to_le_bytes().to_vec()
///     }
///
///     fn size(&self) -> usize {
///         self.byte_size() / 4 // four bytes per u32
///     }
///
///     fn resize(&mut self, size: usize) -> Result<(), IOError> {
///         self.resize_bytes(size * 4)
///     }
/// }
///
/// let mut mem = Mem::default();
/// mem.pwrite_one(0, Whence::Start, &0x0403_0201)?;
/// mem.pwrite_array(4, Whence::Start, &[0x0807_0605, 0x0c0b_0a09])?;
/// assert_eq!(
///     mem.pread_byte_array(0, Whence::Start, 12)?,
///     vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
/// );
/// assert_eq!(mem.size(), 3); // three u32 items
/// assert_eq!(mem.capacity(), 3); // default: capacity mirrors size
///
/// mem.resize(4)?; // one more zeroed item
/// assert_eq!((mem.size(), mem.byte_size()), (4, 16));
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
pub trait IOBase<T>: RawIOBase {
    /// Convert `value` to the bytes that represent it in this resource.
    fn value_to_bytes(&self, value: &T) -> Vec<u8>;

    /// The number of `T` items in the resource.
    fn size(&self) -> usize;

    /// The number of `T` items the resource can hold without reallocating.
    /// Defaults to [`size`](IOBase::size) for resources that do not over-allocate.
    fn capacity(&self) -> usize {
        self.size()
    }

    /// Request room for `capacity` items, returning the resulting capacity.
    ///
    /// The request is a hint: the default leaves the allocation unchanged (and logs
    /// the skip). It never changes [`size`](IOBase::size).
    fn resize_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        crate::log_event!(
            warn,
            "IOBase::resize_capacity({capacity}) ignored: fixed allocation"
        );
        let _ = capacity;
        Ok(self.capacity())
    }

    /// Set the number of `T` items in the resource, truncating or zero-filling.
    fn resize(&mut self, size: usize) -> Result<(), IOError>;

    /// Write one `T` at `position` relative to `whence`, as its bytes.
    fn pwrite_one(&mut self, position: usize, whence: Whence, value: &T) -> Result<(), IOError> {
        crate::log_event!(
            trace,
            "IOBase::pwrite_one position={position} whence={whence:?}"
        );
        let bytes = self.value_to_bytes(value);
        self.pwrite_byte_array(position, whence, &bytes)
    }

    /// Write each `T` in `values` consecutively, starting at `position` relative to
    /// `whence`.
    fn pwrite_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[T],
    ) -> Result<(), IOError> {
        crate::log_event!(
            trace,
            "IOBase::pwrite_array position={position} whence={whence:?} count={}",
            values.len()
        );
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| self.value_to_bytes(value))
            .collect();
        self.pwrite_byte_array(position, whence, &bytes)
    }
}
