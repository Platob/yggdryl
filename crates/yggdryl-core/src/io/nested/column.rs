//! [`Column`] — the erased, recursive column: a **thin enum over the crate's existing typed
//! Series**, so a [`StructSerie`](super::StructSerie) can hold heterogeneous children. It only
//! *wraps and delegates* — every operation (length, serialization, equality, and Arrow conversion)
//! calls the wrapped `Serie`'s own implementation, so there is no parallel column machinery and no
//! reimplemented buffers or codecs. Recursion is the [`Struct`](Column::Struct) variant, whose
//! payload is a whole [`StructSerie`].
//!
//! Arrow **recomposition is zero-copy** wherever the wrapped `Serie` is: the fixed-primitive and
//! decimal columns hand back their shared `Arc` buffer via their own `to_arrow_array`. The six wide
//! non-Arrow-native integers (`u96`/`i96`/`u128`/`i128`/`u256`/`i256`) have no zero-copy `Serie`
//! Arrow path, so they map through the closest-representation Arrow type; everything else delegates.

use super::{ColumnField, StructSerie, Value};
#[cfg(feature = "arrow")]
use crate::io::bitmap::Bitmap;
use crate::io::fixed::{
    f16, Dec128, Dec256, Dec32, Dec64, DecimalBacking, DecimalField, DecimalSerie,
    Field as LeafField, FixedBinarySerie, FixedElement, FixedSizeSerie, FixedUtf8Serie, NativeType,
    NullSerie, Serie, I256, I96, U256, U96,
};
use crate::io::var::{BinarySerie, ByteSerie, Utf8Serie, VarElement};
use crate::io::{Bytes, DataTypeId, FieldType, IOCursor, IoError};

/// The width of one variable-length offset (`i32`) — the fixed portion of a utf8/binary column.
const OFFSET_WIDTH: usize = core::mem::size_of::<i32>();

/// A **column of any type**, type-erased over the crate's concrete Series — the recursive carrier a
/// struct column's heterogeneous children live in. Build one from any typed column with [`From`]
/// (e.g. `Column::from(serie)`), or from an Arrow array with
/// [`from_arrow_array`](Column::from_arrow_array).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Column {
    /// An all-null column (Arrow `Null`).
    Null(NullSerie),
    /// `u8` … the fixed-width primitive columns, each a [`Serie`].
    U8(Serie<u8>),
    /// `u16`.
    U16(Serie<u16>),
    /// `u32`.
    U32(Serie<u32>),
    /// `u64`.
    U64(Serie<u64>),
    /// `u96`.
    U96(Serie<U96>),
    /// `u128`.
    U128(Serie<u128>),
    /// `u256`.
    U256(Serie<U256>),
    /// `i8`.
    I8(Serie<i8>),
    /// `i16`.
    I16(Serie<i16>),
    /// `i32`.
    I32(Serie<i32>),
    /// `i64`.
    I64(Serie<i64>),
    /// `i96`.
    I96(Serie<I96>),
    /// `i128`.
    I128(Serie<i128>),
    /// `i256`.
    I256(Serie<I256>),
    /// `f16`.
    F16(Serie<f16>),
    /// `f32`.
    F32(Serie<f32>),
    /// `f64`.
    F64(Serie<f64>),
    /// `d32` scaled decimal.
    D32(DecimalSerie<Dec32>),
    /// `d64`.
    D64(DecimalSerie<Dec64>),
    /// `d128`.
    D128(DecimalSerie<Dec128>),
    /// `d256`.
    D256(DecimalSerie<Dec256>),
    /// Variable-length UTF-8 string column.
    Utf8(Utf8Serie),
    /// Variable-length opaque binary column.
    Binary(BinarySerie),
    /// Fixed-size opaque binary column.
    FixedBinary(FixedBinarySerie),
    /// Fixed-size UTF-8 string column.
    FixedUtf8(FixedUtf8Serie),
    /// A struct column — heterogeneous child columns (the recursion).
    Struct(StructSerie),
}

