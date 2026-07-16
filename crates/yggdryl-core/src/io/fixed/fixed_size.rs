//! The **fixed-size byte** family: values that are all exactly `N` bytes (Arrow's
//! `FixedSizeBinary(N)`), with `N` carried at **runtime**. Unlike the numeric primitives (whose
//! width is a compile-time `NativeType::WIDTH`) and the variable-length [`var`](crate::io::var)
//! types (offsets + data), a fixed-size column is a flat `N`-byte-slot data buffer + an optional
//! validity bitmap — structurally the var family without the offsets.
//!
//! One generic implementation is shared by both kinds, parameterized by a [`FixedElement`]
//! marker: [`FixedBinary`](crate::io::fixed::binary::FixedBinary) (any bytes) and
//! [`FixedUtf8`](crate::io::fixed::string::FixedUtf8) (each value validated UTF-8). It
//! implements the family-agnostic root traits ([`DataType`] / [`FieldType`] / [`ScalarType`] /
//! [`SerieType`], plus [`VarScalar`] / [`VarSerie`] for the byte accessors), so the category
//! drill-down (`is_fixed_width()` **and** `is_binary()` / `is_utf8()`) works uniformly.

use core::marker::PhantomData;

use super::Field;
use crate::io::any_serie::filter_len_mismatch;
use crate::io::bitmap::{extend_validity, Bitmap};
use crate::io::field_carrier::{field_accessors, field_setters};
use crate::io::var::{VarScalar, VarSerie};
use crate::io::{
    AnyField, Bytes, DataType, DataTypeId, FieldType, Headers, IOCursor, IoError, ScalarType,
    SerieType,
};

/// The kind of a fixed-size byte value — opaque binary or UTF-8 — the way
/// [`VarElement`](crate::io::var::VarElement) distinguishes the variable-length kinds. Both map
/// to Arrow's `FixedSizeBinary(N)` (Arrow has no fixed-size UTF-8 type).
pub trait FixedElement: Send + Sync + 'static {
    /// The stable, lower-case type name (`"fixed_binary"` / `"fixed_utf8"`).
    const NAME: &'static str;
    /// The [`DataTypeId`] — [`FixedBinary`](DataTypeId::FixedBinary) or
    /// [`FixedUtf8`](DataTypeId::FixedUtf8).
    const TYPE_ID: DataTypeId;

    /// Validates one value's bytes for this kind (UTF-8 must decode; binary accepts anything).
    fn validate(bytes: &[u8]) -> Result<(), IoError>;
}

/// Reads a little-endian `u64` from a cursor.
fn read_u64<R: IOCursor>(source: &mut R) -> Result<u64, IoError> {
    let mut bytes = [0u8; 8];
    source.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

// -------------------------------------------------------------------------------------
// Descriptor
// -------------------------------------------------------------------------------------

/// The descriptor of a fixed-size byte type of kind `K`, carrying its byte width `N` at runtime.
pub struct FixedSizeType<K> {
    width: usize,
    _kind: PhantomData<K>,
}

impl<K: FixedElement> FixedSizeType<K> {
    /// A descriptor for values of exactly `width` bytes.
    pub fn new(width: usize) -> Self {
        Self {
            width,
            _kind: PhantomData,
        }
    }

    /// The fixed byte width `N`.
    pub fn width(&self) -> usize {
        self.width
    }

    /// A [`FixedSizeField`] naming a column of this type.
    pub fn field(&self, name: &str, nullable: bool) -> FixedSizeField<K> {
        FixedSizeField::new(name, self.width, nullable)
    }
}

impl<K: FixedElement> DataType for FixedSizeType<K> {
    fn name(&self) -> &'static str {
        K::NAME
    }

    fn byte_width(&self) -> usize {
        self.width
    }

    fn type_id(&self) -> DataTypeId {
        K::TYPE_ID
    }
    // `to_arrow` is the centralized `DataType` default: `FixedBinary`/`FixedUtf8` both map to
    // `FixedSizeBinary(width)` (Arrow has no fixed-size UTF-8 type).
}

// -------------------------------------------------------------------------------------
// Field
// -------------------------------------------------------------------------------------

/// A named, nullable fixed-size-byte column descriptor of kind `K` and width `N`.
pub struct FixedSizeField<K> {
    name: String,
    width: usize,
    nullable: bool,
    metadata: Headers,
    _kind: PhantomData<K>,
}

impl<K: FixedElement> FixedSizeField<K> {
    /// Builds a field from a name, its per-value byte width, and its nullability (empty metadata).
    pub fn new(name: &str, width: usize, nullable: bool) -> Self {
        Self {
            name: name.to_string(),
            width,
            nullable,
            metadata: Headers::new(),
            _kind: PhantomData,
        }
    }

