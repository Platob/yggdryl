//! [`Column`] ↔ Arrow [`Array`](arrow_array::Array) — the leaf column bridge.
//!
//! [`column_to_arrow`] builds the closest Arrow array for every [`Column`] variant, and
//! [`column_from_arrow`] rebuilds our concrete carrier from an Arrow array + its [`ColumnField`]. The
//! nested `Struct` / `List` / `Map` arms **recurse** — a struct becomes a
//! [`StructArray`](arrow_array::StructArray), a list a [`ListArray`](arrow_array::ListArray) (`i32`
//! offsets), a map a [`MapArray`](arrow_array::MapArray) (a `List<Struct<key, value>>`) — through
//! arbitrary depth, reusing the leaf helpers. See the [module docs](super) for the closest-match map
//! and the copy profile.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Decimal128Array, Decimal256Array,
    FixedSizeBinaryArray, Float32Array, Float64Array, Int16Array, Int32Array, Int64Array,
    Int8Array, LargeBinaryArray, LargeStringArray, ListArray, MapArray, NullArray, StringArray,
    StructArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
use arrow_buffer::{i256, BooleanBuffer, Buffer, NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::{ArrowError, DataType, Field, Fields};

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Heap, IOBase, IoError};
use crate::typed::fixedbit::Bit;
use crate::typed::fixedbyte::{
    Decimal128, Decimal256, Decimal32, Decimal64, FixedBinary, FixedSizeSerie, FixedUtf8, Float32,
    Float64, Int128, Int16, Int32, Int64, Int8, UInt128, UInt16, UInt32, UInt64, UInt8, I256,
};
use crate::typed::nested::{
    Column, ColumnField, ListField, ListSerie, MapField, MapSerie, StructField, StructSerie,
};
use crate::typed::varbyte::{Binary, LargeBinary, LargeUtf8, Utf8};
use crate::typed::{
    Decoder, Encoder, Field as _, FixedSerie, HeaderField, Scalar, VarOffset, VarSerie, VarType,
};

use super::field::{column_field_to_arrow, force_non_nullable};

// ---- to/from macros (defined before use) -----------------------------------------------------

/// One numeric [`Column`] arm → the matching Arrow `PrimitiveArray`, over a reinterpreted (one-copy)
/// value buffer + the validity as a [`NullBuffer`].
macro_rules! primitive_to_arrow {
    ($serie:expr, $arrow:ty, $native:ty) => {{
        let len = $serie.len();
        let values = ScalarBuffer::<$native>::new(value_buffer($serie.data()), 0, len);
        let array: ArrayRef = Arc::new(
            <$arrow>::try_new(values, null_buffer($serie.validity(), len)).map_err(build_error)?,
        );
        Ok(array)
    }};
}

/// One variable-length [`Column`] arm → the matching Arrow `GenericByteArray`, from the column's own
/// offsets + data buffers (one copy each — least-copy) + validity.
macro_rules! var_to_arrow {
    ($serie:expr, $offset:ty, $arrow:ty) => {{
        let len = $serie.len();
        let offsets =
            ScalarBuffer::<$offset>::new(Buffer::from($serie.offsets().as_slice()), 0, len + 1);
        let offsets = OffsetBuffer::new(offsets);
        let values = Buffer::from($serie.data().as_slice());
        let array: ArrayRef = Arc::new(
            <$arrow>::try_new(offsets, values, null_buffer($serie.validity(), len))
                .map_err(build_error)?,
        );
        Ok(array)
    }};
}

/// One numeric `DataTypeId` arm → our `FixedSerie<Marker>` from the Arrow `PrimitiveArray`'s logical
/// values (one vectorized encode) + validity.
macro_rules! primitive_from_arrow {
    ($array:expr, $arrow:ty, $marker:ty, $name:expr, $nullable:expr) => {{
        let arr = downcast::<$arrow>($array, stringify!($arrow))?;
        let len = arr.len();
        let mut data =
            Heap::with_capacity(len * <$marker as crate::typed::DataType>::byte_width() as usize);
        <$marker as Encoder>::encode_slice(&mut data, 0, arr.values())?;
        let validity = validity_heap(arr.nulls(), len, $nullable);
        Ok(Column::from(named(
            FixedSerie::<$marker>::from_data(data, validity, len),
            $name,
        )))
    }};
}