/// Runs `$call` against the wrapped `Serie` of every variant (bound to `$s`) — the one-liner every
/// uniform delegating method reduces to, so a new variant is a single line here.
macro_rules! for_each {
    ($self:expr, $s:ident => $call:expr) => {
        match $self {
            Column::Null($s) => $call,
            Column::U8($s) => $call,
            Column::U16($s) => $call,
            Column::U32($s) => $call,
            Column::U64($s) => $call,
            Column::U96($s) => $call,
            Column::U128($s) => $call,
            Column::U256($s) => $call,
            Column::I8($s) => $call,
            Column::I16($s) => $call,
            Column::I32($s) => $call,
            Column::I64($s) => $call,
            Column::I96($s) => $call,
            Column::I128($s) => $call,
            Column::I256($s) => $call,
            Column::F16($s) => $call,
            Column::F32($s) => $call,
            Column::F64($s) => $call,
            Column::D32($s) => $call,
            Column::D64($s) => $call,
            Column::D128($s) => $call,
            Column::D256($s) => $call,
            Column::Utf8($s) => $call,
            Column::Binary($s) => $call,
            Column::FixedBinary($s) => $call,
            Column::FixedUtf8($s) => $call,
            Column::Struct($s) => $call,
        }
    };
}

impl Column {
    /// An all-null column of `len` elements.
    pub fn null(len: usize) -> Self {
        Self::Null(NullSerie::with_len(len))
    }

    /// The number of elements — delegates to the wrapped `Serie`.
    pub fn len(&self) -> usize {
        for_each!(self, s => s.len())
    }

    /// Whether the column is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of null elements — delegates to the wrapped `Serie`.
    pub fn null_count(&self) -> usize {
        for_each!(self, s => s.null_count())
    }

    /// Whether the column carries any nulls.
    pub fn has_nulls(&self) -> bool {
        self.null_count() > 0
    }

    /// The column's element [`DataTypeId`].
    pub fn type_id(&self) -> DataTypeId {
        match self {
            Self::Null(_) => DataTypeId::Null,
            Self::U8(_) => DataTypeId::U8,
            Self::U16(_) => DataTypeId::U16,
            Self::U32(_) => DataTypeId::U32,
            Self::U64(_) => DataTypeId::U64,
            Self::U96(_) => DataTypeId::U96,
            Self::U128(_) => DataTypeId::U128,
            Self::U256(_) => DataTypeId::U256,
            Self::I8(_) => DataTypeId::I8,
            Self::I16(_) => DataTypeId::I16,
            Self::I32(_) => DataTypeId::I32,
            Self::I64(_) => DataTypeId::I64,
            Self::I96(_) => DataTypeId::I96,
            Self::I128(_) => DataTypeId::I128,
            Self::I256(_) => DataTypeId::I256,
            Self::F16(_) => DataTypeId::F16,
            Self::F32(_) => DataTypeId::F32,
            Self::F64(_) => DataTypeId::F64,
            Self::D32(_) => DataTypeId::D32,
            Self::D64(_) => DataTypeId::D64,
            Self::D128(_) => DataTypeId::D128,
            Self::D256(_) => DataTypeId::D256,
            Self::Utf8(_) => DataTypeId::Utf8,
            Self::Binary(_) => DataTypeId::Binary,
            Self::FixedBinary(_) => DataTypeId::FixedBinary,
            Self::FixedUtf8(_) => DataTypeId::FixedUtf8,
            Self::Struct(_) => DataTypeId::Struct,
        }
    }