    /// The field's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// A fresh field with the given metadata [`Headers`] attached.
    pub fn with_metadata(mut self, metadata: Headers) -> Self {
        self.metadata = metadata;
        self
    }

    /// A fresh field with one extra `key = value` metadata entry.
    pub fn with_metadata_entry(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key, value);
        self
    }

    field_setters!();

    /// The typed descriptor.
    pub fn data_type(&self) -> FixedSizeType<K> {
        FixedSizeType::new(self.width)
    }

    /// The erased runtime [`Field`], metadata preserved.
    pub fn erase(&self) -> Field {
        Field::new(&self.name, &self.data_type(), self.nullable)
            .with_metadata(self.metadata.clone())
    }

    /// This field as an [`arrow_schema::Field`] (feature `arrow`), via the erased [`Field`].
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
        self.erase().to_arrow()
    }

    /// Builds a fixed-size field from an [`arrow_schema::Field`] (a `FixedSizeBinary(N)`), or
    /// `None` if its metadata-refined logical type is not this kind `K` (feature `arrow`). The
    /// width `N` comes from the Arrow type; user metadata is preserved.
    #[cfg(feature = "arrow")]
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        let erased = Field::from_arrow(field)?;
        (FieldType::type_id(&erased) == K::TYPE_ID).then(|| {
            Self::new(erased.name(), erased.byte_width(), erased.nullable())
                .with_metadata(erased.metadata().clone())
        })
    }
}

impl<K: FixedElement> FieldType for FixedSizeField<K> {
    fn name(&self) -> &str {
        &self.name
    }

    fn type_name(&self) -> &'static str {
        K::NAME
    }

    fn byte_width(&self) -> usize {
        self.width
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn type_id(&self) -> DataTypeId {
        K::TYPE_ID
    }
}

// -------------------------------------------------------------------------------------
// Scalar
// -------------------------------------------------------------------------------------

/// One nullable fixed-size value of kind `K` — a present value is exactly [`width`] bytes.
pub struct FixedSizeScalar<K> {
    value: Option<Box<[u8]>>,
    /// The value's own [`FixedSizeField`] descriptor — its name, declared nullability, metadata, and
    /// the fixed byte `width`. The `width` joins the bytes in identity; the name / nullable /
    /// metadata are excluded.
    field: FixedSizeField<K>,
    _kind: PhantomData<K>,
}

impl<K: FixedElement> FixedSizeScalar<K> {
    /// A present scalar from `bytes` (its width becomes `bytes.len()`), validated for the kind.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        K::validate(bytes)?;
        Ok(Self::from_bytes_unchecked(bytes))
    }

    /// A present scalar from `bytes` **without** validation — the kind sub-modules use this for
    /// inputs known-valid by construction (any bytes for binary, a `&str` for UTF-8).
    pub(crate) fn from_bytes_unchecked(bytes: &[u8]) -> Self {
        Self {
            value: Some(bytes.into()),
            field: FixedSizeField::new("", bytes.len(), false),
            _kind: PhantomData,
        }
    }

    /// The null scalar of the given byte width.
    pub fn null(width: usize) -> Self {
        Self {
            value: None,
            field: FixedSizeField::new("", width, false),
            _kind: PhantomData,
        }
    }

    /// The fixed byte width `N` (from the held field).
    pub fn width(&self) -> usize {
        self.field.byte_width()
    }

    /// The raw bytes, or `None` if null.
    pub fn value_bytes(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    /// Whether the scalar is null.
    pub fn is_null(&self) -> bool {
        self.value.is_none()
    }

    /// The typed descriptor.
    pub fn data_type(&self) -> FixedSizeType<K> {
        FixedSizeType::new(self.width())
    }

    field_accessors!();

    /// The erased [`AnyField`] this scalar contributes — its **held field** (name + metadata + width)
    /// with **effective** nullability `self.nullable() || self.is_null()`.
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.is_null());
        AnyField::leaf(field.erase())
    }

    /// Like [`field`](FixedSizeScalar::field) but **consumes** the scalar.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.is_null();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field.erase())
    }

    /// Writes this scalar: `[width:u64][validity:u8][bytes?]`.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        sink.write_all(&(self.width() as u64).to_le_bytes())?;
        match &self.value {
            Some(bytes) => {
                sink.write_all(&[1])?;
                sink.write_all(bytes)
            }
            None => sink.write_all(&[0]),
        }
    }

    /// Reads a scalar written by [`write_to`](FixedSizeScalar::write_to), validating a present
    /// value for the kind.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let width = read_u64(source)? as usize;
        let mut validity = [0u8; 1];
        source.read_exact(&mut validity)?;
        if validity[0] == 0 {
            return Ok(Self::null(width));
        }
        let mut bytes = vec![0u8; width];
        source.read_exact(&mut bytes)?;
        K::validate(&bytes)?;
        Ok(Self {
            value: Some(bytes.into_boxed_slice()),
            field: FixedSizeField::new("", width, false),
            _kind: PhantomData,
        })
    }
}