/// One variable-length `DataTypeId` arm → our `VarSerie<Marker>`, rebuilt element-by-element so a
/// **sliced** input array is respected and the offsets are **rebased** from 0.
macro_rules! var_from_arrow {
    ($array:expr, $arrow:ty, $marker:ty, $offset:ty, $name:expr, $nullable:expr $(, $bytes:ident)?) => {{
        let arr = downcast::<$arrow>($array, stringify!($arrow))?;
        let len = arr.len();
        let width = <$offset as VarOffset>::WIDTH;
        let mut offsets = Heap::with_capacity(((len as u64 + 1) * width) as usize);
        <$offset as VarOffset>::write(&mut offsets, 0, 0)?;
        let mut data = Heap::new();
        let mut cursor: i64 = 0;
        for index in 0..len {
            let element: &[u8] = if arr.is_null(index) {
                &[]
            } else {
                arr.value(index) $(.$bytes())?
            };
            data.pwrite_byte_array(cursor as u64, element);
            cursor += element.len() as i64;
            <$offset as VarOffset>::write(&mut offsets, (index as u64 + 1) * width, cursor)?;
        }
        let validity = validity_heap(arr.nulls(), len, $nullable);
        let serie = VarSerie::<$marker>::from_parts(offsets, data, validity, len);
        let serie = match $name {
            Some(name) => serie.with_name(name),
            None => serie,
        };
        Ok(Column::from(serie))
    }};
}

// ---- shared errors ---------------------------------------------------------------------------

/// The guided error for an Arrow array that could not be built from a column's buffers. Shared with
/// the [`RecordBatch`](arrow_array::RecordBatch) bridge.
pub(super) fn build_error(error: ArrowError) -> IoError {
    IoError::TypedCast {
        detail: format!(
            "cannot build the Arrow array from the column's buffers: {error}; check the column \
             length, offsets, and decimal precision/scale"
        ),
    }
}

/// The guided error for an Arrow array whose runtime type does not match the field's declared one.
fn type_mismatch(expected: &str, actual: &DataType) -> IoError {
    IoError::TypedCast {
        detail: format!(
            "Arrow array is {actual:?}, not the {expected} the field declares; pass an array whose \
             Arrow type matches the field"
        ),
    }
}

// ---- to-Arrow buffer helpers -----------------------------------------------------------------

/// One copy of a source's bytes into an owning Arrow [`Buffer`]. The copy is unavoidable: the entry
/// point borrows the `&Column`, so its [`Heap`] cannot be moved into a zero-copy `Buffer::from_vec`.
/// The buffer is 64-byte aligned (via `from_slice_ref`), so reinterpreting it as a wider Arrow
/// native element type is sound.
fn value_buffer(data: &Heap) -> Buffer {
    Buffer::from(data.as_slice())
}

/// A source's validity bitmap (LSB-first, `1` = valid — identical to Arrow's convention) as an
/// Arrow [`NullBuffer`] of `len` bits, or `None` for a non-nullable column. One copy of the packed
/// bytes; padded to `ceil(len / 8)` bytes so a short/lazy validity buffer never underflows.
fn null_buffer(validity: Option<&Heap>, len: usize) -> Option<NullBuffer> {
    let validity = validity?;
    Some(NullBuffer::new(BooleanBuffer::new(
        packed_bytes(validity, len),
        0,
        len,
    )))
}

/// The first `ceil(len / 8)` bytes of a bit source as an owning Arrow [`Buffer`], zero-padded if the
/// source is short. Used for both the validity bitmap and a boolean column's packed data.
fn packed_bytes(source: &Heap, len: usize) -> Buffer {
    let need = len.div_ceil(8);
    let mut bytes = source.pread_vec(0, need);
    if bytes.len() < need {
        bytes.resize(need, 0);
    }
    Buffer::from_vec(bytes)
}

// ---- Column -> Arrow -------------------------------------------------------------------------