    /// The [`ColumnField`] naming a column of this type `name`, its nullability **inferred** from
    /// whether the column currently holds any nulls.
    pub fn field(&self, name: &str) -> ColumnField {
        let nullable = self.has_nulls();
        match self {
            Self::Null(_) => leaf(name, DataTypeId::Null, 0, true),
            Self::U8(_) => prim_field::<u8>(name, nullable),
            Self::U16(_) => prim_field::<u16>(name, nullable),
            Self::U32(_) => prim_field::<u32>(name, nullable),
            Self::U64(_) => prim_field::<u64>(name, nullable),
            Self::U96(_) => prim_field::<U96>(name, nullable),
            Self::U128(_) => prim_field::<u128>(name, nullable),
            Self::U256(_) => prim_field::<U256>(name, nullable),
            Self::I8(_) => prim_field::<i8>(name, nullable),
            Self::I16(_) => prim_field::<i16>(name, nullable),
            Self::I32(_) => prim_field::<i32>(name, nullable),
            Self::I64(_) => prim_field::<i64>(name, nullable),
            Self::I96(_) => prim_field::<I96>(name, nullable),
            Self::I128(_) => prim_field::<i128>(name, nullable),
            Self::I256(_) => prim_field::<I256>(name, nullable),
            Self::F16(_) => prim_field::<f16>(name, nullable),
            Self::F32(_) => prim_field::<f32>(name, nullable),
            Self::F64(_) => prim_field::<f64>(name, nullable),
            Self::D32(s) => dec_field(name, s.precision(), s.scale(), nullable, DataTypeId::D32),
            Self::D64(s) => dec_field(name, s.precision(), s.scale(), nullable, DataTypeId::D64),
            Self::D128(s) => dec_field(name, s.precision(), s.scale(), nullable, DataTypeId::D128),
            Self::D256(s) => dec_field(name, s.precision(), s.scale(), nullable, DataTypeId::D256),
            Self::Utf8(_) => leaf(name, DataTypeId::Utf8, OFFSET_WIDTH, nullable),
            Self::Binary(_) => leaf(name, DataTypeId::Binary, OFFSET_WIDTH, nullable),
            Self::FixedBinary(s) => leaf(name, DataTypeId::FixedBinary, s.width(), nullable),
            Self::FixedUtf8(s) => leaf(name, DataTypeId::FixedUtf8, s.width(), nullable),
            Self::Struct(s) => ColumnField::struct_(s.to_field(name)),
        }
    }

    /// The value at `index` as an erased [`Value`] — null if the element is null or out of range.
    pub fn get(&self, index: usize) -> Value {
        match self {
            Self::Null(_) => Value::Null,
            Self::U8(s) => prim_value(s, index),
            Self::U16(s) => prim_value(s, index),
            Self::U32(s) => prim_value(s, index),
            Self::U64(s) => prim_value(s, index),
            Self::U96(s) => prim_value(s, index),
            Self::U128(s) => prim_value(s, index),
            Self::U256(s) => prim_value(s, index),
            Self::I8(s) => prim_value(s, index),
            Self::I16(s) => prim_value(s, index),
            Self::I32(s) => prim_value(s, index),
            Self::I64(s) => prim_value(s, index),
            Self::I96(s) => prim_value(s, index),
            Self::I128(s) => prim_value(s, index),
            Self::I256(s) => prim_value(s, index),
            Self::F16(s) => prim_value(s, index),
            Self::F32(s) => prim_value(s, index),
            Self::F64(s) => prim_value(s, index),
            Self::D32(s) => dec_value(s, index),
            Self::D64(s) => dec_value(s, index),
            Self::D128(s) => dec_value(s, index),
            Self::D256(s) => dec_value(s, index),
            Self::Utf8(s) => var_value(s, index),
            Self::Binary(s) => var_value(s, index),
            Self::FixedBinary(s) => fixed_size_value(s, index),
            Self::FixedUtf8(s) => fixed_size_value(s, index),
            Self::Struct(s) => s.get_row(index),
        }
    }

    // ---- serialization: delegate to each `Serie`'s own byte codec ----------------------

