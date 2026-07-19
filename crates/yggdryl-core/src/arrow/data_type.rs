//! [`DataTypeId`] (+ its field params) ↔ Arrow [`DataType`] — the total, closest-match type map.
//!
//! See the [module docs](super) for the full closest-match table and every documented lossy edge.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Fields};

use crate::datatype_id::DataTypeId;

/// The closest Arrow [`DataType`] for a leaf [`DataTypeId`] and its field params (`precision` /
/// `scale` for the decimals, `byte_width` for the fixed-size byte types). **Total** — every id maps
/// to something; the nested ids map to a documented structural shell (empty children), since a
/// nested type's real children can't be recovered from the id alone. See the [module docs](super)
/// for the closest-match table and each lossy edge.
///
/// ```
/// use yggdryl_core::arrow::to_arrow_data_type;
/// use yggdryl_core::datatype_id::DataTypeId;
/// use arrow_schema::DataType;
///
/// assert_eq!(to_arrow_data_type(DataTypeId::I32, None, None, None), DataType::Int32);
/// // Decimal32 widens to Decimal128 (documented lossy edge).
/// assert_eq!(
///     to_arrow_data_type(DataTypeId::Decimal32, Some(9), Some(2), None),
///     DataType::Decimal128(9, 2)
/// );
/// // FixedUtf8 has no Arrow equivalent — closest is FixedSizeBinary.
/// assert_eq!(
///     to_arrow_data_type(DataTypeId::FixedUtf8, None, None, Some(4)),
///     DataType::FixedSizeBinary(4)
/// );
/// ```
pub fn to_arrow_data_type(
    id: DataTypeId,
    precision: Option<u32>,
    scale: Option<i32>,
    byte_width: Option<u32>,
) -> DataType {
    // A `Decimal128(precision, scale)`, defaulting precision to the source type's max and scale to 0.
    let decimal128 = |default_precision: u32| {
        let p = precision.unwrap_or(default_precision).clamp(1, 38) as u8;
        let s = scale.unwrap_or(0) as i8;
        DataType::Decimal128(p, s)
    };

    match id {
        DataTypeId::Unknown => DataType::Null,
        DataTypeId::Bool => DataType::Boolean,
        DataTypeId::I8 => DataType::Int8,
        DataTypeId::U8 => DataType::UInt8,
        DataTypeId::I16 => DataType::Int16,
        DataTypeId::U16 => DataType::UInt16,
        DataTypeId::I32 => DataType::Int32,
        DataTypeId::U32 => DataType::UInt32,
        DataTypeId::I64 => DataType::Int64,
        DataTypeId::U64 => DataType::UInt64,
        // Arrow has no 128-bit integer — the closest is a scale-0 Decimal128 (an i128 is exactly a
        // scale-0 decimal). A u128 ≥ 2^127 presents as negative on the Arrow side, but the 16 raw
        // bytes round-trip losslessly. See the module docs.
        DataTypeId::I128 | DataTypeId::U128 => DataType::Decimal128(38, 0),
        DataTypeId::F32 => DataType::Float32,
        DataTypeId::F64 => DataType::Float64,
        // Decimal32/64/128 -> Decimal128 (documented widening); Decimal256 -> Decimal256.
        DataTypeId::Decimal32 => decimal128(9),
        DataTypeId::Decimal64 => decimal128(18),
        DataTypeId::Decimal128 => decimal128(38),
        DataTypeId::Decimal256 => {
            let p = precision.unwrap_or(76).clamp(1, 76) as u8;
            let s = scale.unwrap_or(0) as i8;
            DataType::Decimal256(p, s)
        }
        DataTypeId::Binary => DataType::Binary,
        DataTypeId::LargeBinary => DataType::LargeBinary,
        DataTypeId::Utf8 => DataType::Utf8,
        DataTypeId::LargeUtf8 => DataType::LargeUtf8,
        // Arrow has no fixed-size UTF-8 — the closest is FixedSizeBinary; the reverse restores
        // FixedUtf8 only from our own field metadata (a bare FixedSizeBinary maps back to
        // FixedBinary). The width comes from the field; default to 1 when absent.
        DataTypeId::FixedBinary | DataTypeId::FixedUtf8 => {
            DataType::FixedSizeBinary(byte_width.unwrap_or(1).max(1) as i32)
        }
        // Nested types: a documented structural shell — the children can't come from the id alone,
        // so the nested arrow phase fills in the real mapping.
        DataTypeId::Struct => DataType::Struct(Fields::empty()),
        DataTypeId::List => DataType::List(Arc::new(Field::new("item", DataType::Null, true))),
        DataTypeId::Map => {
            let entries = DataType::Struct(Fields::from(vec![
                Field::new("keys", DataType::Null, false),
                Field::new("values", DataType::Null, true),
            ]));
            DataType::Map(Arc::new(Field::new("entries", entries, false)), false)
        }
    }
}