/// Builds the matching Arrow [`ArrayRef`] for any [`Column`] — a leaf array for a flat column, or a
/// recursively-built [`StructArray`](arrow_array::StructArray) / [`ListArray`](arrow_array::ListArray)
/// / [`MapArray`](arrow_array::MapArray) for a nested one (through arbitrary depth). See the
/// [module docs](super) for the closest-match map and the (one-copy) copy profile.
///
/// ```
/// use yggdryl_core::arrow::column_to_arrow;
/// use yggdryl_core::typed::{Column, FixedSerie};
/// use yggdryl_core::typed::fixedbyte::Int64;
/// use arrow_array::{Array, Int64Array};
///
/// let column = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
/// let array = column_to_arrow(&column).unwrap();
/// let ints = array.as_any().downcast_ref::<Int64Array>().unwrap();
/// assert_eq!(ints.values(), &[1, 2, 3]);
/// ```
pub fn column_to_arrow(column: &Column) -> Result<ArrayRef, IoError> {
    match column {
        Column::Null(n) => Ok(Arc::new(NullArray::new(*n))),
        Column::Int8(serie) => primitive_to_arrow!(serie, Int8Array, i8),
        Column::UInt8(serie) => primitive_to_arrow!(serie, UInt8Array, u8),
        Column::Int16(serie) => primitive_to_arrow!(serie, Int16Array, i16),
        Column::UInt16(serie) => primitive_to_arrow!(serie, UInt16Array, u16),
        Column::Int32(serie) => primitive_to_arrow!(serie, Int32Array, i32),
        Column::UInt32(serie) => primitive_to_arrow!(serie, UInt32Array, u32),
        Column::Int64(serie) => primitive_to_arrow!(serie, Int64Array, i64),
        Column::UInt64(serie) => primitive_to_arrow!(serie, UInt64Array, u64),
        // No 128-bit Arrow integer — a scale-0 Decimal128 over the same 16 raw bytes (module docs).
        Column::Int128(serie) => {
            decimal128_to_arrow(serie.data(), serie.validity(), serie.len(), 38, 0)
        }
        Column::UInt128(serie) => {
            decimal128_to_arrow(serie.data(), serie.validity(), serie.len(), 38, 0)
        }
        Column::Float32(serie) => primitive_to_arrow!(serie, Float32Array, f32),
        Column::Float64(serie) => primitive_to_arrow!(serie, Float64Array, f64),
        Column::Bool(serie) => {
            let len = serie.len();
            let values = BooleanBuffer::new(packed_bytes(serie.data(), len), 0, len);
            let array: ArrayRef = Arc::new(BooleanArray::new(
                values,
                null_buffer(serie.validity(), len),
            ));
            Ok(array)
        }
        // Decimal32/64 widen i32/i64 -> i128 (one owned Vec); Decimal128 reinterprets its bytes.
        Column::Decimal32(serie) => {
            let widened: Vec<i128> = serie.values().into_iter().map(i128::from).collect();
            decimal128_from_i128(
                widened,
                serie.validity(),
                serie.decimal_precision(),
                serie.decimal_scale(),
            )
        }
        Column::Decimal64(serie) => {
            let widened: Vec<i128> = serie.values().into_iter().map(i128::from).collect();
            decimal128_from_i128(
                widened,
                serie.validity(),
                serie.decimal_precision(),
                serie.decimal_scale(),
            )
        }
        Column::Decimal128(serie) => decimal128_to_arrow(
            serie.data(),
            serie.validity(),
            serie.len(),
            serie.decimal_precision().clamp(1, 38) as u8,
            serie.decimal_scale() as i8,
        ),
        Column::Decimal256(serie) => {
            let values: Vec<i256> = serie
                .values()
                .iter()
                .map(|v| i256::from_le_bytes(v.to_le_bytes()))
                .collect();
            let nulls = null_buffer(serie.validity(), serie.len());
            let array = Decimal256Array::try_new(ScalarBuffer::from(values), nulls)
                .map_err(build_error)?
                .with_precision_and_scale(
                    serie.decimal_precision().clamp(1, 76) as u8,
                    serie.decimal_scale() as i8,
                )
                .map_err(build_error)?;
            Ok(Arc::new(array))
        }
        Column::Binary(serie) => var_to_arrow!(serie, i32, BinaryArray),
        Column::Utf8(serie) => var_to_arrow!(serie, i32, StringArray),
        Column::LargeBinary(serie) => var_to_arrow!(serie, i64, LargeBinaryArray),
        Column::LargeUtf8(serie) => var_to_arrow!(serie, i64, LargeStringArray),
        Column::FixedBinary(serie) => fixed_size_to_arrow(serie),
        Column::FixedUtf8(serie) => fixed_size_to_arrow(serie),
        Column::Struct(serie) => struct_to_arrow(serie),
        Column::List(serie) => list_to_arrow(serie),
        Column::Map(serie) => map_to_arrow(serie),
    }
}

/// A validity [`NullBuffer`] built from a per-index `valid` predicate — the to-Arrow validity for a
/// nested carrier that exposes `is_valid` but not its raw bitmap. `None` for a non-`nullable`
/// carrier (no validity buffer); otherwise `len` bits, `1` = valid (Arrow's convention).
fn nulls_from_predicate<F: Fn(usize) -> bool>(
    len: usize,
    nullable: bool,
    valid: F,
) -> Option<NullBuffer> {
    if !nullable {
        return None;
    }
    let mut bits = vec![0u8; len.div_ceil(8)];
    for index in 0..len {
        if valid(index) {
            bits[index / 8] |= 1 << (index % 8);
        }
    }
    Some(NullBuffer::new(BooleanBuffer::new(
        Buffer::from_vec(bits),
        0,
        len,
    )))
}

