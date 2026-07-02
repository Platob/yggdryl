//! The typed [`IOBase`] layer over [`RawIOBase`](super::RawIOBase).

use super::{IOCursor, IOError, IOSlice, RawIOBase, Whence};

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
///     fn element_width(&self) -> usize {
///         4 // fixed width, so typed views and streams work even when empty
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
///
/// // Optimized typed streaming: copy two items into another resource, element-aligned
/// // and without deserializing them.
/// let mut sink = Mem::default();
/// mem.pread_typed_io(0, Whence::Start, 2, &mut sink, 0, Whence::Start)?;
/// assert_eq!(sink.byte_size(), 8); // two u32 items == eight bytes
/// assert_eq!(sink.pread_byte_array(0, Whence::Start, 8)?, vec![1, 2, 3, 4, 5, 6, 7, 8]);
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
pub trait IOBase<T>: RawIOBase {
    /// Convert `value` to the bytes that represent it in this resource.
    fn value_to_bytes(&self, value: &T) -> Vec<u8>;

    /// The number of `T` items in the resource.
    fn size(&self) -> usize;

    /// The fixed byte width of a single `T` in this resource.
    ///
    /// The default infers it as [`byte_size`](RawIOBase::byte_size) divided by
    /// [`size`](IOBase::size) — exact for fixed-width items — and is `0` for an empty
    /// resource. Implementors whose items have a constant width should override it
    /// with that constant, so a derived view such as [`IOSlice`](super::IOSlice) can
    /// convert item counts to bytes even when the resource is empty.
    fn element_width(&self) -> usize {
        self.byte_size().checked_div(self.size()).unwrap_or(0)
    }

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

    /// Stream `size` items from `self` (at item `position` relative to `whence`) into
    /// `sink` (at item `sink_position` relative to `sink_whence`).
    ///
    /// This is the typed counterpart of
    /// [`pread_raw_io`](RawIOBase::pread_raw_io): item offsets are scaled to bytes by
    /// this resource's [`element_width`](IOBase::element_width) and the element-aligned
    /// bytes are copied in chunks, so a large transfer never materializes in full and
    /// no item is serialized or deserialized. `sink` must therefore share this
    /// resource's element width and byte layout. A non-zero `size` over a resource
    /// whose width is indeterminate returns
    /// [`IOError::IndeterminateElementWidth`].
    fn pread_typed_io(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
        sink: &mut dyn RawIOBase,
        sink_position: usize,
        sink_whence: Whence,
    ) -> Result<(), IOError> {
        crate::log_event!(debug, "IOBase::pread_typed_io size={size}");
        let width = self.element_width();
        if size != 0 && width == 0 {
            return Err(IOError::IndeterminateElementWidth);
        }
        self.pread_raw_io(
            position.saturating_mul(width),
            whence,
            size.saturating_mul(width),
            sink,
            sink_position.saturating_mul(width),
            sink_whence,
        )
    }

    /// Stream `size` items from `source` (at item `source_position` relative to
    /// `source_whence`) into `self` (at item `position` relative to `whence`).
    ///
    /// The typed counterpart of [`pwrite_raw_io`](RawIOBase::pwrite_raw_io): item
    /// offsets are scaled to bytes by this resource's
    /// [`element_width`](IOBase::element_width) and the element-aligned bytes are
    /// copied in chunks. `source` must share this resource's element width and byte
    /// layout; a non-zero `size` over a resource whose width is indeterminate returns
    /// [`IOError::IndeterminateElementWidth`].
    fn pwrite_typed_io(
        &mut self,
        position: usize,
        whence: Whence,
        source: &dyn RawIOBase,
        source_position: usize,
        source_whence: Whence,
        size: usize,
    ) -> Result<(), IOError> {
        crate::log_event!(debug, "IOBase::pwrite_typed_io size={size}");
        let width = self.element_width();
        if size != 0 && width == 0 {
            return Err(IOError::IndeterminateElementWidth);
        }
        self.pwrite_raw_io(
            position.saturating_mul(width),
            whence,
            source,
            source_position.saturating_mul(width),
            source_whence,
            size.saturating_mul(width),
        )
    }

    /// Consume this resource into an [`IOCursor`], a moving typed cursor over it
    /// that advances on every read and write.
    ///
    /// A type that implements both [`RawIOBase`] and [`IOBase`] carries this and
    /// [`RawIOBase::cursor`]; call it as `IOBase::<T>::cursor(resource)` to pick the
    /// typed one.
    fn cursor(self) -> IOCursor<Self>
    where
        Self: Sized,
    {
        IOCursor::new(self)
    }

    /// Consume this resource into an [`IOSlice`], a typed view bounded to the byte
    /// window `[start, end)`. Disambiguate from [`RawIOBase::slice`] as
    /// `IOBase::<T>::slice(resource, start, end)`.
    fn slice(self, start: usize, end: usize) -> IOSlice<Self>
    where
        Self: Sized,
    {
        IOSlice::new(self, start, end)
    }
}