/// The **leaf-only inverse** of [`to_arrow_data_type`]: an Arrow [`DataType`] → the matching
/// [`DataTypeId`] and its params `(precision, scale, byte_width)`. Total — an Arrow type this crate
/// has no leaf for (e.g. `Float16`, the temporal / view / union types) degrades to
/// [`Unknown`](DataTypeId::Unknown); the nested `Struct` / `List` / `Map` types return their marker
/// id (the nested phase owns the real mapping).
///
/// `FixedSizeBinary(w)` maps back to [`FixedBinary`](DataTypeId::FixedBinary) + width `w` —
/// [`FixedUtf8`](DataTypeId::FixedUtf8) is only recoverable from our own field metadata, never from
/// the Arrow type alone.
///
/// ```
/// use yggdryl_core::arrow::from_arrow_data_type;
/// use yggdryl_core::datatype_id::DataTypeId;
/// use arrow_schema::DataType;
///
/// assert_eq!(from_arrow_data_type(&DataType::Int64), (DataTypeId::I64, None, None, None));
/// assert_eq!(
///     from_arrow_data_type(&DataType::Decimal128(10, 2)),
///     (DataTypeId::Decimal128, Some(10), Some(2), None)
/// );
/// assert_eq!(
///     from_arrow_data_type(&DataType::FixedSizeBinary(8)),
///     (DataTypeId::FixedBinary, None, None, Some(8))
/// );
/// ```
pub fn from_arrow_data_type(dt: &DataType) -> (DataTypeId, Option<u32>, Option<i32>, Option<u32>) {
    match dt {
        DataType::Null => (DataTypeId::Unknown, None, None, None),
        DataType::Boolean => (DataTypeId::Bool, None, None, None),
        DataType::Int8 => (DataTypeId::I8, None, None, None),
        DataType::UInt8 => (DataTypeId::U8, None, None, None),
        DataType::Int16 => (DataTypeId::I16, None, None, None),
        DataType::UInt16 => (DataTypeId::U16, None, None, None),
        DataType::Int32 => (DataTypeId::I32, None, None, None),
        DataType::UInt32 => (DataTypeId::U32, None, None, None),
        DataType::Int64 => (DataTypeId::I64, None, None, None),
        DataType::UInt64 => (DataTypeId::U64, None, None, None),
        DataType::Float32 => (DataTypeId::F32, None, None, None),
        DataType::Float64 => (DataTypeId::F64, None, None, None),
        DataType::Decimal32(p, s) => (
            DataTypeId::Decimal32,
            Some(*p as u32),
            Some(*s as i32),
            None,
        ),
        DataType::Decimal64(p, s) => (
            DataTypeId::Decimal64,
            Some(*p as u32),
            Some(*s as i32),
            None,
        ),
        DataType::Decimal128(p, s) => (
            DataTypeId::Decimal128,
            Some(*p as u32),
            Some(*s as i32),
            None,
        ),
        DataType::Decimal256(p, s) => (
            DataTypeId::Decimal256,
            Some(*p as u32),
            Some(*s as i32),
            None,
        ),
        DataType::Binary => (DataTypeId::Binary, None, None, None),
        DataType::LargeBinary => (DataTypeId::LargeBinary, None, None, None),
        DataType::Utf8 => (DataTypeId::Utf8, None, None, None),
        DataType::LargeUtf8 => (DataTypeId::LargeUtf8, None, None, None),
        DataType::FixedSizeBinary(w) => (
            DataTypeId::FixedBinary,
            None,
            None,
            Some((*w).max(0) as u32),
        ),
        DataType::Struct(_) => (DataTypeId::Struct, None, None, None),
        DataType::List(_) | DataType::LargeList(_) | DataType::FixedSizeList(_, _) => {
            (DataTypeId::List, None, None, None)
        }
        DataType::Map(_, _) => (DataTypeId::Map, None, None, None),
        // Every other Arrow type (Float16, temporal, view, union, dictionary, run-end, …) has no
        // leaf here yet — degrade to raw bytes.
        _ => (DataTypeId::Unknown, None, None, None),
    }
}
