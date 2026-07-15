//! [`Serie`] — a nullable column of fixed-width `T`: a validity bitmap over a values
//! [`Buffer`] — and the [`FixedSerie`] sub-trait of the root [`SerieType`](crate::io::SerieType).

use super::{Buffer, NativeType, PrimitiveType, Scalar, TypedField};
use crate::io::bitmap::Bitmap;
use crate::io::{IOBase, IOCursor, IoError, SerieType};

/// The largest fixed-width primitive is 32 bytes (`u256`/`i256`); a stack scratch of this size
/// encodes one value with no allocation while building a column's raw bytes in one pass.
const MAX_WIDTH: usize = 32;

/// The **fixed-width column** sub-trait — a [`SerieType`] over a [`NativeType`], with the
/// descriptor mutualized as a default method.
pub trait FixedSerie: SerieType {
    /// The native element type.
    type Native: NativeType;

    /// The typed data type of the column — mutualized default.
    fn data_type(&self) -> PrimitiveType<Self::Native> {
        PrimitiveType::new()
    }
}

/// A **nullable column** of fixed-width `T` — Arrow-style: an optional validity bitmap over a
/// contiguous values [`Buffer`]. `Serie<u8> = U8Serie`. With no nulls the validity mask is
/// absent (zero overhead); a null slot keeps a placeholder value so the values stay
/// contiguous. The whole column reads and writes through the [`IOCursor`] abstraction.
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::{Bytes, IOCursor};
///
/// let mut col = Serie::<i32>::new();
/// col.push(Some(1));
/// col.push(None);
/// col.push(Some(3));
/// assert_eq!(col.len(), 3);
/// assert_eq!(col.null_count(), 1);
/// assert_eq!(col.get(1), None);
/// assert_eq!(col.get(2), Some(3));
///
/// // Round-trips through any byte sink.
/// let mut sink = Bytes::new();
/// col.write_to(&mut sink).unwrap();
/// sink.rewind();
/// assert_eq!(Serie::<i32>::read_from(&mut sink).unwrap(), col);
/// ```
#[derive(Debug, Clone)]
pub struct Serie<T: NativeType> {
    /// `None` means "no nulls" — every element is present.
    validity: Option<Bitmap>,
    /// The contiguous values; `values.count() == len`.
    values: Buffer<T>,
    len: usize,
}

impl<T: NativeType> Serie<T> {
    /// An empty column.
    pub fn new() -> Self {
        Self::default()
    }

    /// A non-null column from `values` (no validity mask).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let col = Serie::from_values(&[10i32, 20, 30]);
    /// assert_eq!(col.len(), 3);
    /// assert_eq!(col.null_count(), 0);
    /// ```
    pub fn from_values(values: &[T]) -> Self {
        Self {
            validity: None,
            values: Buffer::from_slice(values),
            len: values.len(),
        }
    }