impl<K: FixedElement> ScalarType for FixedSizeScalar<K> {
    type Data = FixedSizeType<K>;

    fn data_type(&self) -> FixedSizeType<K> {
        FixedSizeType::new(self.width())
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }
}

impl<K: FixedElement> VarScalar for FixedSizeScalar<K> {
    fn value_bytes(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }
}

// -------------------------------------------------------------------------------------
// Serie (column)
// -------------------------------------------------------------------------------------

/// A nullable column of fixed-size values of kind `K` — a flat `N`-byte-slot data buffer over
/// an optional validity bitmap. Value `i` is `data[i * width .. (i + 1) * width]`.
pub struct FixedSizeSerie<K> {
    data: Vec<u8>,
    validity: Option<Bitmap>,
    len: usize,
    /// The column's own [`FixedSizeField`] descriptor — its name, declared nullability, metadata, and
    /// the fixed byte `width`. The `width` joins the data in value identity and the byte codec; the
    /// name / nullable / metadata are excluded.
    field: FixedSizeField<K>,
    _kind: PhantomData<K>,
}

impl<K: FixedElement> FixedSizeSerie<K> {
    /// An empty column whose values are `width` bytes each.
    pub fn new(width: usize) -> Self {
        Self {
            data: Vec::new(),
            validity: None,
            len: 0,
            field: FixedSizeField::new("", width, false),
            _kind: PhantomData,
        }
    }

    field_accessors!();

    /// The fixed byte width `N` (from the held field).
    pub fn width(&self) -> usize {
        self.field.byte_width()
    }

