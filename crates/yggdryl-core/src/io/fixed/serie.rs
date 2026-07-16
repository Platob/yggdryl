//! [`Serie`] — a nullable column of fixed-width `T`: a validity bitmap over a values
//! [`Buffer`] — and the [`FixedSerie`] sub-trait of the root [`SerieType`](crate::io::SerieType).

use super::{Buffer, Field, NativeType, PrimitiveType, Scalar, TypedField};
use crate::io::bitmap::{extend_validity, Bitmap};
use crate::io::field_carrier::field_accessors;
use crate::io::{AnyField, Bytes, IOBase, IOCursor, IoError, SerieType};

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
    /// The column's own leaf [`Field`] descriptor — its name, declared nullability, and metadata.
    /// Excluded from value identity and the byte codec (only its dtype params, fixed at `T`, and the
    /// data participate). The single source of truth for the column's schema intent.
    field: Field,
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
            field: Field::of("", T::TYPE_ID, T::WIDTH, false),
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
            field: Field::of("", T::TYPE_ID, T::WIDTH, false),
        }
    }

    /// An empty column that can grow to `capacity` elements before its first reallocation.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            validity: None,
            values: Buffer::with_capacity(capacity),
            len: 0,
            field: Field::of("", T::TYPE_ID, T::WIDTH, false),
        }
    }

    /// A column from raw little-endian value bytes (`len * T::WIDTH`, zero placeholders under nulls)
    /// and an optional validity mask — the low-level constructor the erased
    /// [`Column`](crate::io::nested::Column) uses to rebuild a *wide* (non-Arrow-native) `Serie`
    /// from an imported Arrow array's bytes. Crate-internal, Arrow-only.
    #[cfg(feature = "arrow")]
    pub(crate) fn from_byte_slice(
        values: Vec<u8>,
        validity: Option<crate::io::bitmap::Bitmap>,
        len: usize,
    ) -> Self {
        Self {
            validity,
            values: Buffer::from_byte_vec(values),
            len,
            field: Field::of("", T::TYPE_ID, T::WIDTH, false),
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

    field_accessors!();

    /// The erased [`AnyField`] this column contributes — its **held field** (name + metadata) with
    /// the **effective** nullability folded in.
    ///
    /// DESIGN: the surfaced nullability is `self.nullable() || self.has_nulls()` (declared OR the
    /// column currently holds nulls) — a lenient, Arrow-standard over-approximation, so it is
    /// always a safe field for the data (a null-bearing column is never described as non-nullable).
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.has_nulls());
        AnyField::leaf(field)
    }

    /// Like [`field`](Serie::field) but **consumes** the column, moving the held field (its name and
    /// metadata) into the result with no clone — the zero-copy hand-off.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.has_nulls();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field)
    }

    /// A [`TypedField`] naming a column of this serie's type with **explicit** nullability.
    pub fn typed_field(&self, name: &str, nullable: bool) -> TypedField<T> {
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

    /// The raw contiguous values as a **zero-copy** slice — the analytics fast-path. Length is
    /// [`len`](Serie::len); **null positions hold a placeholder** (`T::default()`), so pair this
    /// with [`iter`](Serie::iter) / [`null_count`](Serie::null_count) when nulls matter, or use it
    /// directly for a vectorized kernel that reads a separate null mask. Zero-copy over the value
    /// buffer (see [`Buffer::as_slice`](crate::io::fixed::Buffer::as_slice); panics only on the
    /// externally-misaligned Arrow-import path — use [`iter`](Serie::iter) there).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let col = Serie::from_values(&[10i32, 20, 30]);
    /// assert_eq!(col.values(), &[10, 20, 30]);
    /// ```
    pub fn values(&self) -> &[T] {
        self.values.as_slice()
    }

    /// Iterates the elements as `Option`s, in order — **allocation-free** (unlike
    /// [`to_options`](Serie::to_options), which collects a `Vec`). A null yields `None`. Decodes
    /// each element (so it is safe on the misaligned Arrow-import path, unlike
    /// [`values`](Serie::values)).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    /// assert_eq!(col.iter().collect::<Vec<_>>(), [Some(1), None, Some(3)]);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = Option<T>> + '_ {
        (0..self.len).map(move |index| self.get(index))
    }

    /// Iterates only the **present** (non-null) elements, in order — allocation-free.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    /// assert_eq!(col.iter_valid().collect::<Vec<_>>(), [1, 3]);
    /// ```
    pub fn iter_valid(&self) -> impl Iterator<Item = T> + '_ {
        self.iter().flatten()
    }

    /// The values as an **element-aligned** Arrow buffer — **zero-copy** (a shared `Arc`) when the
    /// backing bytes are already aligned to `T` (every yggdryl-produced column is), else realigned
    /// with one copy. The erased [`AnySerie`](crate::io::AnySerie) maps any primitive column to its
    /// Arrow array from this buffer + the id's Arrow data type, so it is zero-copy uniformly (native
    /// *and* wide integers), with no per-type code.
    #[cfg(feature = "arrow")]
    pub(crate) fn arrow_value_buffer(&self) -> arrow_buffer::Buffer {
        let buffer = self.values.arrow_bytes();
        if buffer.as_ptr().align_offset(core::mem::align_of::<T>()) == 0 {
            buffer
        } else {
            arrow_buffer::Buffer::from(buffer.as_slice())
        }
    }

    /// The validity bitmap, if any — the null shape the erased [`AnySerie`](crate::io::AnySerie)
    /// reads for the Arrow null buffer.
    #[cfg(feature = "arrow")]
    pub(crate) fn validity_bitmap(&self) -> Option<&crate::io::bitmap::Bitmap> {
        self.validity.as_ref()
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

    /// A column from a slice of [`Scalar`]s — the plural of [`from_scalar`](Serie::from_scalar),
    /// each scalar contributing its value (or a null). The inverse of collecting a column's
    /// [`get_scalar`](Serie::get_scalar)s, and the bulk analogue of the in-place
    /// [`set_scalars`](Serie::set_scalars).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{Scalar, Serie};
    ///
    /// let col = Serie::from_scalars(&[Scalar::of(1i32), Scalar::null(), Scalar::of(3)]);
    /// assert_eq!(col.to_options(), [Some(1), None, Some(3)]);
    /// ```
    pub fn from_scalars(scalars: &[Scalar<T>]) -> Self {
        Self::from_options(&scalars.iter().map(Scalar::value).collect::<Vec<_>>())
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

    // ---- grow: append single + bulk (the mutator vocabulary) ----------------------------

    /// Appends `count` optional values in **one pass**: builds the appended value bytes into one
    /// pre-sized buffer and commits them with a **single** copy-on-write append of the values buffer
    /// (not one re-seal per element — which would be O(n²) over a bulk grow), materializing the
    /// validity mask only if a null appears. Shared by every `extend_*`.
    fn extend_with(&mut self, count: usize, mut next: impl FnMut(usize) -> Option<T>) {
        if count == 0 {
            return; // an empty grow is a no-op (no COW, no mask churn)
        }
        let base = self.len;
        let mut patch = Vec::with_capacity(count * T::WIDTH);
        let mut scratch = [0u8; MAX_WIDTH];
        for offset in 0..count {
            let value = next(offset);
            value.unwrap_or_default().write_le(&mut scratch); // placeholder bytes under a null
            patch.extend_from_slice(&scratch[..T::WIDTH]);
            match value {
                Some(_) => {
                    if let Some(validity) = &mut self.validity {
                        validity.push(true);
                    }
                }
                None => {
                    self.validity
                        .get_or_insert_with(|| Bitmap::all_present(base + offset))
                        .push(false);
                }
            }
        }
        // One COW append of the values buffer for the whole contiguous range.
        self.values.pwrite((base * T::WIDTH) as u64, &patch);
        self.len += count;
    }

    /// Appends a slice of **present** native values (no nulls) — the bulk twin of
    /// [`set_values`](Serie::set_values) that grows the column. One copy-on-write of the values
    /// buffer; the validity mask is touched only if the column already carries nulls.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let mut col = Serie::from_values(&[1i32, 2]);
    /// col.extend_values(&[3, 4, 5]);
    /// assert_eq!(col.to_options(), [Some(1), Some(2), Some(3), Some(4), Some(5)]);
    /// ```
    pub fn extend_values(&mut self, values: &[T]) {
        self.extend_with(values.len(), |offset| Some(values[offset]));
    }

    /// Appends a slice of **optional** values — the bulk twin of
    /// [`from_options`](Serie::from_options). A null in the slice lazily materializes the validity
    /// mask; the values commit in one copy-on-write.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let mut col = Serie::from_values(&[1i32]);
    /// col.extend_options(&[Some(2), None, Some(4)]);
    /// assert_eq!(col.to_options(), [Some(1), Some(2), None, Some(4)]);
    /// assert_eq!(col.null_count(), 1);
    /// ```
    pub fn extend_options(&mut self, values: &[Option<T>]) {
        self.extend_with(values.len(), |offset| values[offset]);
    }

    /// Appends a slice of [`Scalar`]s (each its value or a null) — the bulk twin of
    /// [`from_scalars`](Serie::from_scalars), reusing the one-pass grow over each scalar's value.
    pub fn extend_scalars(&mut self, scalars: &[Scalar<T>]) {
        self.extend_with(scalars.len(), |offset| scalars[offset].value());
    }

    /// Appends **another whole column** of the same type to this one — the two columns concatenate.
    /// The source's raw value bytes are appended with a **single** copy-on-write (a memcpy, not a
    /// per-element re-encode) and its null positions are carried over in the same pass. Infallible:
    /// a fixed-width column has no descriptor to reconcile (`T` is the same by construction).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let mut a = Serie::from_options(&[Some(1i32), None]);
    /// let b = Serie::from_values(&[3, 4]);
    /// a.concat(&b);
    /// assert_eq!(a.to_options(), [Some(1), None, Some(3), Some(4)]);
    /// ```
    pub fn concat(&mut self, source: &Serie<T>) {
        if source.len == 0 {
            return;
        }
        let base = self.len;
        // One COW append: the source's value bytes memcpy straight in (placeholder bytes under its
        // nulls are canonical zeros, so the appended null slots stay canonical).
        self.values
            .pwrite((base * T::WIDTH) as u64, source.values.as_bytes());
        extend_validity(&mut self.validity, base, source.len, |offset| {
            source.validity.as_ref().is_none_or(|mask| mask.get(offset))
        });
        self.len += source.len;
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
            let bits = source.read_exact_vec(len.div_ceil(8))?;
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
        let value_bytes = source.read_exact_vec(byte_len)?;
        Ok(Self {
            validity,
            values: Buffer::from_bytes(&value_bytes),
            len,
            field: Field::of("", T::TYPE_ID, T::WIDTH, false),
        })
    }

    /// This column's canonical bytes — the same `[len][flags][validity?][values]` frame
    /// [`write_to`](Serie::write_to) produces, returned as an owned `Vec`. The exact inverse of
    /// [`deserialize_bytes`](Serie::deserialize_bytes), and the codec the Python / Node bindings
    /// expose (`serialize_bytes` / `serializeBytes`).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Serie;
    ///
    /// let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    /// assert_eq!(Serie::<i32>::deserialize_bytes(&col.serialize_bytes()).unwrap(), col);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a column from the bytes produced by
    /// [`serialize_bytes`](Serie::serialize_bytes), erroring ([`IoError::UnexpectedEof`] /
    /// [`IoError::CorruptLength`]) on a truncated or corrupt frame.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }
}

impl<T: NativeType> Default for Serie<T> {
    fn default() -> Self {
        Self {
            validity: None,
            values: Buffer::new(),
            len: 0,
            field: Field::of("", T::TYPE_ID, T::WIDTH, false),
        }
    }
}

// Value identity is **byte-wise**, unconditional over `T: NativeType`: two columns are equal iff
// their lengths, null positions, and raw value bytes match. `Buffer<T>` already compares by its
// bytes (not `T`'s `==`), so this is bit-canonical and works for the float types too — a manual
// impl (not a derive) because a derive would spuriously require `T: Eq`, which the floats lack.
//
// A fully-present column compares equal whether or not its validity mask is *materialized*: an
// absent mask (`None`) and a materialized all-present one (`Some(all-true)`, left behind after a
// `set` clears the last null) denote the same value. This keeps identity in lock-step with the
// byte codec, whose [`write_to`](Serie::write_to) already drops an all-present mask — so
// `deserialize_bytes(serialize_bytes(x)) == x` holds for every column.
impl<T: NativeType> PartialEq for Serie<T> {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len || self.values != other.values {
            return false;
        }
        match (self.has_nulls(), other.has_nulls()) {
            (false, false) => true, // both fully present (mask or not)
            (true, true) => self.validity == other.validity, // same null positions
            _ => false,             // one has nulls, the other doesn't
        }
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
            field: Field::of("", T::TYPE_ID, T::WIDTH, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equality_ignores_a_materialized_all_present_mask() {
        // Clearing the last null with `set` leaves a materialized all-present validity mask; the
        // column must still equal (and round-trip byte-equal to) the same values with no mask.
        let mut cleared = Serie::from_options(&[Some(1i32), None, Some(3)]);
        cleared.set(1, Some(2)).unwrap();
        assert_eq!(cleared.null_count(), 0);

        let dense = Serie::from_values(&[1i32, 2, 3]);
        assert_eq!(cleared, dense);
        assert_eq!(
            Serie::<i32>::deserialize_bytes(&cleared.serialize_bytes()).unwrap(),
            cleared
        );

        // A genuine null must still make the columns differ.
        let with_null = Serie::from_options(&[Some(1i32), None, Some(3)]);
        assert_ne!(with_null, dense);
    }

    #[test]
    fn from_scalars_round_trips_a_column_through_its_own_scalars() {
        let col = Serie::from_options(&[Some(1i32), None, Some(3), Some(4)]);
        let scalars: Vec<_> = (0..col.len()).map(|i| col.get_scalar(i)).collect();
        assert_eq!(Serie::from_scalars(&scalars), col);

        // The empty slice yields the empty column.
        assert_eq!(Serie::<i32>::from_scalars(&[]), Serie::new());
    }
}