    /// A column from `Option` values, materializing a validity mask only if a null appears.
    ///
    /// DESIGN: builds the raw value **bytes** and the validity bitmap in **one pass**, then
    /// hands the byte `Vec` to [`Buffer::from_byte_vec`](Buffer::from_byte_vec) with no copy — so
    /// it allocates a small constant number of times (and works for every [`NativeType`],
    /// including the wide non-Arrow-native ones), unlike a `push` loop that would round-trip the
    /// immutable buffer per element.
    pub fn from_options(values: &[Option<T>]) -> Self {
        let len = values.len();
        let mut bytes = Vec::with_capacity(len * T::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        let mut validity: Option<Bitmap> = None;
        for (index, value) in values.iter().enumerate() {
            match value {
                Some(value) => {
                    value.write_le(&mut scratch);
                    if let Some(bitmap) = &mut validity {
                        bitmap.push(true);
                    }
                }
                None => {
                    T::default().write_le(&mut scratch); // placeholder keeps the values contiguous
                    validity
                        .get_or_insert_with(|| Bitmap::all_present(index))
                        .push(false);
                }
            }
            bytes.extend_from_slice(&scratch[..T::WIDTH]);
        }
        Self {
            validity,
            values: Buffer::from_byte_vec(bytes),
            len,
        }
    }

    /// An empty column that can grow to `capacity` elements before its first reallocation.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            validity: None,
            values: Buffer::with_capacity(capacity),
            len: 0,
        }
    }

    /// Appends one element (`None` is a null). A null lazily materializes the validity mask.
    /// For building from a known set, prefer [`from_values`](Serie::from_values) /
    /// [`from_options`](Serie::from_options), which build the values in one pass instead of
    /// re-sealing the immutable buffer per element.
    pub fn push(&mut self, value: Option<T>) {
        match value {
            Some(value) => {
                self.values.push(value);
                if let Some(validity) = &mut self.validity {
                    validity.push(true);
                }
            }
            None => {
                self.values.push(T::default()); // placeholder keeps the values contiguous
                self.validity
                    .get_or_insert_with(|| Bitmap::all_present(self.len))
                    .push(false);
            }
        }
        self.len += 1;
    }

    /// The element at `index`, or `None` if it is null or out of range.
    pub fn get(&self, index: usize) -> Option<T> {
        if index >= self.len {
            return None;
        }
        match &self.validity {
            Some(validity) if !validity.get(index) => None,
            _ => self.values.get(index),
        }
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of null elements.
    pub fn null_count(&self) -> usize {
        self.validity.as_ref().map_or(0, Bitmap::null_count)
    }

    /// Whether the column carries any nulls.
    pub fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The typed data type of the column — a zero-cost `const` descriptor.
    pub const fn data_type(&self) -> PrimitiveType<T> {
        PrimitiveType::new()
    }

    /// A [`TypedField`] naming a column of this serie's type with explicit nullability.
    pub fn field(&self, name: &str, nullable: bool) -> TypedField<T> {
        TypedField::new(name, nullable)
    }

    /// A [`TypedField`] naming this column, its nullability **inferred** from whether the
    /// column currently holds any nulls.
    pub fn to_field(&self, name: &str) -> TypedField<T> {
        TypedField::new(name, self.has_nulls())
    }

    /// The elements as `Option`s, in order.
    pub fn to_options(&self) -> Vec<Option<T>> {
        (0..self.len).map(|i| self.get(i)).collect()
    }

    // ---- scalar interop: a column is usable as a scalar and vice versa ----------------

    /// The element at `index` as a [`Scalar`] — null if the element is null or out of range.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{Scalar, Serie};
    ///
    /// let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    /// assert_eq!(col.get_scalar(0), Scalar::of(1));
    /// assert_eq!(col.get_scalar(1), Scalar::null());
    /// assert_eq!(col.get_scalar(9), Scalar::null()); // out of range
    /// ```
    pub fn get_scalar(&self, index: usize) -> Scalar<T> {
        Scalar::new(self.get(index))
    }

    /// This column **viewed as a single [`Scalar`]**, if it holds exactly one element — so a
    /// length-1 serie is usable wherever a scalar is expected.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{Scalar, Serie};
    ///
    /// assert_eq!(Serie::from_values(&[42i32]).as_scalar(), Some(Scalar::of(42)));
    /// assert_eq!(Serie::from_values(&[1i32, 2]).as_scalar(), None); // not a scalar
    /// ```
    pub fn as_scalar(&self) -> Option<Scalar<T>> {
        (self.len == 1).then(|| self.get_scalar(0))
    }

    /// A length-1 column broadcasting `scalar` (the inverse of [`as_scalar`](Serie::as_scalar)).
    pub fn from_scalar(scalar: Scalar<T>) -> Self {
        Self::from_options(&[scalar.value()])
    }

    // ---- in-place set: single element + bulk (from a Serie / scalars / native values) --------

    /// Overwrites element `index` in place — `Some` writes a value, `None` a null (lazily
    /// materializing the validity mask). Errors [`IndexOutOfBounds`](IoError::IndexOutOfBounds) if
    /// `index` is not an existing element (`set` replaces; use [`push`](Serie::push) to grow).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let mut col = Serie::from_values(&[1i32, 2, 3]);
    /// col.set(1, Some(20)).unwrap();
    /// col.set(2, None).unwrap();
    /// assert_eq!(col.to_options(), [Some(1), Some(20), None]);
    /// assert!(col.set(9, Some(0)).is_err()); // out of bounds
    /// ```
    pub fn set(&mut self, index: usize, value: Option<T>) -> Result<(), IoError> {
        if index >= self.len {
            return Err(IoError::IndexOutOfBounds {
                index,
                len: self.len,
            });
        }
        match value {
            Some(value) => {
                self.values.set(index, value);
                if let Some(validity) = &mut self.validity {
                    validity.set(index, true);
                }
            }
            None => {
                self.values.set(index, T::default()); // canonical placeholder under the null
                self.validity
                    .get_or_insert_with(|| Bitmap::all_present(self.len))
                    .set(index, false);
            }
        }
        Ok(())
    }

    /// Overwrites element `index` from a [`Scalar`] (its value or null).
    pub fn set_scalar(&mut self, index: usize, scalar: &Scalar<T>) -> Result<(), IoError> {
        self.set(index, scalar.value())
    }

    /// Bounds-checks a bulk range `[start, start + count)` against the column length.
    fn check_range(&self, start: usize, count: usize) -> Result<(), IoError> {
        match start.checked_add(count) {
            Some(end) if end <= self.len => Ok(()),
            _ => Err(IoError::IndexOutOfBounds {
                index: start.max(self.len),
                len: self.len,
            }),
        }
    }

    /// Writes `count` optional values into `[start, start + count)` in **one pass**: builds the
    /// contiguous value bytes and commits them with a **single** copy-on-write of the values buffer
    /// (not one re-seal per element), materializing the validity mask only if a null appears.
    fn set_options_range(
        &mut self,
        start: usize,
        count: usize,
        mut next: impl FnMut(usize) -> Option<T>,
    ) {
        let mut patch = Vec::with_capacity(count * T::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        for offset in 0..count {
            let value = next(offset);
            value.unwrap_or_default().write_le(&mut scratch); // placeholder bytes under a null
            patch.extend_from_slice(&scratch[..T::WIDTH]);
            match value {
                Some(_) => {
                    if let Some(validity) = &mut self.validity {
                        validity.set(start + offset, true);
                    }
                }
                None => {
                    self.validity
                        .get_or_insert_with(|| Bitmap::all_present(self.len))
                        .set(start + offset, false);
                }
            }
        }
        // One COW of the values buffer for the whole contiguous range.
        self.values.pwrite((start * T::WIDTH) as u64, &patch);
    }

    /// Bulk-overwrites `[start, start + source.len())` from another column, element-for-element
    /// (nulls included). Errors if the range runs past the end (the column is left unchanged).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let mut col = Serie::from_values(&[0i32, 0, 0, 0]);
    /// let patch = Serie::from_options(&[Some(7), None]);
    /// col.set_range(1, &patch).unwrap();
    /// assert_eq!(col.to_options(), [Some(0), Some(7), None, Some(0)]);
    /// ```
    pub fn set_range(&mut self, start: usize, source: &Serie<T>) -> Result<(), IoError> {
        self.check_range(start, source.len())?;
        self.set_options_range(start, source.len(), |offset| source.get(offset));
        Ok(())
    }

    /// Bulk-overwrites `[start, start + scalars.len())` from a slice of [`Scalar`]s.
    pub fn set_scalars(&mut self, start: usize, scalars: &[Scalar<T>]) -> Result<(), IoError> {
        self.check_range(start, scalars.len())?;
        self.set_options_range(start, scalars.len(), |offset| scalars[offset].value());
        Ok(())
    }

    /// Bulk-overwrites `[start, start + values.len())` from present native values.
    pub fn set_values(&mut self, start: usize, values: &[T]) -> Result<(), IoError> {
        self.check_range(start, values.len())?;
        self.set_options_range(start, values.len(), |offset| Some(values[offset]));
        Ok(())
    }

    /// Writes this column to `sink` — `[len: u64][flags: u8][validity?][values]` — advancing
    /// its cursor. The validity mask is written only when the column has nulls.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        sink.write_all(&(self.len as u64).to_le_bytes())?;
        let has_validity = self.has_nulls();
        sink.write_all(&[u8::from(has_validity)])?;
        if has_validity {
            // `has_nulls` implies `validity` is `Some`.
            sink.write_all(self.validity.as_ref().unwrap().as_bytes())?;
        }
        sink.write_all(self.values.as_bytes())
    }

    /// Reads a column written by [`write_to`](Serie::write_to) from `source`, advancing its
    /// cursor. Errors ([`IoError::UnexpectedEof`]) if the frame is truncated.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let mut header = [0u8; 9];
        source.read_exact(&mut header)?;
        let len = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
        let has_validity = header[8] != 0;

        let validity = if has_validity {
            let mut bits = vec![0u8; len.div_ceil(8)];
            source.read_exact(&mut bits)?;
            Some(Bitmap::from_bytes(&bits, len))
        } else {
            None
        };

        // Guard against a corrupt/hostile length: `len * WIDTH` must not overflow `usize`
        // before it is used to size the read buffer.
        let byte_len = len.checked_mul(T::WIDTH).ok_or(IoError::CorruptLength {
            len: len as u64,
            width: T::WIDTH,
        })?;
        let mut value_bytes = vec![0u8; byte_len];
        source.read_exact(&mut value_bytes)?;
        Ok(Self {
            validity,
            values: Buffer::from_bytes(&value_bytes),
            len,
        })
    }
}