    /// The erased [`AnyField`] this column contributes — its **held field** (name + metadata + width)
    /// with **effective** nullability `self.nullable() || self.has_nulls()` folded in — a lenient,
    /// Arrow-standard over-approximation.
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.has_nulls());
        AnyField::leaf(field.erase())
    }

    /// Like [`field`](FixedSizeSerie::field) but **consumes** the column.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.has_nulls();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field.erase())
    }

    /// Appends one value (`None` is a null). Errors ([`IoError::CorruptLength`]) if a present
    /// value is not exactly [`width`](FixedSizeSerie::width) bytes, or fails the kind's validation.
    pub fn push(&mut self, value: Option<&[u8]>) -> Result<(), IoError> {
        match value {
            Some(bytes) => {
                if bytes.len() != self.width() {
                    return Err(IoError::CorruptLength {
                        len: bytes.len() as u64,
                        width: self.width(),
                    });
                }
                K::validate(bytes)?;
                self.data.extend_from_slice(bytes);
                if let Some(validity) = &mut self.validity {
                    validity.push(true);
                }
            }
            None => {
                self.data.resize(self.data.len() + self.width(), 0); // zero placeholder slot
                self.validity
                    .get_or_insert_with(|| Bitmap::all_present(self.len))
                    .push(false);
            }
        }
        self.len += 1;
        Ok(())
    }

    /// A column from **present** values (no nulls), each exactly `width` bytes and validated for the
    /// kind — the present-only twin of [`from_options`](FixedSizeSerie::from_options) (mirrors
    /// [`extend_values`](FixedSizeSerie::extend_values)). `from_values` = present, `from_options` =
    /// nullable, uniform across every family.
    pub fn from_values(width: usize, values: &[&[u8]]) -> Result<Self, IoError> {
        let mut serie = Self::new(width);
        serie.extend_values(values)?;
        Ok(serie)
    }

    /// A column from **optional** values (a `None` is a null), each present value validated to be
    /// exactly `width` bytes — the nullable twin of [`from_values`](FixedSizeSerie::from_values).
    pub fn from_options(width: usize, values: &[Option<&[u8]>]) -> Result<Self, IoError> {
        let mut serie = Self::new(width);
        for &value in values {
            serie.push(value)?;
        }
        Ok(serie)
    }

    /// A column of `width`-byte values from a slice of [`FixedSizeScalar`]s — each scalar
    /// contributing its bytes (which must be exactly `width` long, and valid for the kind), a null
    /// scalar a null. The bulk analogue of the in-place [`set_scalars`](FixedSizeSerie::set_scalars).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{FixedBinaryScalar, FixedBinarySerie};
    ///
    /// let col = FixedBinarySerie::from_scalars(
    ///     2,
    ///     &[FixedBinaryScalar::from_bytes(b"ab").unwrap(), FixedBinaryScalar::null(2)],
    /// )
    /// .unwrap();
    /// assert_eq!(col.get_bytes(0), Some("ab".as_bytes()));
    /// assert_eq!(col.get_bytes(1), None);
    /// ```
    pub fn from_scalars(width: usize, scalars: &[FixedSizeScalar<K>]) -> Result<Self, IoError> {
        Self::from_options(
            width,
            &scalars
                .iter()
                .map(FixedSizeScalar::value_bytes)
                .collect::<Vec<_>>(),
        )
    }

    /// The raw bytes of element `index` — zero-copy — or `None` if null or out of range.
    pub fn get_bytes(&self, index: usize) -> Option<&[u8]> {
        if index >= self.len {
            return None;
        }
        if let Some(validity) = &self.validity {
            if !validity.get(index) {
                return None;
            }
        }
        Some(&self.data[index * self.width()..(index + 1) * self.width()])
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

    /// The typed descriptor.
    pub fn data_type(&self) -> FixedSizeType<K> {
        FixedSizeType::new(self.width())
    }

    /// A [`FixedSizeField`] naming this column, nullability inferred from whether it has nulls.
    pub fn to_field(&self, name: &str) -> FixedSizeField<K> {
        FixedSizeField::new(name, self.width(), self.has_nulls())
    }

    /// Element `index` as a [`FixedSizeScalar`] — null if the element is null or out of range.
    pub fn get_scalar(&self, index: usize) -> FixedSizeScalar<K> {
        match self.get_bytes(index) {
            // The bytes entered through a checked path, so they are already valid for the kind.
            Some(bytes) => FixedSizeScalar::from_bytes_unchecked(bytes),
            None => FixedSizeScalar::null(self.width()),
        }
    }

    // ---- in-place set: single element + bulk (from a Serie / scalars / byte values) ----------

    /// Overwrites element `index` in place — `Some` writes bytes (validated for the kind; they
    /// must be exactly [`width`](FixedSizeSerie::width) long), `None` a null. Errors
    /// [`IndexOutOfBounds`](IoError::IndexOutOfBounds) if `index` is not an existing element, or
    /// [`CorruptLength`](IoError::CorruptLength) on a wrong-width value.
    pub fn set(&mut self, index: usize, value: Option<&[u8]>) -> Result<(), IoError> {
        if index >= self.len {
            return Err(IoError::IndexOutOfBounds {
                index,
                len: self.len,
            });
        }
        let slot = index * self.width()..(index + 1) * self.width();
        match value {
            Some(bytes) => {
                if bytes.len() != self.width() {
                    return Err(IoError::CorruptLength {
                        len: bytes.len() as u64,
                        width: self.width(),
                    });
                }
                K::validate(bytes)?;
                self.data[slot].copy_from_slice(bytes);
                if let Some(validity) = &mut self.validity {
                    validity.set(index, true);
                }
            }
            None => {
                self.data[slot].fill(0); // zero the slot under the null
                self.validity
                    .get_or_insert_with(|| Bitmap::all_present(self.len))
                    .set(index, false);
            }
        }
        Ok(())
    }

    /// Overwrites element `index` from a [`FixedSizeScalar`].
    pub fn set_scalar(&mut self, index: usize, scalar: &FixedSizeScalar<K>) -> Result<(), IoError> {
        self.set(index, scalar.value_bytes())
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

    /// Bulk-overwrites `[start, start + source.len())` from another column (nulls included).
    pub fn set_range(&mut self, start: usize, source: &FixedSizeSerie<K>) -> Result<(), IoError> {
        self.check_range(start, source.len())?;
        for index in 0..source.len() {
            self.set(start + index, source.get_bytes(index))?;
        }
        Ok(())
    }

    /// Bulk-overwrites `[start, start + scalars.len())` from a slice of [`FixedSizeScalar`]s.
    pub fn set_scalars(
        &mut self,
        start: usize,
        scalars: &[FixedSizeScalar<K>],
    ) -> Result<(), IoError> {
        self.check_range(start, scalars.len())?;
        for (offset, scalar) in scalars.iter().enumerate() {
            self.set(start + offset, scalar.value_bytes())?;
        }
        Ok(())
    }

    /// Bulk-overwrites `[start, start + values.len())` from present byte values.
    pub fn set_values(&mut self, start: usize, values: &[&[u8]]) -> Result<(), IoError> {
        self.check_range(start, values.len())?;
        for (offset, &value) in values.iter().enumerate() {
            self.set(start + offset, Some(value))?;
        }
        Ok(())
    }

    // ---- grow: append single + bulk (the mutator vocabulary) ----------------------------

    /// Validates one optional value for a grow: a present value must be exactly
    /// [`width`](FixedSizeSerie::width) bytes and valid for the kind.
    fn validate_slot(&self, value: Option<&[u8]>) -> Result<(), IoError> {
        if let Some(bytes) = value {
            if bytes.len() != self.width() {
                return Err(IoError::CorruptLength {
                    len: bytes.len() as u64,
                    width: self.width(),
                });
            }
            K::validate(bytes)?;
        }
        Ok(())
    }

    /// Appends a slice of **optional** values (each exactly `width` bytes, validated for the kind) —
    /// the bulk grow twin of [`from_options`](FixedSizeSerie::from_options). The `N`-byte-slot data
    /// buffer is an owned `Vec`, so the grow is an amortized-`O(1)` append (one `reserve` + `extend`,
    /// not a per-element re-seal). Every value is validated **up front**, so a bad value leaves the
    /// column unchanged; a null appends a zero placeholder slot and lazily materializes the mask.
    pub fn extend_options(&mut self, values: &[Option<&[u8]>]) -> Result<(), IoError> {
        for &value in values {
            self.validate_slot(value)?;
        }
        let base = self.len;
        self.data.reserve(values.len() * self.width());
        for (offset, &value) in values.iter().enumerate() {
            match value {
                Some(bytes) => {
                    self.data.extend_from_slice(bytes);
                    if let Some(validity) = &mut self.validity {
                        validity.push(true);
                    }
                }
                None => {
                    self.data.resize(self.data.len() + self.width(), 0); // zero placeholder slot
                    self.validity
                        .get_or_insert_with(|| Bitmap::all_present(base + offset))
                        .push(false);
                }
            }
        }
        self.len += values.len();
        Ok(())
    }

    /// Appends a slice of **present** values (each exactly `width` bytes) — the bulk grow twin of
    /// [`set_values`](FixedSizeSerie::set_values).
    pub fn extend_values(&mut self, values: &[&[u8]]) -> Result<(), IoError> {
        self.extend_options(&values.iter().map(|&bytes| Some(bytes)).collect::<Vec<_>>())
    }

    /// Appends a slice of [`FixedSizeScalar`]s (each its bytes or a null) — the bulk grow twin of
    /// [`from_scalars`](FixedSizeSerie::from_scalars).
    pub fn extend_scalars(&mut self, scalars: &[FixedSizeScalar<K>]) -> Result<(), IoError> {
        self.extend_options(
            &scalars
                .iter()
                .map(FixedSizeScalar::value_bytes)
                .collect::<Vec<_>>(),
        )
    }

    /// Appends **another whole column** to this one — the two columns concatenate. Validates the
    /// source shares this column's fixed byte `width` (a guided
    /// [`CorruptLength`](IoError::CorruptLength) otherwise), then memcpy's the whole `N`-byte-slot
    /// data blob in one `extend` and carries the null positions over.
    pub fn concat(&mut self, source: &FixedSizeSerie<K>) -> Result<(), IoError> {
        if source.width() != self.width() {
            return Err(IoError::CorruptLength {
                len: source.width() as u64,
                width: self.width(),
            });
        }
        if source.len == 0 {
            return Ok(());
        }
        let base = self.len;
        self.data.extend_from_slice(&source.data); // memcpy the N-byte-slot data blob
        extend_validity(&mut self.validity, base, source.len, |offset| {
            source.validity.as_ref().is_none_or(|mask| mask.get(offset))
        });
        self.len += source.len;
        Ok(())
    }

    // ---- reshape: filter (keep selected rows) + fill_null (replace nulls) -----------------

    /// A **new** column keeping only the elements where `mask[i]` is `true` — the row filter over the
    /// flat `N`-byte-slot data buffer (the fixed `width` is preserved). Errors
    /// ([`Unsupported`](IoError::Unsupported)) if `mask.len() != self.len()`.
    ///
    /// ```
    /// use yggdryl_core::io::fixed::FixedBinarySerie;
    ///
    /// let col = FixedBinarySerie::from_options(2, &[Some(&b"ab"[..]), None, Some(&b"cd"[..])]).unwrap();
    /// let kept = col.filter(&[true, false, true]).unwrap();
    /// assert_eq!(kept.len(), 2);
    /// assert_eq!(kept.get_bytes(0), Some(&b"ab"[..]));
    /// assert_eq!(kept.get_bytes(1), Some(&b"cd"[..]));
    /// ```
    pub fn filter(&self, mask: &[bool]) -> Result<FixedSizeSerie<K>, IoError> {
        if mask.len() != self.len {
            return Err(filter_len_mismatch(mask.len(), self.len));
        }
        let width = self.width();
        let kept = mask.iter().filter(|&&keep| keep).count();
        let mut data = Vec::with_capacity(kept * width);
        let mut validity: Option<Bitmap> = None;
        let mut out_len = 0;
        for (index, &keep) in mask.iter().enumerate() {
            if !keep {
                continue;
            }
            data.extend_from_slice(&self.data[index * width..(index + 1) * width]);
            if self
                .validity
                .as_ref()
                .is_none_or(|bitmap| bitmap.get(index))
            {
                if let Some(bitmap) = &mut validity {
                    bitmap.push(true);
                }
            } else {
                validity
                    .get_or_insert_with(|| Bitmap::all_present(out_len))
                    .push(false);
            }
            out_len += 1;
        }
        Ok(Self {
            data,
            validity,
            len: kept,
            field: self.field.clone(),
            _kind: PhantomData,
        })
    }

    /// A **new** column with every null slot replaced by `fill` (validated for the kind; it must be
    /// exactly [`width`](FixedSizeSerie::width) bytes) — one pass, the erased
    /// [`fill_null`](crate::io::AnySerie::fill_null) path. If the column has no nulls it is cloned;
    /// otherwise the data is copied, each null slot overwritten with `fill`, and the validity mask
    /// **dropped** (fully present). Errors [`CorruptLength`](IoError::CorruptLength) on a wrong-width
    /// value.
    pub fn fill_null_bytes(&self, fill: &[u8]) -> Result<FixedSizeSerie<K>, IoError> {
        let width = self.width();
        if fill.len() != width {
            return Err(IoError::CorruptLength {
                len: fill.len() as u64,
                width,
            });
        }
        K::validate(fill)?;
        if !self.has_nulls() {
            return Ok(self.clone());
        }
        let mut data = self.data.clone();
        if let Some(validity) = &self.validity {
            for index in 0..self.len {
                if !validity.get(index) {
                    data[index * width..(index + 1) * width].copy_from_slice(fill);
                }
            }
        }
        Ok(Self {
            data,
            validity: None,
            len: self.len,
            field: self.field.clone(),
            _kind: PhantomData,
        })
    }

    /// Writes the column: `[len:u64][width:u64][flags:u8][validity?][data]`.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        let has_validity = self.has_nulls();
        let validity_bytes: &[u8] = if has_validity {
            self.validity.as_ref().unwrap().as_bytes()
        } else {
            &[]
        };
        let mut prefix = Vec::with_capacity(8 + 8 + 1 + validity_bytes.len());
        prefix.extend_from_slice(&(self.len as u64).to_le_bytes());
        prefix.extend_from_slice(&(self.width() as u64).to_le_bytes());
        prefix.push(u8::from(has_validity));
        prefix.extend_from_slice(validity_bytes);
        sink.write_all(&prefix)?;
        sink.write_all(&self.data)
    }

    /// Reads a column written by [`write_to`](FixedSizeSerie::write_to). Validates the decoded
    /// data for the kind and refuses a corrupt (overflowing) `len * width`.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let len = read_u64(source)? as usize;
        let width = read_u64(source)? as usize;
        let mut flags = [0u8; 1];
        source.read_exact(&mut flags)?;

        let validity = if flags[0] != 0 {
            let bits = source.read_exact_vec(len.div_ceil(8))?;
            Some(Bitmap::from_bytes(&bits, len))
        } else {
            None
        };

        let data_len = len.checked_mul(width).ok_or(IoError::CorruptLength {
            len: len as u64,
            width,
        })?;
        let data = source.read_exact_vec(data_len)?;
        // Validate each fixed-size slot for the kind — validating the whole `data` blob is not
        // sufficient (a multi-byte code point could straddle a slot boundary for `FixedUtf8`).
        if width > 0 {
            for slot in data.chunks_exact(width) {
                K::validate(slot)?;
            }
        }

        Ok(Self {
            data,
            validity,
            len,
            field: FixedSizeField::new("", width, false),
            _kind: PhantomData,
        })
    }

    /// This column's canonical bytes — the same `[len][width][flags][validity?][data]` frame
    /// [`write_to`](FixedSizeSerie::write_to) produces, returned as an owned `Vec`. The exact
    /// inverse of [`deserialize_bytes`](FixedSizeSerie::deserialize_bytes).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::FixedBinarySerie;
    ///
    /// let col = FixedBinarySerie::from_options(2, &[Some(&b"ab"[..]), None, Some(&b"cd"[..])]).unwrap();
    /// assert_eq!(FixedBinarySerie::deserialize_bytes(&col.serialize_bytes()).unwrap(), col);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a column from the bytes produced by
    /// [`serialize_bytes`](FixedSizeSerie::serialize_bytes), validating each slot for the kind and
    /// erroring on a truncated or corrupt frame.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }
}