/// The `len + 1` `i32` offsets of a nested carrier (list or map) as an Arrow [`OffsetBuffer`], from a
/// per-index `[start, end)` range accessor. `offsets[0]` is `0` and `offsets[i + 1]` is range `i`'s
/// end — contiguous, so the flattened child `[0, offsets[len])` is the referenced region.
fn offset_buffer<F: Fn(usize) -> (usize, usize)>(len: usize, range: F) -> OffsetBuffer<i32> {
    let mut offsets: Vec<i32> = Vec::with_capacity(len + 1);
    offsets.push(0);
    for index in 0..len {
        offsets.push(range(index).1 as i32);
    }
    OffsetBuffer::new(ScalarBuffer::from(offsets))
}

/// A [`StructSerie`] → an Arrow [`StructArray`]: each child column recurses through
/// [`column_to_arrow`], the child fields through [`column_field_to_arrow`], and the struct's
/// row-level validity becomes the array's [`NullBuffer`].
fn struct_to_arrow(serie: &StructSerie) -> Result<ArrayRef, IoError> {
    let len = serie.len();
    let struct_field = serie.field();
    let fields: Vec<Field> = struct_field
        .children()
        .iter()
        .map(column_field_to_arrow)
        .collect();
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(serie.num_columns());
    for column in serie.columns() {
        arrays.push(column_to_arrow(column)?);
    }
    let nulls = nulls_from_predicate(len, struct_field.nullable(), |index| serie.is_valid(index));
    let array = StructArray::try_new(Fields::from(fields), arrays, nulls).map_err(build_error)?;
    Ok(Arc::new(array))
}

/// A [`ListSerie`] → an Arrow [`ListArray`]: the flattened child recurses through
/// [`column_to_arrow`], the `i32` offsets become an [`OffsetBuffer`], and the list-level validity
/// the [`NullBuffer`].
fn list_to_arrow(serie: &ListSerie) -> Result<ArrayRef, IoError> {
    let len = serie.len();
    let list_field = serie.field();
    let item_field = Arc::new(column_field_to_arrow(list_field.item()));
    let values = column_to_arrow(serie.values())?;
    let offsets = offset_buffer(len, |index| serie.list_at(index).unwrap_or((0, 0)));
    let nulls = nulls_from_predicate(len, list_field.nullable(), |index| serie.is_valid(index));
    let array = ListArray::try_new(item_field, offsets, values, nulls).map_err(build_error)?;
    Ok(Arc::new(array))
}

/// A [`MapSerie`] → an Arrow [`MapArray`]: the flattened key + value columns recurse through
/// [`column_to_arrow`] into an `entries` [`StructArray`], the `i32` offsets become an
/// [`OffsetBuffer`], and the map-level validity the [`NullBuffer`]. The `entries` key field is forced
/// non-nullable (Arrow forbids null map keys) and `keys_sorted` rides along.
fn map_to_arrow(serie: &MapSerie) -> Result<ArrayRef, IoError> {
    let len = serie.len();
    let map_field = serie.field();
    let key_field = force_non_nullable(column_field_to_arrow(map_field.key()));
    let value_field = column_field_to_arrow(map_field.value());
    let entries_fields = Fields::from(vec![key_field, value_field]);

    let key_array = column_to_arrow(serie.keys())?;
    let value_array = column_to_arrow(serie.values())?;
    let entries = StructArray::try_new(entries_fields.clone(), vec![key_array, value_array], None)
        .map_err(build_error)?;
    let entries_field = Arc::new(Field::new(
        "entries",
        DataType::Struct(entries_fields),
        false,
    ));

    let offsets = offset_buffer(len, |index| serie.map_at(index).unwrap_or((0, 0)));
    let nulls = nulls_from_predicate(len, map_field.nullable(), |index| serie.is_valid(index));
    let array = MapArray::try_new(entries_field, offsets, entries, nulls, serie.keys_sorted())
        .map_err(build_error)?;
    Ok(Arc::new(array))
}

