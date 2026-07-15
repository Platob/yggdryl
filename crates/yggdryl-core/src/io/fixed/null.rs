//! The **null type** — Arrow's `Null`: a 0-width type whose every value is null. It carries no
//! bytes at all (no value buffer, no validity mask — a column is just its length), so it models an
//! all-null column at zero storage cost and sits at the bottom of the type lattice (any other type
//! casts *to* and *from* it — see [`Converter`](crate::io::Converter)).
//!
//! Like the runtime-`N` [fixed-size byte family](super::fixed_size) it has no [`NativeType`]
//! (there is no value to store), so [`NullType`] / [`NullField`] / [`NullScalar`] / [`NullSerie`]
//! implement the family-agnostic root traits ([`DataType`] / [`FieldType`] / [`ScalarType`] /
//! [`SerieType`]) directly. Every value is null, so the value semantics are trivial: two null
//! scalars are always equal, and two null columns are equal iff they have the same length.

use super::Field;
use crate::io::{
    DataType, DataTypeId, FieldType, Headers, IOCursor, IoError, ScalarType, SerieType,
};

/// Reads a little-endian `u64` from a cursor.
fn read_u64<R: IOCursor>(source: &mut R) -> Result<u64, IoError> {
    let mut bytes = [0u8; 8];
    source.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

// -------------------------------------------------------------------------------------
// Descriptor
// -------------------------------------------------------------------------------------

/// The **null** data-type descriptor — Arrow's `Null`, of byte width `0`.
///
/// ```
/// use yggdryl_core::io::{DataType, DataTypeCategory, DataTypeId};
/// use yggdryl_core::io::fixed::NullType;
///
/// let dt = NullType::new();
/// assert_eq!(dt.name(), "null");
/// assert_eq!(dt.byte_width(), 0);
/// assert_eq!(dt.type_id(), DataTypeId::Null);
/// assert_eq!(dt.category(), DataTypeCategory::Null);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NullType;

impl NullType {
    /// The type name as a **compile-time constant**.
    pub const NAME: &'static str = "null";
    /// The byte width as a **compile-time constant** (always `0`).
    pub const BYTE_WIDTH: usize = 0;

    /// The (only) value of this zero-sized descriptor.
    pub const fn new() -> Self {
        Self
    }

    /// A [`NullField`] naming a column of this type.
    pub fn field(&self, name: &str) -> NullField {
        NullField::new(name)
    }
}

impl DataType for NullType {
    fn name(&self) -> &'static str {
        "null"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Null
    }
    // `to_arrow` is the centralized `DataType` default (`DataTypeId::Null` maps to `A::Null`).
}

// -------------------------------------------------------------------------------------
// Field
// -------------------------------------------------------------------------------------

/// A named **null** column descriptor. A null column is all-null, so it is always nullable; it
/// carries only its name and metadata [`Headers`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NullField {
    name: String,
    metadata: Headers,
}

impl NullField {
    /// A null field with the given name (empty metadata).
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            metadata: Headers::new(),
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
    pub fn data_type(&self) -> NullType {
        NullType
    }

    /// The erased runtime [`Field`], metadata preserved (always nullable).
    pub fn erase(&self) -> Field {
        Field::new(&self.name, &NullType, true).with_metadata(self.metadata.clone())
    }

    /// This field as an [`arrow_schema::Field`] (feature `arrow`), via the erased [`Field`].
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
        self.erase().to_arrow()
    }

    /// Builds a null field from an [`arrow_schema::Field`] (an Arrow `Null`), or `None` if it is
    /// not a null type (feature `arrow`). User metadata is preserved.
    #[cfg(feature = "arrow")]
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        let erased = Field::from_arrow(field)?;
        (FieldType::type_id(&erased) == DataTypeId::Null)
            .then(|| Self::new(erased.name()).with_metadata(erased.metadata().clone()))
    }
}

impl FieldType for NullField {
    fn name(&self) -> &str {
        &self.name
    }

    fn type_name(&self) -> &'static str {
        "null"
    }

    fn byte_width(&self) -> usize {
        0
    }

    fn nullable(&self) -> bool {
        true
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::Null
    }
}

// -------------------------------------------------------------------------------------
// Scalar
// -------------------------------------------------------------------------------------

/// One **null** value — the null type has no other inhabitant, so every [`NullScalar`] is equal
/// (and hashes the same) and its wire form is empty (`0` bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NullScalar;

impl NullScalar {
    /// The null value.
    pub const fn new() -> Self {
        Self
    }

    /// The null value (the cross-family name; every [`NullScalar`] is null).
    pub const fn null() -> Self {
        Self
    }

    /// Always `true` — the null type has only the null value.
    pub const fn is_null(&self) -> bool {
        true
    }