impl<T: NativeType> Default for Serie<T> {
    fn default() -> Self {
        Self {
            validity: None,
            values: Buffer::new(),
            len: 0,
        }
    }
}

// Value identity is **byte-wise**, unconditional over `T: NativeType`: two columns are equal iff
// their lengths, validity masks, and raw value bytes match. `Buffer<T>` already compares by its
// bytes (not `T`'s `==`), so this is bit-canonical and works for the float types too — a manual
// impl (not a derive) because a derive would spuriously require `T: Eq`, which the floats lack.
impl<T: NativeType> PartialEq for Serie<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.validity == other.validity && self.values == other.values
    }
}

impl<T: NativeType> Eq for Serie<T> {}

impl<T: NativeType> FromIterator<Option<T>> for Serie<T> {
    fn from_iter<I: IntoIterator<Item = Option<T>>>(iter: I) -> Self {
        // Collect once, then take the single-pass bulk path (not a per-element `push` loop,
        // which would re-allocate the immutable values buffer every element).
        let values: Vec<Option<T>> = iter.into_iter().collect();
        Self::from_options(&values)
    }
}

// The trait-hierarchy impls: `Serie<T>` is the fixed implementation of `SerieType`. Bodies
// delegate to the inherent methods (inherent resolves before trait, so no recursion).
impl<T: NativeType> SerieType for Serie<T> {
    type Elem = T;

