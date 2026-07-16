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
use crate::io::bitmap::Bitmap;
use crate::io::var::{VarScalar, VarSerie};
use crate::io::{
    DataType, DataTypeId, FieldType, Headers, IOCursor, IoError, ScalarType, SerieType,
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
    width: usize,
    value: Option<Box<[u8]>>,
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
            width: bytes.len(),
            value: Some(bytes.into()),
            _kind: PhantomData,
        }
    }

    /// The null scalar of the given byte width.
    pub fn null(width: usize) -> Self {
        Self {
            width,
            value: None,
            _kind: PhantomData,
        }
    }

    /// The fixed byte width `N`.
    pub fn width(&self) -> usize {
        self.width
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
        FixedSizeType::new(self.width)
    }

    /// Writes this scalar: `[width:u64][validity:u8][bytes?]`.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        sink.write_all(&(self.width as u64).to_le_bytes())?;
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
            width,
            value: Some(bytes.into_boxed_slice()),
            _kind: PhantomData,
        })
    }
}

impl<K: FixedElement> ScalarType for FixedSizeScalar<K> {
    type Data = FixedSizeType<K>;

    fn data_type(&self) -> FixedSizeType<K> {
        FixedSizeType::new(self.width)
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
    width: usize,
    data: Vec<u8>,
    validity: Option<Bitmap>,
    len: usize,
    _kind: PhantomData<K>,
}

impl<K: FixedElement> FixedSizeSerie<K> {
    /// An empty column whose values are `width` bytes each.
    pub fn new(width: usize) -> Self {
        Self {
            width,
            data: Vec::new(),
            validity: None,
            len: 0,
            _kind: PhantomData,
        }
    }

    /// The fixed byte width `N`.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Appends one value (`None` is a null). Errors ([`IoError::CorruptLength`]) if a present
    /// value is not exactly [`width`](FixedSizeSerie::width) bytes, or fails the kind's validation.
    pub fn push(&mut self, value: Option<&[u8]>) -> Result<(), IoError> {
        match value {
            Some(bytes) => {
                if bytes.len() != self.width {
                    return Err(IoError::CorruptLength {
                        len: bytes.len() as u64,
                        width: self.width,
                    });
                }
                K::validate(bytes)?;
                self.data.extend_from_slice(bytes);
                if let Some(validity) = &mut self.validity {
                    validity.push(true);
                }
            }
            None => {
                self.data.resize(self.data.len() + self.width, 0); // zero placeholder slot
                self.validity
                    .get_or_insert_with(|| Bitmap::all_present(self.len))
                    .push(false);
            }
        }
        self.len += 1;
        Ok(())
    }

    /// A column from optional values, each validated to be exactly `width` bytes.
    pub fn from_values(width: usize, values: &[Option<&[u8]>]) -> Result<Self, IoError> {
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
        Self::from_values(
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
        Some(&self.data[index * self.width..(index + 1) * self.width])
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
        FixedSizeType::new(self.width)
    }

    /// A [`FixedSizeField`] naming this column, nullability inferred from whether it has nulls.
    pub fn to_field(&self, name: &str) -> FixedSizeField<K> {
        FixedSizeField::new(name, self.width, self.has_nulls())
    }

    /// Element `index` as a [`FixedSizeScalar`] — null if the element is null or out of range.
    pub fn get_scalar(&self, index: usize) -> FixedSizeScalar<K> {
        match self.get_bytes(index) {
            // The bytes entered through a checked path, so they are already valid for the kind.
            Some(bytes) => FixedSizeScalar::from_bytes_unchecked(bytes),
            None => FixedSizeScalar::null(self.width),
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
        let slot = index * self.width..(index + 1) * self.width;
        match value {
            Some(bytes) => {
                if bytes.len() != self.width {
                    return Err(IoError::CorruptLength {
                        len: bytes.len() as u64,
                        width: self.width,
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
        prefix.extend_from_slice(&(self.width as u64).to_le_bytes());
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
            width,
            data,
            validity,
            len,
            _kind: PhantomData,
        })
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
        arrow_array::FixedSizeBinaryArray::new(self.width as i32, values, nulls)
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
fixed_size_value_impls!(FixedSizeScalar, width, value);
fixed_size_value_impls!(FixedSizeSerie, width, data, validity, len);

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
        self.width.hash(state);
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
            .field("width", &self.width)
            .field("null", &self.is_null())
            .finish()
    }
}
impl<K: FixedElement> core::fmt::Debug for FixedSizeSerie<K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FixedSizeSerie")
            .field("type", &K::NAME)
            .field("width", &self.width)
            .field("len", &self.len)
            .field("null_count", &self.null_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::io::fixed::{FixedBinaryScalar, FixedBinarySerie};

    #[test]
    fn from_scalars_round_trips_a_column_through_its_own_scalars() {
        let col = FixedBinarySerie::from_values(
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