    /// Writes this column to `sink` — delegates to the wrapped `Serie`'s `write_to`, so the frame is
    /// exactly the typed column's own self-describing frame.
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        for_each!(self, s => s.write_to(sink))
    }

    /// Reads a column written by [`write_to`](Column::write_to), guided by its `field` (which pins
    /// which `Serie` codec to invoke). Delegates entirely to that `Serie`'s `read_from`.
    pub fn read_from<R: IOCursor>(field: &ColumnField, source: &mut R) -> Result<Self, IoError> {
        let leaf = match field {
            ColumnField::Struct(_) => return Ok(Self::Struct(StructSerie::read_from(source)?)),
            ColumnField::Leaf(leaf) => leaf,
        };
        Ok(match FieldType::type_id(leaf) {
            DataTypeId::Null => Self::Null(NullSerie::read_from(source)?),
            DataTypeId::U8 => Self::U8(Serie::read_from(source)?),
            DataTypeId::U16 => Self::U16(Serie::read_from(source)?),
            DataTypeId::U32 => Self::U32(Serie::read_from(source)?),
            DataTypeId::U64 => Self::U64(Serie::read_from(source)?),
            DataTypeId::U96 => Self::U96(Serie::read_from(source)?),
            DataTypeId::U128 => Self::U128(Serie::read_from(source)?),
            DataTypeId::U256 => Self::U256(Serie::read_from(source)?),
            DataTypeId::I8 => Self::I8(Serie::read_from(source)?),
            DataTypeId::I16 => Self::I16(Serie::read_from(source)?),
            DataTypeId::I32 => Self::I32(Serie::read_from(source)?),
            DataTypeId::I64 => Self::I64(Serie::read_from(source)?),
            DataTypeId::I96 => Self::I96(Serie::read_from(source)?),
            DataTypeId::I128 => Self::I128(Serie::read_from(source)?),
            DataTypeId::I256 => Self::I256(Serie::read_from(source)?),
            DataTypeId::F16 => Self::F16(Serie::read_from(source)?),
            DataTypeId::F32 => Self::F32(Serie::read_from(source)?),
            DataTypeId::F64 => Self::F64(Serie::read_from(source)?),
            DataTypeId::D32 => Self::D32(DecimalSerie::read_from(source)?),
            DataTypeId::D64 => Self::D64(DecimalSerie::read_from(source)?),
            DataTypeId::D128 => Self::D128(DecimalSerie::read_from(source)?),
            DataTypeId::D256 => Self::D256(DecimalSerie::read_from(source)?),
            DataTypeId::Utf8 => Self::Utf8(Utf8Serie::read_from(source)?),
            DataTypeId::Binary => Self::Binary(BinarySerie::read_from(source)?),
            DataTypeId::FixedBinary => Self::FixedBinary(FixedBinarySerie::read_from(source)?),
            DataTypeId::FixedUtf8 => Self::FixedUtf8(FixedUtf8Serie::read_from(source)?),
            other => {
                return Err(IoError::Unsupported {
                    what: format!(
                        "cannot deserialize a nested column of type {}",
                        other.name()
                    ),
                })
            }
        })
    }

    /// This column's canonical bytes (the wrapped `Serie`'s frame), as an owned `Vec`.
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::new();
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a column from [`serialize_bytes`](Column::serialize_bytes), guided by `field`.
    pub fn deserialize_bytes(field: &ColumnField, bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(field, &mut Bytes::from_slice(bytes))
    }

    /// An empty (zero-row) column matching `field`'s type — an empty wrapped `Serie` of the right
    /// kind (decimal precision/scale and fixed-size width recovered from the field). The building
    /// block for an empty [`StructSerie`](super::StructSerie).
    pub(crate) fn empty_of(field: &ColumnField) -> Self {
        let leaf = match field {
            ColumnField::Struct(schema) => return Self::Struct(StructSerie::empty(schema)),
            ColumnField::Leaf(leaf) => leaf,
        };
        match FieldType::type_id(leaf) {
            DataTypeId::Null => Self::Null(NullSerie::new()),
            DataTypeId::U8 => Self::U8(Serie::new()),
            DataTypeId::U16 => Self::U16(Serie::new()),
            DataTypeId::U32 => Self::U32(Serie::new()),
            DataTypeId::U64 => Self::U64(Serie::new()),
            DataTypeId::U96 => Self::U96(Serie::new()),
            DataTypeId::U128 => Self::U128(Serie::new()),
            DataTypeId::U256 => Self::U256(Serie::new()),
            DataTypeId::I8 => Self::I8(Serie::new()),
            DataTypeId::I16 => Self::I16(Serie::new()),
            DataTypeId::I32 => Self::I32(Serie::new()),
            DataTypeId::I64 => Self::I64(Serie::new()),
            DataTypeId::I96 => Self::I96(Serie::new()),
            DataTypeId::I128 => Self::I128(Serie::new()),
            DataTypeId::I256 => Self::I256(Serie::new()),
            DataTypeId::F16 => Self::F16(Serie::new()),
            DataTypeId::F32 => Self::F32(Serie::new()),
            DataTypeId::F64 => Self::F64(Serie::new()),
            DataTypeId::D32 => {
                Self::D32(DecimalSerie::new(dec_precision(leaf, 9), dec_scale(leaf)))
            }
            DataTypeId::D64 => {
                Self::D64(DecimalSerie::new(dec_precision(leaf, 18), dec_scale(leaf)))
            }
            DataTypeId::D128 => {
                Self::D128(DecimalSerie::new(dec_precision(leaf, 38), dec_scale(leaf)))
            }
            DataTypeId::D256 => {
                Self::D256(DecimalSerie::new(dec_precision(leaf, 76), dec_scale(leaf)))
            }
            DataTypeId::Utf8 => Self::Utf8(Utf8Serie::new()),
            DataTypeId::Binary => Self::Binary(BinarySerie::new()),
            DataTypeId::FixedBinary => Self::FixedBinary(FixedBinarySerie::new(leaf.byte_width())),
            DataTypeId::FixedUtf8 => Self::FixedUtf8(FixedUtf8Serie::new(leaf.byte_width())),
            _ => Self::Null(NullSerie::new()),
        }
    }
}