/// Arrow array interop (feature `arrow`): a fixed-size byte column ↔
/// [`FixedSizeBinaryArray`](arrow_array::FixedSizeBinaryArray). Both `FixedBinary` and `FixedUtf8`
/// map to the same Arrow array (the utf8/binary tag is a *schema*-level distinction carried in field
/// metadata, per [`DataTypeId`](crate::io::DataTypeId)); the flat `N`-byte data buffer maps straight
/// across (copied, since the column owns a `Vec<u8>`).
#[cfg(feature = "arrow")]
impl<K: FixedElement> FixedSizeSerie<K> {
    /// This column as an Arrow [`FixedSizeBinaryArray`](arrow_array::FixedSizeBinaryArray) of value
    /// width `N`. Panics only for the degenerate `N == 0` (which Arrow cannot model).
    pub fn to_arrow_array(&self) -> arrow_array::FixedSizeBinaryArray {
        let values = arrow_buffer::Buffer::from(self.data.as_slice());
        let nulls = self.validity.as_ref().map(|bitmap| {
            let buffer = arrow_buffer::Buffer::from(bitmap.as_bytes());
            arrow_buffer::NullBuffer::new(arrow_buffer::BooleanBuffer::new(buffer, 0, self.len))
        });
        arrow_array::FixedSizeBinaryArray::new(self.width() as i32, values, nulls)
    }