/// A `Decimal128Array` over an in-heap `i128` value buffer (reinterpreted, one copy) + validity.
fn decimal128_to_arrow(
    data: &Heap,
    validity: Option<&Heap>,
    len: usize,
    precision: u8,
    scale: i8,
) -> Result<ArrayRef, IoError> {
    let values = ScalarBuffer::<i128>::new(value_buffer(data), 0, len);
    let array = Decimal128Array::try_new(values, null_buffer(validity, len))
        .map_err(build_error)?
        .with_precision_and_scale(precision, scale)
        .map_err(build_error)?;
    Ok(Arc::new(array))
}

/// A `Decimal128Array` over an owned, already-widened `Vec<i128>` (zero-copy from the `Vec`) +
/// validity — the target for a `Decimal32` / `Decimal64` column.
fn decimal128_from_i128(
    values: Vec<i128>,
    validity: Option<&Heap>,
    precision: u32,
    scale: i32,
) -> Result<ArrayRef, IoError> {
    let len = values.len();
    let array = Decimal128Array::try_new(ScalarBuffer::from(values), null_buffer(validity, len))
        .map_err(build_error)?
        .with_precision_and_scale(precision.clamp(1, 38) as u8, scale as i8)
        .map_err(build_error)?;
    Ok(Arc::new(array))
}

/// A `FixedSizeBinaryArray` from a fixed-stride column's data buffer (one copy) + validity.
fn fixed_size_to_arrow<T: VarType>(serie: &FixedSizeSerie<T>) -> Result<ArrayRef, IoError> {
    let width = serie.width();
    if width == 0 {
        return Err(IoError::TypedCast {
            detail: "cannot convert a zero-width fixed-size column to an Arrow FixedSizeBinary; \
                     rebuild the column with a positive byte width"
                .to_owned(),
        });
    }
    let size = i32::try_from(width).map_err(|_| IoError::TypedCast {
        detail: format!(
            "fixed-size width {width} exceeds Arrow's i32 FixedSizeBinary limit; use a Binary \
             column for values this wide"
        ),
    })?;
    let values = Buffer::from(serie.data().as_slice());
    let array =
        FixedSizeBinaryArray::try_new(size, values, null_buffer(serie.validity(), serie.len()))
            .map_err(build_error)?;
    Ok(Arc::new(array))
}

// ---- Arrow -> Column -------------------------------------------------------------------------