    /// Always `false`.
    pub const fn is_valid(&self) -> bool {
        false
    }

    /// The typed descriptor.
    pub const fn data_type(&self) -> NullType {
        NullType
    }

    /// A [`NullField`] naming a column of this scalar's type.
    pub fn field(&self, name: &str) -> NullField {
        NullField::new(name)
    }

    /// This scalar broadcast to a length-1 [`NullSerie`].
    pub const fn to_serie(&self) -> NullSerie {
        NullSerie::with_len(1)
    }

    /// The serialized byte width — always `0`.
    pub const fn serialized_width() -> usize {
        0
    }

    /// Writes this scalar — a no-op, since a null value has no bytes.
    pub fn write_to<W: IOCursor>(&self, _sink: &mut W) -> Result<(), IoError> {
        Ok(())
    }

    /// Reads a scalar (a no-op — the null value carries nothing).
    pub fn read_from<R: IOCursor>(_source: &mut R) -> Result<Self, IoError> {
        Ok(Self)
    }

    /// The value's canonical bytes — empty.
    pub fn serialize_bytes(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Reconstructs the null value (any input; there is only one value).
    pub fn deserialize_bytes(_bytes: &[u8]) -> Self {
        Self
    }
}

impl ScalarType for NullScalar {
    type Data = NullType;

    fn data_type(&self) -> NullType {
        NullType
    }

    fn is_null(&self) -> bool {
        true
    }
}

// -------------------------------------------------------------------------------------
// Serie (column)
// -------------------------------------------------------------------------------------

/// A **null column** — a run of `len` null values, stored as just the length (no value buffer, no
/// validity mask). Every element is null, so `null_count() == len()` and every `get` is `None`.
///
/// ```
/// use yggdryl_core::io::fixed::{NullScalar, NullSerie};
/// use yggdryl_core::io::{Bytes, IOCursor};
///
/// let mut col = NullSerie::with_len(2);
/// col.push(); // one more null
/// assert_eq!(col.len(), 3);
/// assert_eq!(col.null_count(), 3);
/// assert_eq!(col.get_scalar(0), NullScalar::null());
///
/// let mut sink = Bytes::new();
/// col.write_to(&mut sink).unwrap();
/// sink.rewind();
/// assert_eq!(NullSerie::read_from(&mut sink).unwrap(), col);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NullSerie {
    len: usize,
}

impl NullSerie {
    /// An empty null column.
    pub const fn new() -> Self {
        Self { len: 0 }
    }

    /// A null column of `len` nulls.
    pub const fn with_len(len: usize) -> Self {
        Self { len }
    }

    /// Appends one null, growing the column by one.
    pub fn push(&mut self) {
        self.len += 1;
    }

    /// Grows the column by `count` nulls.
    pub fn extend(&mut self, count: usize) {
        self.len += count;
    }

    /// The number of elements.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the column is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of null elements — always [`len`](NullSerie::len).
    pub const fn null_count(&self) -> usize {
        self.len
    }

    /// Whether the column carries any nulls — `true` unless empty.
    pub const fn has_nulls(&self) -> bool {
        self.len > 0
    }

    /// The typed descriptor.
    pub const fn data_type(&self) -> NullType {
        NullType
    }

    /// A [`NullField`] naming this column.
    pub fn to_field(&self, name: &str) -> NullField {
        NullField::new(name)
    }

    /// Element `index` as a [`NullScalar`] — always the null value.
    pub const fn get_scalar(&self, _index: usize) -> NullScalar {
        NullScalar
    }

    /// Writes the column — just its length as a little-endian `u64`.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        sink.write_all(&(self.len as u64).to_le_bytes())
    }

    /// Reads a column written by [`write_to`](NullSerie::write_to).
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        Ok(Self {
            len: read_u64(source)? as usize,
        })
    }

    /// This column as an Arrow [`NullArray`](arrow_array::NullArray) (feature `arrow`).
    #[cfg(feature = "arrow")]
    pub fn to_arrow_array(&self) -> arrow_array::NullArray {
        arrow_array::NullArray::new(self.len)
    }

    /// A column from an Arrow [`NullArray`](arrow_array::NullArray) (feature `arrow`).
    #[cfg(feature = "arrow")]
    pub fn from_arrow_array(array: &arrow_array::NullArray) -> Self {
        use arrow_array::Array;
        Self { len: array.len() }
    }
}

impl SerieType for NullSerie {
    // A null column has no value type — every `get` is `None`.
    type Elem = ();

    fn len(&self) -> usize {
        self.len
    }

    fn null_count(&self) -> usize {
        self.len
    }

    fn get(&self, _index: usize) -> Option<()> {
        None
    }
}