/// A decimal field's precision from its reserved metadata, or `default` (the id's max precision).
fn dec_precision(leaf: &LeafField, default: u8) -> u8 {
    leaf.metadata()
        .get(DataTypeId::PRECISION_METADATA_KEY)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// A decimal field's scale from its reserved metadata, or `0`.
fn dec_scale(leaf: &LeafField) -> i8 {
    leaf.metadata()
        .get(DataTypeId::SCALE_METADATA_KEY)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

// ---- small per-family helpers (erase a value, name a field) -----------------------------

/// A leaf [`ColumnField`] from its parts.
fn leaf(name: &str, id: DataTypeId, width: usize, nullable: bool) -> ColumnField {
    ColumnField::leaf(LeafField::of(name, id, width, nullable))
}

/// A leaf field for a fixed-width primitive `T`.
fn prim_field<T: NativeType>(name: &str, nullable: bool) -> ColumnField {
    leaf(name, T::TYPE_ID, T::WIDTH, nullable)
}

/// A leaf field for a decimal, carrying its precision/scale (via [`DecimalField::erase`]).
fn dec_field(name: &str, precision: u8, scale: i8, nullable: bool, id: DataTypeId) -> ColumnField {
    // The precision/scale come from the serie; erasing a `DecimalField` keeps them in the reserved
    // metadata keys that the leaf field's Arrow mapping needs.
    let field = match id {
        DataTypeId::D32 => DecimalField::<Dec32>::new(name, precision, scale, nullable).erase(),
        DataTypeId::D64 => DecimalField::<Dec64>::new(name, precision, scale, nullable).erase(),
        DataTypeId::D128 => DecimalField::<Dec128>::new(name, precision, scale, nullable).erase(),
        _ => DecimalField::<Dec256>::new(name, precision, scale, nullable).erase(),
    };
    ColumnField::leaf(field)
}

/// Erases a fixed-width primitive value at `index` to a [`Value`].
fn prim_value<T: NativeType>(serie: &Serie<T>, index: usize) -> Value {
    match serie.get(index) {
        Some(value) => {
            let mut scratch = [0u8; 32];
            value.write_le(&mut scratch);
            Value::leaf(
                LeafField::of("", T::TYPE_ID, T::WIDTH, false),
                scratch[..T::WIDTH].to_vec(),
            )
        }
        None => Value::Null,
    }
}

/// Erases a decimal coefficient at `index` to a [`Value`] (its raw bytes + the decimal field).
fn dec_value<B: DecimalBacking>(serie: &DecimalSerie<B>, index: usize) -> Value {
    if index >= serie.len() || serie.get_coeff(index).is_none() {
        return Value::Null;
    }
    let field = DecimalField::<B>::new("", serie.precision(), serie.scale(), false).erase();
    let bytes = serie.coeff_bytes()[index * B::WIDTH..(index + 1) * B::WIDTH].to_vec();
    Value::leaf(field, bytes)
}

/// Erases a variable-length value at `index` to a [`Value`].
fn var_value<E: VarElement>(serie: &ByteSerie<E>, index: usize) -> Value {
    match serie.get_bytes(index) {
        Some(bytes) => Value::leaf(
            LeafField::of("", E::TYPE_ID, OFFSET_WIDTH, false),
            bytes.to_vec(),
        ),
        None => Value::Null,
    }
}

/// Erases a fixed-size byte value at `index` to a [`Value`].
fn fixed_size_value<K: FixedElement>(serie: &FixedSizeSerie<K>, index: usize) -> Value {
    match serie.get_bytes(index) {
        Some(bytes) => Value::leaf(
            LeafField::of("", K::TYPE_ID, serie.width(), false),
            bytes.to_vec(),
        ),
        None => Value::Null,
    }
}

// -------------------------------------------------------------------------------------
// Erasing a typed column into a `Column` — one thin `From` per Serie (macro-generated).
// -------------------------------------------------------------------------------------

macro_rules! from_serie {
    ($($variant:ident => $ty:ty),* $(,)?) => {
        $(
            impl From<$ty> for Column {
                fn from(serie: $ty) -> Self {
                    Column::$variant(serie)
                }
            }
        )*
    };
}

from_serie! {
    Null => NullSerie,
    U8 => Serie<u8>, U16 => Serie<u16>, U32 => Serie<u32>, U64 => Serie<u64>,
    U96 => Serie<U96>, U128 => Serie<u128>, U256 => Serie<U256>,
    I8 => Serie<i8>, I16 => Serie<i16>, I32 => Serie<i32>, I64 => Serie<i64>,
    I96 => Serie<I96>, I128 => Serie<i128>, I256 => Serie<I256>,
    F16 => Serie<f16>, F32 => Serie<f32>, F64 => Serie<f64>,
    D32 => DecimalSerie<Dec32>, D64 => DecimalSerie<Dec64>,
    D128 => DecimalSerie<Dec128>, D256 => DecimalSerie<Dec256>,
    Utf8 => Utf8Serie, Binary => BinarySerie,
    FixedBinary => FixedBinarySerie, FixedUtf8 => FixedUtf8Serie,
    Struct => StructSerie,
}

// -------------------------------------------------------------------------------------
// Arrow array interop (feature `arrow`): delegate to each `Serie`'s own zero-copy converter.
// -------------------------------------------------------------------------------------

/// Builds a fresh validity [`Bitmap`] over an Arrow array's logical window (offset-aware, so a
/// *sliced* array converts correctly), canonicalizing an all-present array to `None`. Shared with
/// [`StructSerie`](super::StructSerie)'s top-level validity import.
#[cfg(feature = "arrow")]
pub(crate) fn validity_from_arrow(array: &dyn arrow_array::Array) -> Option<Bitmap> {
    if array.null_count() == 0 {
        return None;
    }
    let mut bitmap = Bitmap::all_present(array.len());
    for index in 0..array.len() {
        if array.is_null(index) {
            bitmap.set(index, false);
        }
    }
    Some(bitmap)
}

/// A wide (non-Arrow-native) integer column's Arrow array — built from the wrapped `Serie`'s value
/// bytes through the closest-representation Arrow type (`FixedSizeBinary` / `Decimal`). The only
/// non-delegating Arrow path (Arrow has no native array for these widths).
#[cfg(feature = "arrow")]
fn wide_to_arrow<T: NativeType>(serie: &Serie<T>) -> arrow_array::ArrayRef {
    let data_type = T::TYPE_ID.to_arrow(T::WIDTH);
    let values = arrow_buffer::Buffer::from(serie.value_bytes());
    let nulls = serie
        .validity_bitmap()
        .map(|bitmap| arrow_buffer::Buffer::from(bitmap.as_bytes()));
    let data =
        arrow_data::ArrayData::try_new(data_type, serie.len(), nulls, 0, vec![values], vec![])
            .expect("a wide integer column's bytes are valid for its Arrow type");
    arrow_array::make_array(data)
}

/// Rebuilds a wide (non-Arrow-native) `Serie<T>` from an imported Arrow array's flat value bytes,
/// reading its **logical** window (offset-aware) and zeroing bytes under null slots.
#[cfg(feature = "arrow")]
fn wide_from_arrow<T: NativeType>(array: &dyn arrow_array::Array) -> Serie<T> {
    let width = T::WIDTH;
    let len = array.len();
    let data = array.to_data();
    let src = data.buffers()[0].as_slice();
    let base = data.offset() * width;
    let mut values = vec![0u8; len * width];
    for index in 0..len {
        if !array.is_null(index) {
            let start = base + index * width;
            values[index * width..(index + 1) * width].copy_from_slice(&src[start..start + width]);
        }
    }
    Serie::from_byte_slice(values, validity_from_arrow(array), len)
}

/// Downcasts `array` to a concrete Arrow array type, or a guided [`Unsupported`](IoError::Unsupported).
#[cfg(feature = "arrow")]
fn downcast<'a, A: 'static>(
    array: &'a dyn arrow_array::Array,
    field: &arrow_schema::Field,
) -> Result<&'a A, IoError> {
    array
        .as_any()
        .downcast_ref::<A>()
        .ok_or_else(|| unsupported(field))
}

/// The guided "Arrow type not modeled" error for a field the crate cannot import.
#[cfg(feature = "arrow")]
fn unsupported(field: &arrow_schema::Field) -> IoError {
    IoError::Unsupported {
        what: format!(
            "Arrow field {:?} of type {:?} is not a yggdryl-modeled column type",
            field.name(),
            field.data_type()
        ),
    }
}

#[cfg(feature = "arrow")]
impl Column {
    /// This column as an Arrow [`ArrayRef`](arrow_array::ArrayRef) — **delegates** to the wrapped
    /// `Serie`'s own `to_arrow_array` (zero-copy for the fixed-primitive and decimal columns, which
    /// share their `Arc` buffer). The six wide non-Arrow-native integers map through the
    /// closest-representation Arrow type; a struct recurses into a `StructArray`.
    pub fn to_arrow_array(&self) -> arrow_array::ArrayRef {
        use std::sync::Arc;
        match self {
            Self::Null(s) => Arc::new(s.to_arrow_array()),
            Self::U8(s) => Arc::new(s.to_arrow_array()),
            Self::U16(s) => Arc::new(s.to_arrow_array()),
            Self::U32(s) => Arc::new(s.to_arrow_array()),
            Self::U64(s) => Arc::new(s.to_arrow_array()),
            Self::U96(s) => wide_to_arrow(s),
            Self::U128(s) => wide_to_arrow(s),
            Self::U256(s) => wide_to_arrow(s),
            Self::I8(s) => Arc::new(s.to_arrow_array()),
            Self::I16(s) => Arc::new(s.to_arrow_array()),
            Self::I32(s) => Arc::new(s.to_arrow_array()),
            Self::I64(s) => Arc::new(s.to_arrow_array()),
            Self::I96(s) => wide_to_arrow(s),
            Self::I128(s) => wide_to_arrow(s),
            Self::I256(s) => wide_to_arrow(s),
            Self::F16(s) => Arc::new(s.to_arrow_array()),
            Self::F32(s) => Arc::new(s.to_arrow_array()),
            Self::F64(s) => Arc::new(s.to_arrow_array()),
            Self::D32(s) => Arc::new(s.to_arrow_array()),
            Self::D64(s) => Arc::new(s.to_arrow_array()),
            Self::D128(s) => Arc::new(s.to_arrow_array()),
            Self::D256(s) => Arc::new(s.to_arrow_array()),
            Self::Utf8(s) => Arc::new(s.to_arrow_array()),
            Self::Binary(s) => Arc::new(s.to_arrow_array()),
            Self::FixedBinary(s) => Arc::new(s.to_arrow_array()),
            Self::FixedUtf8(s) => Arc::new(s.to_arrow_array()),
            Self::Struct(s) => Arc::new(s.to_arrow_array()),
        }
    }

    /// Builds a column from an Arrow [`Array`](arrow_array::Array) and its
    /// [`Field`](arrow_schema::Field) — **delegates** to the matching `Serie`'s own
    /// `from_arrow_array` (recovering the exact logical leaf type from the field metadata).
    pub fn from_arrow_array(
        array: &dyn arrow_array::Array,
        field: &arrow_schema::Field,
    ) -> Result<Self, IoError> {
        use arrow_array::types::*;
        if matches!(field.data_type(), arrow_schema::DataType::Struct(_)) {
            let struct_array = downcast::<arrow_array::StructArray>(array, field)?;
            return Ok(Self::Struct(StructSerie::from_arrow_array(
                struct_array,
                field,
            )?));
        }
        let leaf = LeafField::from_arrow(field).ok_or_else(|| unsupported(field))?;
        Ok(match FieldType::type_id(&leaf) {
            DataTypeId::Null => Self::Null(NullSerie::with_len(array.len())),
            DataTypeId::U8 => Self::U8(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<UInt8Type>,
            >(array, field)?)),
            DataTypeId::U16 => Self::U16(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<UInt16Type>,
            >(array, field)?)),
            DataTypeId::U32 => Self::U32(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<UInt32Type>,
            >(array, field)?)),
            DataTypeId::U64 => Self::U64(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<UInt64Type>,
            >(array, field)?)),
            DataTypeId::I8 => Self::I8(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Int8Type>,
            >(array, field)?)),
            DataTypeId::I16 => Self::I16(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Int16Type>,
            >(array, field)?)),
            DataTypeId::I32 => Self::I32(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Int32Type>,
            >(array, field)?)),
            DataTypeId::I64 => Self::I64(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Int64Type>,
            >(array, field)?)),
            DataTypeId::F16 => Self::F16(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Float16Type>,
            >(array, field)?)),
            DataTypeId::F32 => Self::F32(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Float32Type>,
            >(array, field)?)),
            DataTypeId::F64 => Self::F64(Serie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Float64Type>,
            >(array, field)?)),
            DataTypeId::U96 => Self::U96(wide_from_arrow(array)),
            DataTypeId::U128 => Self::U128(wide_from_arrow(array)),
            DataTypeId::U256 => Self::U256(wide_from_arrow(array)),
            DataTypeId::I96 => Self::I96(wide_from_arrow(array)),
            DataTypeId::I128 => Self::I128(wide_from_arrow(array)),
            DataTypeId::I256 => Self::I256(wide_from_arrow(array)),
            DataTypeId::D32 => Self::D32(DecimalSerie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Decimal32Type>,
            >(
                array, field
            )?)),
            DataTypeId::D64 => Self::D64(DecimalSerie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Decimal64Type>,
            >(
                array, field
            )?)),
            DataTypeId::D128 => Self::D128(DecimalSerie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Decimal128Type>,
            >(
                array, field
            )?)),
            DataTypeId::D256 => Self::D256(DecimalSerie::from_arrow_array(downcast::<
                arrow_array::PrimitiveArray<Decimal256Type>,
            >(
                array, field
            )?)),
            DataTypeId::Utf8 => Self::Utf8(Utf8Serie::from_arrow_array(downcast::<
                arrow_array::StringArray,
            >(
                array, field
            )?)?),
            DataTypeId::Binary => Self::Binary(BinarySerie::from_arrow_array(downcast::<
                arrow_array::BinaryArray,
            >(
                array, field
            )?)?),
            DataTypeId::FixedBinary => {
                Self::FixedBinary(FixedBinarySerie::from_arrow_array(downcast::<
                    arrow_array::FixedSizeBinaryArray,
                >(
                    array, field
                )?)?)
            }
            DataTypeId::FixedUtf8 => {
                Self::FixedUtf8(FixedUtf8Serie::from_arrow_array(downcast::<
                    arrow_array::FixedSizeBinaryArray,
                >(
                    array, field
                )?)?)
            }
            _ => return Err(unsupported(field)),
        })
    }
}