/// Rebuilds our concrete leaf [`Column`] from an Arrow [`ArrayRef`] and its [`ColumnField`], using
/// the field to restore the exact internal type (e.g. `FixedUtf8` vs `FixedBinary`, a decimal's
/// precision / scale, or the `I128` / `U128` behind a `Decimal128`). Sliced / offset input arrays
/// are handled — every column is rebuilt through the logical Arrow accessors. A nested
/// [`ColumnField::Struct`] / [`List`](ColumnField::List) / [`Map`](ColumnField::Map) **recurses** into
/// the matching nested carrier.
///
/// ```
/// use yggdryl_core::arrow::{column_from_arrow, column_to_arrow};
/// use yggdryl_core::typed::{Column, FixedSerie, Value};
/// use yggdryl_core::typed::fixedbyte::Int64;
///
/// let column = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
/// let field = column.field();
/// let array = column_to_arrow(&column).unwrap();
/// let back = column_from_arrow(&array, &field).unwrap();
/// assert_eq!(back.len(), 3);
/// assert_eq!(back.get(2), Value::Int64(3));
/// ```
pub fn column_from_arrow(array: &ArrayRef, field: &ColumnField) -> Result<Column, IoError> {
    let leaf = match field {
        ColumnField::Leaf(header) => header,
        ColumnField::Struct(struct_field) => return struct_from_arrow(array, struct_field),
        ColumnField::List(list_field) => return list_from_arrow(array, list_field),
        ColumnField::Map(map_field) => return map_from_arrow(array, map_field),
    };
    let name = leaf.name();
    let nullable = leaf.nullable();

    match leaf.data_type_id() {
        DataTypeId::Unknown => Ok(Column::Null(array.len())),
        DataTypeId::Bool => {
            let arr = downcast::<BooleanArray>(array, "BooleanArray")?;
            let len = arr.len();
            let mut bits = vec![0u8; len.div_ceil(8)];
            for index in 0..len {
                if arr.value(index) {
                    bits[index / 8] |= 1 << (index % 8);
                }
            }
            let validity = validity_heap(arr.nulls(), len, nullable);
            let serie = named(
                FixedSerie::<Bit>::from_data(Heap::from_vec(bits), validity, len),
                name,
            );
            Ok(Column::from(serie))
        }
        DataTypeId::I8 => primitive_from_arrow!(array, Int8Array, Int8, name, nullable),
        DataTypeId::U8 => primitive_from_arrow!(array, UInt8Array, UInt8, name, nullable),
        DataTypeId::I16 => primitive_from_arrow!(array, Int16Array, Int16, name, nullable),
        DataTypeId::U16 => primitive_from_arrow!(array, UInt16Array, UInt16, name, nullable),
        DataTypeId::I32 => primitive_from_arrow!(array, Int32Array, Int32, name, nullable),
        DataTypeId::U32 => primitive_from_arrow!(array, UInt32Array, UInt32, name, nullable),
        DataTypeId::I64 => primitive_from_arrow!(array, Int64Array, Int64, name, nullable),
        DataTypeId::U64 => primitive_from_arrow!(array, UInt64Array, UInt64, name, nullable),
        DataTypeId::F32 => primitive_from_arrow!(array, Float32Array, Float32, name, nullable),
        DataTypeId::F64 => primitive_from_arrow!(array, Float64Array, Float64, name, nullable),
        // I128/U128 came from a Decimal128 (scale-0) carrying the raw 16 bytes.
        DataTypeId::I128 => {
            let arr = downcast::<Decimal128Array>(array, "Decimal128Array (from Int128)")?;
            let len = arr.len();
            let mut data = Heap::with_capacity(len * 16);
            <Int128 as Encoder>::encode_slice(&mut data, 0, arr.values())?;
            let validity = validity_heap(arr.nulls(), len, nullable);
            Ok(Column::from(named(
                FixedSerie::<Int128>::from_data(data, validity, len),
                name,
            )))
        }
        DataTypeId::U128 => {
            let arr = downcast::<Decimal128Array>(array, "Decimal128Array (from UInt128)")?;
            let len = arr.len();
            let values: Vec<u128> = arr.values().iter().map(|&v| v as u128).collect();
            let mut data = Heap::with_capacity(len * 16);
            <UInt128 as Encoder>::encode_slice(&mut data, 0, &values)?;
            let validity = validity_heap(arr.nulls(), len, nullable);
            Ok(Column::from(named(
                FixedSerie::<UInt128>::from_data(data, validity, len),
                name,
            )))
        }
        DataTypeId::Decimal32 => {
            let arr = downcast::<Decimal128Array>(array, "Decimal128Array (widened Decimal32)")?;
            let len = arr.len();
            let narrowed: Vec<i32> = arr.values().iter().map(|&v| v as i32).collect();
            let mut data = Heap::with_capacity(len * 4);
            <Decimal32 as Encoder>::encode_slice(&mut data, 0, &narrowed)?;
            let validity = validity_heap(arr.nulls(), len, nullable);
            let (precision, scale) = decimal_ps(leaf, arr.precision(), arr.scale());
            let serie = named(
                FixedSerie::<Decimal32>::from_data(data, validity, len),
                name,
            )
            .with_precision_scale(precision, scale);
            Ok(Column::from(serie))
        }
        DataTypeId::Decimal64 => {
            let arr = downcast::<Decimal128Array>(array, "Decimal128Array (widened Decimal64)")?;
            let len = arr.len();
            let narrowed: Vec<i64> = arr.values().iter().map(|&v| v as i64).collect();
            let mut data = Heap::with_capacity(len * 8);
            <Decimal64 as Encoder>::encode_slice(&mut data, 0, &narrowed)?;
            let validity = validity_heap(arr.nulls(), len, nullable);
            let (precision, scale) = decimal_ps(leaf, arr.precision(), arr.scale());
            let serie = named(
                FixedSerie::<Decimal64>::from_data(data, validity, len),
                name,
            )
            .with_precision_scale(precision, scale);
            Ok(Column::from(serie))
        }
        DataTypeId::Decimal128 => {
            let arr = downcast::<Decimal128Array>(array, "Decimal128Array")?;
            let len = arr.len();
            let mut data = Heap::with_capacity(len * 16);
            <Decimal128 as Encoder>::encode_slice(&mut data, 0, arr.values())?;
            let validity = validity_heap(arr.nulls(), len, nullable);
            let (precision, scale) = decimal_ps(leaf, arr.precision(), arr.scale());
            let serie = named(
                FixedSerie::<Decimal128>::from_data(data, validity, len),
                name,
            )
            .with_precision_scale(precision, scale);
            Ok(Column::from(serie))
        }
        DataTypeId::Decimal256 => {
            let arr = downcast::<Decimal256Array>(array, "Decimal256Array")?;
            let len = arr.len();
            let values: Vec<I256> = arr
                .values()
                .iter()
                .map(|v| I256::from_le_bytes(v.to_le_bytes()))
                .collect();
            let mut data = Heap::with_capacity(len * 32);
            <Decimal256 as Encoder>::encode_slice(&mut data, 0, &values)?;
            let validity = validity_heap(arr.nulls(), len, nullable);
            let (precision, scale) = decimal_ps(leaf, arr.precision(), arr.scale());
            let serie = named(
                FixedSerie::<Decimal256>::from_data(data, validity, len),
                name,
            )
            .with_precision_scale(precision, scale);
            Ok(Column::from(serie))
        }
        DataTypeId::Binary => var_from_arrow!(array, BinaryArray, Binary, i32, name, nullable),
        DataTypeId::Utf8 => {
            var_from_arrow!(array, StringArray, Utf8, i32, name, nullable, as_bytes)
        }
        DataTypeId::LargeBinary => {
            var_from_arrow!(array, LargeBinaryArray, LargeBinary, i64, name, nullable)
        }
        DataTypeId::LargeUtf8 => {
            var_from_arrow!(
                array,
                LargeStringArray,
                LargeUtf8,
                i64,
                name,
                nullable,
                as_bytes
            )
        }
        id @ (DataTypeId::FixedBinary | DataTypeId::FixedUtf8) => {
            let arr = downcast::<FixedSizeBinaryArray>(array, "FixedSizeBinaryArray")?;
            let len = arr.len();
            let width = arr.value_length().max(0) as usize;
            let mut data = Heap::with_capacity(len * width);
            for index in 0..len {
                data.pwrite_byte_array(index as u64 * width as u64, arr.value(index));
            }
            let validity = validity_heap(arr.nulls(), len, nullable);
            if id == DataTypeId::FixedBinary {
                let serie = named_fixed(
                    FixedSizeSerie::<FixedBinary>::from_parts(data, validity, len, width),
                    name,
                );
                Ok(Column::from(serie))
            } else {
                let serie = named_fixed(
                    FixedSizeSerie::<FixedUtf8>::from_parts(data, validity, len, width),
                    name,
                );
                Ok(Column::from(serie))
            }
        }
        // A `ColumnField::Leaf` that nonetheless declares a nested id is contradictory — the nested
        // arms above (matched on the field, not the id) own every real nested column.
        DataTypeId::Struct | DataTypeId::List | DataTypeId::Map => Err(IoError::TypedCast {
            detail: format!(
                "a leaf field declares the nested type {:?}, which has no leaf carrier: describe a \
                 nested column with a ColumnField::Struct / List / Map, not a leaf field",
                leaf.data_type_id()
            ),
        }),
    }
}

