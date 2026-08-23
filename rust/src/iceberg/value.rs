//! Iceberg's two renderings of one scalar value: its text and its bytes.
//!
//! Two places in the format store a value as text rather than as data: a
//! partition directory name and a snapshot summary entry. Both need the same
//! rendering, and neither can use the core [`Value`]'s serialization, because
//! `"XNAS"` must become `XNAS` and not `"XNAS"`.
//!
//! The rendering itself is not Iceberg's. A `column=value` directory is the
//! layout the whole project reads and writes, so the text comes from
//! [`crate::io::partition::partition_text`] - the same formatter, with the same
//! null spelling, that a partitioned folder write applies to a column. A table
//! this module writes is therefore a lake the rest of the crate can walk,
//! rather than one that happens to look like one.
//!
//! The textual rendering is deliberately not the inverse of anything. A
//! partition path spells a null value `null`, which is indistinguishable from
//! the string `"null"`, so a reader takes partition values from the manifest and
//! treats the path as layout only.
//!
//! The other rendering is the *single-value binary* one, which is what a
//! manifest bound and a manifest-list field summary carry. It is emitted only
//! for the types whose Parquet statistic bytes already are that encoding, which
//! is what lets [`super::statistics`] hand a footer's bytes straight to a
//! manifest and lets a scan compare a filter against them without decoding
//! either side. A type outside that set has no bound rather than a bound that
//! means something else.

use std::cmp::Ordering;

use smol_str::SmolStr;

use crate::{DataType, Value};

/// The literal Iceberg writes for a null partition value.
pub(super) const NULL_TEXT: &str = crate::io::partition::NULL_PARTITION;

/// Render one scalar value the way a `column=value` directory spells it.
///
/// A value that names no datatype - a sequence whose children disagree, a
/// mapping - has no directory spelling at all, so it falls back to its JSON
/// form: lossless and readable rather than invented. A partition tuple never
/// contains one, because a partition value is a scalar.
pub(super) fn scalar_text(value: &Value) -> SmolStr {
    crate::io::partition::partition_text(value).unwrap_or_else(|_| {
        crate::json::into_bytes(value)
            .ok()
            .and_then(|encoded| String::from_utf8(encoded).ok())
            .map_or_else(|| SmolStr::new_static(NULL_TEXT), SmolStr::new)
    })
}

/// Return whether a Parquet statistic byte string is also the Iceberg one.
///
/// A decimal is the case that differs - Parquet stores it big-endian in a fixed
/// width, Iceberg stores the minimal two's-complement big-endian - so a decimal
/// column gets counts but no bounds. A missing statistic costs a planner one
/// file read; a wrong one costs correctness.
pub(super) const fn is_portable(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Boolean
            | DataType::Int32
            | DataType::Int64
            | DataType::Float32
            | DataType::Float64
            | DataType::Date32
            | DataType::Time64(_)
            | DataType::Timestamp(_, _)
            | DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::FixedSizeBinary(_)
    )
}

/// Encode one scalar as the single value a manifest bound carries.
///
/// The datatype decides the encoding rather than the value's own variant,
/// because a column declared `Int32` still arrives as a 64-bit
/// [`Value::I64`]. A value that does not fit the column, and every type whose
/// encoding is not [`is_portable`], has no bytes rather than the wrong ones.
pub(super) fn single_value(value: &Value, data_type: &DataType) -> Option<Vec<u8>> {
    match data_type {
        DataType::Boolean => value.as_bool().map(|flag| vec![u8::from(flag)]),
        DataType::Int32 | DataType::Date32 => i32::try_from(count(value)?)
            .ok()
            .map(|number| number.to_le_bytes().to_vec()),
        DataType::Int64 | DataType::Time64(_) | DataType::Timestamp(_, _) => {
            Some(count(value)?.to_le_bytes().to_vec())
        }
        #[allow(clippy::cast_possible_truncation)]
        DataType::Float32 => value
            .as_f64()
            .map(|number| (number as f32).to_le_bytes().to_vec()),
        DataType::Float64 => value.as_f64().map(|number| number.to_le_bytes().to_vec()),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            value.as_str().map(|text| text.as_bytes().to_vec())
        }
        DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => value.as_bytes().map(<[u8]>::to_vec),
        _ => None,
    }
}