    /// Builds a column from an Arrow [`FixedSizeBinaryArray`](arrow_array::FixedSizeBinaryArray),
    /// validating each slot for the kind. Reads the array's **logical** window (so a *sliced* array
    /// converts correctly); a null slot becomes the zero placeholder.
    pub fn from_arrow_array(array: &arrow_array::FixedSizeBinaryArray) -> Result<Self, IoError> {
        use arrow_array::Array;
        let width = array.value_length().max(0) as usize;
        let mut serie = Self::new(width);
        for index in 0..array.len() {
            if array.is_null(index) {
                serie.push(None)?;
            } else {
                serie.push(Some(array.value(index)))?;
            }
        }
        Ok(serie)
    }
}

impl<K: FixedElement> SerieType for FixedSizeSerie<K> {
    type Elem = Box<[u8]>;

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        self.null_count()
    }

    fn get(&self, index: usize) -> Option<Box<[u8]>> {
        self.get_bytes(index).map(Into::into)
    }
}

impl<K: FixedElement> VarSerie for FixedSizeSerie<K> {
    fn value_bytes(&self, index: usize) -> Option<&[u8]> {
        self.get_bytes(index)
    }
}

// -------------------------------------------------------------------------------------
// Value semantics + Debug for the generic types
// -------------------------------------------------------------------------------------