    fn len(&self) -> usize {
        self.len()
    }

    fn null_count(&self) -> usize {
        self.null_count()
    }

    fn get(&self, index: usize) -> Option<T> {
        self.get(index)
    }
}

impl<T: NativeType> FixedSerie for Serie<T> {
    type Native = T;
}

/// Zero-copy interop with Arrow's [`PrimitiveArray`](arrow_array::PrimitiveArray) (feature
/// `arrow`). Our validity bitmap is LSB-first with `1 = valid`, byte-identical to Arrow's
/// [`NullBuffer`](arrow_buffer::NullBuffer), and the values share the `Arc` allocation — so
/// the **values** convert with no copy (the small validity mask is rebuilt).
#[cfg(feature = "arrow")]
impl<T: super::ArrowNative> Serie<T> {
    /// This column as an Arrow [`PrimitiveArray`](arrow_array::PrimitiveArray) — the values
    /// are **zero-copy** (a shared `Arc`); the validity mask is wrapped as a `NullBuffer`.
    pub fn to_arrow_array(&self) -> arrow_array::PrimitiveArray<T::Arrow> {
        let values = arrow_buffer::ScalarBuffer::<T>::new(self.values.arrow_values(), 0, self.len);
        let nulls = self.validity.as_ref().map(|bitmap| {
            let buffer = arrow_buffer::Buffer::from(bitmap.as_bytes());
            arrow_buffer::NullBuffer::new(arrow_buffer::BooleanBuffer::new(buffer, 0, self.len))
        });
        arrow_array::PrimitiveArray::<T::Arrow>::new(values, nulls)
    }

    /// Builds a column from an Arrow [`PrimitiveArray`](arrow_array::PrimitiveArray) — the
    /// values are **zero-copy** (a shared `Arc`) when the array is dense; the validity is read
    /// back into our bitmap.
    ///
    /// DESIGN: Arrow leaves the bytes *under null slots* undefined (a real array from IPC/Parquet
    /// carries garbage there), but this crate's value identity is byte-canonical — so a null slot
    /// carrying non-default bytes is overwritten with `T::default()`. That copy-on-write happens
    /// **only** when a null slot is actually non-canonical, so a dense array — or one whose nulls
    /// are already zeroed (every yggdryl-produced array) — stays fully zero-copy, while a foreign
    /// array with garbage under its nulls is canonicalized so equal columns compare byte-equal.
    pub fn from_arrow_array(array: &arrow_array::PrimitiveArray<T::Arrow>) -> Self {
        use arrow_array::Array;
        let len = array.len();
        let mut values = Buffer::from_arrow_buffer(array.values().inner().clone());
        let validity = array.nulls().map(|_| {
            // The canonical placeholder bytes for a null slot.
            let mut default_bytes = [0u8; MAX_WIDTH];
            T::default().write_le(&mut default_bytes);
            let default_slot = &default_bytes[..T::WIDTH];

            let mut bits = vec![0u8; len.div_ceil(8)];
            for index in 0..len {
                if array.is_valid(index) {
                    bits[index / 8] |= 1 << (index % 8);
                } else {
                    let start = index * T::WIDTH;
                    if &values.as_bytes()[start..start + T::WIDTH] != default_slot {
                        values.set(index, T::default()); // canonicalize a non-default null slot
                    }
                }
            }
            Bitmap::from_bytes(&bits, len)
        });
        Self {
            validity,
            values,
            len,
        }
    }
}