/// An Arrow [`StructArray`] → a [`StructSerie`]: each child array rebuilds through
/// [`column_from_arrow`] with the matching child [`ColumnField`], then the row-level validity is
/// restored. A **sliced** struct array carries sliced children, so the recursion respects the slice.
fn struct_from_arrow(array: &ArrayRef, field: &StructField) -> Result<Column, IoError> {
    let arr = downcast::<StructArray>(array, "StructArray")?;
    let len = arr.len();
    if field.num_fields() != arr.num_columns() {
        return Err(IoError::TypedCast {
            detail: format!(
                "struct field declares {} child column(s) but the Arrow StructArray has {}: pass \
                 the struct schema that matches the array",
                field.num_fields(),
                arr.num_columns()
            ),
        });
    }
    let mut columns = Vec::with_capacity(field.num_fields());
    for (index, child_field) in field.children().iter().enumerate() {
        columns.push(column_from_arrow(arr.column(index), child_field)?);
    }
    let mut serie = StructSerie::from_columns(columns)?;
    if let Some(name) = field.name() {
        serie = serie.with_name(name);
    }
    *serie.metadata_mut() = field.metadata().clone();
    let validity = validity_heap(arr.nulls(), len, field.nullable());
    Ok(Column::from(serie.with_validity(validity)))
}