macro_rules! fixed_size_value_impls {
    ($ty:ident, $($field:ident),+) => {
        impl<K: FixedElement> Clone for $ty<K> {
            fn clone(&self) -> Self {
                Self { $($field: self.$field.clone(),)+ _kind: PhantomData }
            }
        }
        impl<K: FixedElement> PartialEq for $ty<K> {
            fn eq(&self, other: &Self) -> bool {
                $(self.$field == other.$field &&)+ true
            }
        }
        impl<K: FixedElement> Eq for $ty<K> {}
    };
}

fixed_size_value_impls!(FixedSizeField, name, width, nullable, metadata);

// A manual `Clone` / `PartialEq` / `Hash` for the scalar (not the macro): identity is over the
// **dtype param** (width, from the held field) + the bytes — never the field's name / nullable /
// metadata (schema intent). A null value's placeholder bytes are `None`, so nulls compare by width.
impl<K: FixedElement> Clone for FixedSizeScalar<K> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            field: self.field.clone(),
            _kind: PhantomData,
        }
    }
}
impl<K: FixedElement> PartialEq for FixedSizeScalar<K> {
    fn eq(&self, other: &Self) -> bool {
        self.width() == other.width() && self.value == other.value
    }
}
impl<K: FixedElement> Eq for FixedSizeScalar<K> {}

// A manual `Clone` for the column (the macro would work, but its `PartialEq` must be hand-written —
// see below — so `Clone` lives here beside it).
impl<K: FixedElement> Clone for FixedSizeSerie<K> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            validity: self.validity.clone(),
            len: self.len,
            field: self.field.clone(),
            _kind: PhantomData,
        }
    }
}