/// Read one scalar back out of the single value a manifest bound carries.
///
/// The inverse of [`single_value`], and the reason a manifest bound can be
/// handed to the crate's own statistics pruner: the pruner compares values,
/// not bytes, so a bound has to become a value exactly once. A type whose
/// encoding [`is_portable`] does not cover has no value rather than a wrong
/// one, and the pruner then simply declines.
pub(super) fn single_to_value(bytes: &[u8], data_type: &DataType) -> Option<Value> {
    match data_type {
        DataType::Boolean => bytes.first().map(|byte| Value::Bool(*byte != 0)),
        DataType::Int32 => Some(Value::I32(int32(bytes))),
        DataType::Date32 => Some(Value::date32(int32(bytes))),
        DataType::Int64 => Some(Value::I64(int64(bytes))),
        DataType::Time64(unit) => Value::time64(int64(bytes), *unit, crate::Timezone::NAIVE).ok(),
        DataType::Timestamp(unit, Some(zone)) => {
            Value::datetime64(int64(bytes), *unit, zone.clone()).ok()
        }
        DataType::Timestamp(unit, None) => {
            Value::datetime64(int64(bytes), *unit, crate::Timezone::NAIVE).ok()
        }
        DataType::Float32 => Some(Value::F32(crate::Float32::from_f32(float32(bytes)))),
        DataType::Float64 => Some(Value::F64(crate::Float64::from_f64(float64(bytes)))),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
            std::str::from_utf8(bytes).ok().map(Value::from)
        }
        DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => Some(Value::from(bytes)),
        _ => None,
    }
}

/// Read the integer count a value holds, whatever it counts.
///
/// A date counts days, a time counts its unit since midnight, and a timestamp
/// counts its unit since the epoch, so all three are one integer to an encoder.
fn count(value: &Value) -> Option<i64> {
    match value {
        Value::Date32(count, _, _)
        | Value::Time32(count, _, _)
        | Value::Duration32(count, _, _) => Some(i64::from(*count)),
        Value::Date64(count, _, _)
        | Value::Time64(count, _, _)
        | Value::DateTime64(count, _, _)
        | Value::Duration64(count, _, _) => Some(*count),
        other => other.as_i64(),
    }
}

/// Compare two single values the way their datatype orders them.
///
/// A little-endian integer does not order as bytes do, so folding bounds across
/// row groups, and testing a filter against one, has to decode before it
/// compares. Text and bytes are the exception: they order lexicographically in
/// both encodings.
pub(super) fn compare_single(left: &[u8], right: &[u8], data_type: &DataType) -> Ordering {
    match data_type {
        DataType::Boolean => left.first().cmp(&right.first()),
        DataType::Int32 | DataType::Date32 => int32(left).cmp(&int32(right)),
        DataType::Int64 | DataType::Time64(_) | DataType::Timestamp(_, _) => {
            int64(left).cmp(&int64(right))
        }
        DataType::Float32 => float32(left).total_cmp(&float32(right)),
        DataType::Float64 => float64(left).total_cmp(&float64(right)),
        _ => left.cmp(right),
    }
}

/// Decode a little-endian 32-bit integer, treating a short value as zero.
fn int32(bytes: &[u8]) -> i32 {
    bytes
        .get(..4)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
        .map_or(0, i32::from_le_bytes)
}

/// Decode a little-endian 64-bit integer, treating a short value as zero.
fn int64(bytes: &[u8]) -> i64 {
    bytes
        .get(..8)
        .and_then(|slice| <[u8; 8]>::try_from(slice).ok())
        .map_or(0, i64::from_le_bytes)
}

/// Decode a little-endian 32-bit float, treating a short value as zero.
fn float32(bytes: &[u8]) -> f32 {
    bytes
        .get(..4)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
        .map_or(0.0, f32::from_le_bytes)
}

/// Decode a little-endian 64-bit float, treating a short value as zero.
fn float64(bytes: &[u8]) -> f64 {
    bytes
        .get(..8)
        .and_then(|slice| <[u8; 8]>::try_from(slice).ok())
        .map_or(0.0, f64::from_le_bytes)
}