/// An Arrow [`ListArray`] → a [`ListSerie`]: the referenced value range `[offsets[0], offsets[len])`
/// is sliced out of the values array and rebuilt through [`column_from_arrow`], the offsets are
/// **rebased** from 0, and the list-level validity is restored. Respects a **sliced** list array (its
/// logical offsets index the full values array).
fn list_from_arrow(array: &ArrayRef, field: &ListField) -> Result<Column, IoError> {
    let arr = downcast::<ListArray>(array, "ListArray")?;
    let len = arr.len();
    let offsets = arr.value_offsets();
    let first = offsets[0];
    let last = offsets[len];
    let span = (last - first).max(0) as usize;
    let value_slice = arr.values().slice(first.max(0) as usize, span);
    let child = column_from_arrow(&value_slice, field.item())?;

    let mut offset_heap = Heap::with_capacity((len + 1) * 4);
    for (index, &offset) in offsets.iter().enumerate() {
        offset_heap.pwrite_i32(index as u64 * 4, offset - first)?;
    }
    let validity = validity_heap(arr.nulls(), len, field.nullable());
    let serie = ListSerie::from_offsets(
        field.name().unwrap_or(""),
        offset_heap,
        child,
        validity,
        len,
    );
    Ok(Column::from(serie))
}

/// An Arrow [`MapArray`] → a [`MapSerie`]: the referenced entry range `[offsets[0], offsets[len])` is
/// sliced out of the key + value arrays and rebuilt through [`column_from_arrow`], the offsets are
/// **rebased** from 0, and the map-level validity + `keys_sorted` restored. Respects a **sliced** map
/// array.
fn map_from_arrow(array: &ArrayRef, field: &MapField) -> Result<Column, IoError> {
    let arr = downcast::<MapArray>(array, "MapArray")?;
    let len = arr.len();
    let offsets = arr.value_offsets();
    let first = offsets[0];
    let last = offsets[len];
    let start = first.max(0) as usize;
    let span = (last - first).max(0) as usize;
    let key_col = column_from_arrow(&arr.keys().slice(start, span), field.key())?;
    let value_col = column_from_arrow(&arr.values().slice(start, span), field.value())?;

    let mut offset_heap = Heap::with_capacity((len + 1) * 4);
    for (index, &offset) in offsets.iter().enumerate() {
        offset_heap.pwrite_i32(index as u64 * 4, offset - first)?;
    }
    let validity = validity_heap(arr.nulls(), len, field.nullable());
    let keys_sorted = matches!(arr.data_type(), DataType::Map(_, true));
    let serie = MapSerie::from_offsets(
        field.name().unwrap_or(""),
        offset_heap,
        key_col,
        value_col,
        validity,
        len,
    )?
    .with_keys_sorted(keys_sorted);
    Ok(Column::from(serie))
}

// ---- from-Arrow helpers ----------------------------------------------------------------------

/// Downcasts an Arrow array to a concrete type, or a guided [`IoError`] naming the mismatch.
fn downcast<'a, A: Array + 'static>(array: &'a ArrayRef, expected: &str) -> Result<&'a A, IoError> {
    array
        .as_any()
        .downcast_ref::<A>()
        .ok_or_else(|| type_mismatch(expected, array.data_type()))
}

/// An Arrow [`NullBuffer`] (LSB-first, `1` = valid — matching our convention) as a validity
/// [`Heap`], respecting the buffer's **logical** positions (so a sliced array is handled). `None`
/// when the array has no nulls and the field is non-nullable; an all-valid buffer when the field is
/// nullable but the array carried no null buffer (preserving the declared nullability).
fn validity_heap(nulls: Option<&NullBuffer>, len: usize, nullable: bool) -> Option<Heap> {
    match nulls {
        Some(nulls) => {
            let mut bits = vec![0u8; len.div_ceil(8)];
            for index in 0..len {
                if nulls.is_valid(index) {
                    bits[index / 8] |= 1 << (index % 8);
                }
            }
            Some(Heap::from_vec(bits))
        }
        None if nullable => {
            let mut bits = vec![0u8; len.div_ceil(8)];
            for index in 0..len {
                bits[index / 8] |= 1 << (index % 8);
            }
            Some(Heap::from_vec(bits))
        }
        None => None,
    }
}

/// The decimal precision / scale to restore — the field's when present, else the Arrow array's.
fn decimal_ps(leaf: &HeaderField, precision: u8, scale: i8) -> (u32, i32) {
    (
        leaf.precision().unwrap_or(precision as u32),
        leaf.scale().unwrap_or(scale as i32),
    )
}

/// Applies the optional column name to a rebuilt fixed-width / variable-length carrier.
fn named<T: Encoder + Decoder>(serie: FixedSerie<T>, name: Option<&str>) -> FixedSerie<T> {
    match name {
        Some(name) => serie.with_name(name),
        None => serie,
    }
}

/// Applies the optional column name to a rebuilt fixed-size byte carrier.
fn named_fixed<T: VarType>(serie: FixedSizeSerie<T>, name: Option<&str>) -> FixedSizeSerie<T> {
    match name {
        Some(name) => serie.with_name(name),
        None => serie,
    }
}