// Structural identity: same `(width, len)` and — at every index — the same present-or-null value.
// Because [`get_bytes`](FixedSizeSerie::get_bytes) returns `None` for a null slot (never the zero
// placeholder bytes), this covers null *positions* directly, so it is independent of whether the
// validity mask is materialized (an absent mask and a `Some(all-present)` one, left behind after a
// `set` clears the last null, denote the same value) and the placeholder bytes under a null never
// affect equality — keeping identity in lock-step with the byte codec, whose
// [`write_to`](FixedSizeSerie::write_to) zeroes null slots. Mirrors `DecimalSerie` / `TemporalSerie`
// / `Serie` (a raw `data`/`validity` compare, as a derive would do, wrongly makes a `set`-cleared
// all-present column differ from the equivalent dense one).
impl<K: FixedElement> PartialEq for FixedSizeSerie<K> {
    fn eq(&self, other: &Self) -> bool {
        // Identity is over the **dtype param** (width, from the held field) + the data — never the
        // field's name / nullable / metadata (schema intent).
        self.width() == other.width()
            && self.len == other.len
            && (0..self.len).all(|index| self.get_bytes(index) == other.get_bytes(index))
    }
}
impl<K: FixedElement> Eq for FixedSizeSerie<K> {}

impl<K: FixedElement> Copy for FixedSizeType<K> {}
impl<K: FixedElement> Clone for FixedSizeType<K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<K: FixedElement> PartialEq for FixedSizeType<K> {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
    }
}
impl<K: FixedElement> Eq for FixedSizeType<K> {}
impl<K: FixedElement> core::hash::Hash for FixedSizeType<K> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        K::NAME.hash(state);
        self.width.hash(state);
    }
}

impl<K: FixedElement> core::hash::Hash for FixedSizeScalar<K> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.width().hash(state);
        self.value.hash(state);
    }
}

impl<K: FixedElement> core::fmt::Debug for FixedSizeType<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "FixedSizeType<{}>({})", K::NAME, self.width)
    }
}
impl<K: FixedElement> core::fmt::Debug for FixedSizeField<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FixedSizeField")
            .field("name", &self.name)
            .field("type", &K::NAME)
            .field("width", &self.width)
            .field("nullable", &self.nullable)
            .finish()
    }
}
impl<K: FixedElement> core::fmt::Debug for FixedSizeScalar<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FixedSizeScalar")
            .field("type", &K::NAME)
            .field("width", &self.width())
            .field("null", &self.is_null())
            .finish()
    }
}
impl<K: FixedElement> core::fmt::Debug for FixedSizeSerie<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FixedSizeSerie")
            .field("type", &K::NAME)
            .field("width", &self.width())
            .field("len", &self.len)
            .field("null_count", &self.null_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::io::fixed::{FixedBinaryScalar, FixedBinarySerie};

    #[test]
    fn equality_ignores_a_materialized_all_present_mask() {
        // Clearing the last null with `set` leaves a materialized all-present validity mask; the
        // column must still equal (and round-trip byte-equal to) the same values with no mask — the
        // identity is over present-or-null values, not the raw validity mask / placeholder bytes.
        let mut cleared = FixedBinarySerie::from_options(2, &[Some(&b"ab"[..]), None]).unwrap();
        cleared.set(1, Some(&b"cd"[..])).unwrap();
        assert_eq!(cleared.null_count(), 0);

        let dense =
            FixedBinarySerie::from_options(2, &[Some(&b"ab"[..]), Some(&b"cd"[..])]).unwrap();
        assert_eq!(cleared, dense);
        assert_eq!(
            FixedBinarySerie::deserialize_bytes(&cleared.serialize_bytes()).unwrap(),
            cleared
        );

        // A genuine null still makes the columns differ (a null slot never reads placeholder bytes).
        let with_null = FixedBinarySerie::from_options(2, &[Some(&b"ab"[..]), None]).unwrap();
        assert_ne!(with_null, dense);
    }

    #[test]
    fn from_scalars_round_trips_a_column_through_its_own_scalars() {
        let col = FixedBinarySerie::from_options(
            2,
            &[Some(&b"ab"[..]), None, Some(&b"cd"[..]), Some(&b"ef"[..])],
        )
        .unwrap();
        let scalars: Vec<_> = (0..col.len()).map(|i| col.get_scalar(i)).collect();
        assert_eq!(FixedBinarySerie::from_scalars(2, &scalars).unwrap(), col);

        // A null scalar becomes a null element; the empty slice yields the empty column.
        let with_null = FixedBinarySerie::from_scalars(2, &[FixedBinaryScalar::null(2)]).unwrap();
        assert_eq!(with_null.get_bytes(0), None);
        assert_eq!(
            FixedBinarySerie::from_scalars(2, &[]).unwrap(),
            FixedBinarySerie::new(2)
        );
    }
}
