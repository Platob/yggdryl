//! The encode walk: one [`Value`] tree into Variant value bytes.
//!
//! The walk is two passes over the same tree: [`Dictionary::build`] collects
//! and sorts the object field names, then this pass spells every node against
//! that dictionary. A refusal - a kind the format cannot carry back, a number
//! outside its range - reports the `$`-rooted path of the offending node, so
//! the caller fixes the value without reading source.

use smol_str::{SmolStr, format_smolstr};

use crate::path::{Path, Segment};
use crate::{Error, Limits, Result, TimeUnit, Value};

use super::metadata::Dictionary;
use super::{
    BASIC_ARRAY, BASIC_OBJECT, BASIC_SHORT_STRING, DECIMAL_MAGNITUDE_LIMIT, PRIMITIVE_BINARY,
    PRIMITIVE_DATE, PRIMITIVE_DECIMAL4, PRIMITIVE_DECIMAL8, PRIMITIVE_DECIMAL16, PRIMITIVE_DOUBLE,
    PRIMITIVE_FALSE, PRIMITIVE_FLOAT, PRIMITIVE_INT8, PRIMITIVE_INT16, PRIMITIVE_INT32,
    PRIMITIVE_INT64, PRIMITIVE_NULL, PRIMITIVE_STRING, PRIMITIVE_TIME_MICROS,
    PRIMITIVE_TIMESTAMP_MICROS, PRIMITIVE_TIMESTAMP_NANOS, PRIMITIVE_TIMESTAMP_NTZ_MICROS,
    PRIMITIVE_TIMESTAMP_NTZ_NANOS, PRIMITIVE_TRUE, SHORT_STRING_LIMIT, byte_width, push_unsigned,
};

/// Encode one value tree as its Variant `(metadata, value)` byte pair.
///
/// The metadata is always version 1 with a sorted, deduplicated field-name
/// dictionary; the value uses the smallest offset, field-id, and count widths
/// that hold each container, and the short-string form for every string under
/// 64 bytes. [`super::decode_value`] reads the pair back.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] naming the failing node's `$`-rooted path
/// for every value the encoding cannot carry back losslessly: a `Duration` or
/// `Geospatial` value, a non-UTC `Timestamp`, an integer beyond `i64`, a
/// decimal outside the 38-digit range, a temporal count whose microsecond
/// widening loses information, a non-string mapping key, or nesting past the
/// default depth limit.
pub fn encode_value(value: &Value) -> Result<(Vec<u8>, Vec<u8>)> {
    let dictionary = Dictionary::build(value);
    let metadata = dictionary.encode()?;
    let mut bytes = Vec::new();
    write_value(&mut bytes, value, &dictionary, &Path::root(), 1)?;
    Ok((metadata, bytes))
}

/// Refuse a value the encoding cannot carry, at its path.
fn invalid(path: &Path<'_>, reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::from(path.render()),
        reason,
    }
}

/// Append the value-metadata byte of one primitive type id.
fn push_primitive(target: &mut Vec<u8>, id: u8) {
    target.push(id << 2);
}

/// Append one primitive with its fixed-width little-endian value data.
fn push_scalar<const WIDTH: usize>(target: &mut Vec<u8>, id: u8, data: [u8; WIDTH]) {
    push_primitive(target, id);
    target.extend_from_slice(&data);
}

/// Refuse a container nested past the default depth limit, at its path.
///
/// The limit bounds the decode-side recursion of everything this encoder
/// writes; only containers check it, because only containers nest.
fn check_depth(depth: usize, path: &Path<'_>) -> Result<()> {
    let depth_limit = Limits::default().max_depth();
    if depth > depth_limit {
        return Err(invalid(
            path,
            format_smolstr!("expected a value at most {depth_limit} levels deep"),
        ));
    }
    Ok(())
}

/// Spell one node, appending its complete Variant value encoding.
///
/// `depth` is the container nesting level this value sits at; the root is
/// level 1.
#[allow(clippy::too_many_lines)]
fn write_value(
    target: &mut Vec<u8>,
    value: &Value,
    dictionary: &Dictionary,
    path: &Path<'_>,
    depth: usize,
) -> Result<()> {
    match value {
        Value::Null => push_primitive(target, PRIMITIVE_NULL),
        Value::Bool(true) => push_primitive(target, PRIMITIVE_TRUE),
        Value::Bool(false) => push_primitive(target, PRIMITIVE_FALSE),
        Value::I8(value) => push_scalar(target, PRIMITIVE_INT8, value.to_le_bytes()),
        Value::I16(value) => push_scalar(target, PRIMITIVE_INT16, value.to_le_bytes()),
        Value::I32(value) => push_scalar(target, PRIMITIVE_INT32, value.to_le_bytes()),
        Value::I64(value) => push_scalar(target, PRIMITIVE_INT64, value.to_le_bytes()),
        // Unsigned widths widen into the next signed width, staying inside
        // the integer kind; the 64- and 128-bit kinds have no wider signed
        // spelling, so they encode as int64 exactly when the value fits.
        Value::U8(value) => push_scalar(target, PRIMITIVE_INT16, i16::from(*value).to_le_bytes()),
        Value::U16(value) => push_scalar(target, PRIMITIVE_INT32, i32::from(*value).to_le_bytes()),
        Value::U32(value) => push_scalar(target, PRIMITIVE_INT64, i64::from(*value).to_le_bytes()),
        Value::U64(value) => {
            let value = i64::try_from(*value).map_err(|_| {
                invalid(
                    path,
                    format_smolstr!(
                        "expected an integer within the Variant int64 range, got {value}"
                    ),
                )
            })?;
            push_scalar(target, PRIMITIVE_INT64, value.to_le_bytes());
        }
        Value::I128(value) => {
            let value = i64::try_from(*value).map_err(|_| {
                invalid(
                    path,
                    format_smolstr!(
                        "expected an integer within the Variant int64 range, got {value}"
                    ),
                )
            })?;
            push_scalar(target, PRIMITIVE_INT64, value.to_le_bytes());
        }
        Value::U128(value) => {
            let value = i64::try_from(*value).map_err(|_| {
                invalid(
                    path,
                    format_smolstr!(
                        "expected an integer within the Variant int64 range, got {value}"
                    ),
                )
            })?;
            push_scalar(target, PRIMITIVE_INT64, value.to_le_bytes());
        }
        Value::F32(value) => push_scalar(target, PRIMITIVE_FLOAT, value.as_f32().to_le_bytes()),
        Value::F64(value) => push_scalar(target, PRIMITIVE_DOUBLE, value.as_f64().to_le_bytes()),
        Value::Decimal(unscaled, scale) => write_decimal(target, *unscaled, *scale, path)?,
        Value::String(value) => write_string(target, value, path)?,
        Value::Bytes(value) => {
            let length = u32::try_from(value.len()).map_err(|_| {
                invalid(
                    path,
                    format_smolstr!(
                        "expected at most {} binary bytes, got {}",
                        u32::MAX,
                        value.len()
                    ),
                )
            })?;
            push_primitive(target, PRIMITIVE_BINARY);
            target.extend_from_slice(&length.to_le_bytes());
            target.extend_from_slice(value);
        }
        // Spelling WKB as the binary primitive would decode as plain bytes,
        // silently dropping the geospatial reading, so the kind is refused
        // until the format grows a geometry primitive.
        Value::Geospatial(_) => {
            return Err(invalid(
                path,
                format_smolstr!("expected a Variant-encodable value, got a geospatial value"),
            ));
        }
        Value::Date(value) => push_scalar(target, PRIMITIVE_DATE, value.to_le_bytes()),
        Value::Time(count, unit) => {
            // The Variant time primitive is microseconds only, so coarser
            // units widen and whole-microsecond nanoseconds divide down.
            let micros = match unit {
                TimeUnit::Nanosecond if count % 1_000 == 0 => count / 1_000,
                TimeUnit::Nanosecond => {
                    return Err(invalid(
                        path,
                        format_smolstr!(
                            "expected a time in whole microseconds, got {count} nanoseconds"
                        ),
                    ));
                }
                _ => widen_to_micros(*count, *unit, path)?,
            };
            push_scalar(target, PRIMITIVE_TIME_MICROS, micros.to_le_bytes());
        }
        Value::Timestamp(count, unit, zone) => {
            // The Variant instant is UTC-adjusted and carries no zone name,
            // so any other zone could not come back from the bytes.
            if !zone.is_utc() {
                return Err(invalid(
                    path,
                    format_smolstr!("expected a UTC timestamp, got zone {:?}", zone.as_str()),
                ));
            }
            write_instant(
                target,
                *count,
                *unit,
                PRIMITIVE_TIMESTAMP_MICROS,
                PRIMITIVE_TIMESTAMP_NANOS,
                path,
            )?;
        }
        Value::DateTime(count, unit) => write_instant(
            target,
            *count,
            *unit,
            PRIMITIVE_TIMESTAMP_NTZ_MICROS,
            PRIMITIVE_TIMESTAMP_NTZ_NANOS,
            path,
        )?,
        Value::Duration(count, _) => {
            return Err(invalid(
                path,
                format_smolstr!("expected a Variant-encodable value, got a duration of {count}"),
            ));
        }
        Value::Sequence(values) => {
            check_depth(depth, path)?;
            let mut fields = Vec::new();
            let mut offsets = Vec::with_capacity(values.len() + 1);
            for (index, child) in values.iter().enumerate() {
                offsets.push(fields.len());
                let child_path = path.child(Segment::Index(index));
                write_value(&mut fields, child, dictionary, &child_path, depth + 1)?;
            }
            offsets.push(fields.len());
            write_array(target, &offsets, &fields, path)?;
        }
        Value::Mapping(entries) => {
            check_depth(depth, path)?;
            let mut ordered = Vec::with_capacity(entries.len());
            for (index, (key, child)) in entries.iter().enumerate() {
                let Value::String(name) = key else {
                    return Err(invalid(
                        &path.child(Segment::MapKey(index)),
                        format_smolstr!("expected a string object key, got {}", key.kind()),
                    ));
                };
                ordered.push((name.as_str(), child));
            }
            // Field ids and offsets are spelled in unsigned-byte
            // lexicographic name order, as the specification mandates.
            ordered.sort_unstable_by(|left, right| left.0.cmp(right.0));
            write_object(target, &ordered, dictionary, path, depth)?;
        }
        Value::Record(data_type, values) => {
            check_depth(depth, path)?;
            let Some(fields) = data_type.as_fields() else {
                return Err(invalid(
                    path,
                    format_smolstr!("expected a struct record datatype, got {data_type}"),
                ));
            };
            let mut ordered = Vec::with_capacity(values.len());
            for (field, child) in fields.iter().zip(values.iter()) {
                ordered.push((field.name(), child));
            }
            ordered.sort_unstable_by(|left, right| left.0.cmp(right.0));
            write_object(target, &ordered, dictionary, path, depth)?;
        }
    }
    Ok(())
}

/// Append one decimal at the narrowest physical width holding its unscaled
/// value, refusing the scales and precisions the specification excludes.
fn write_decimal(target: &mut Vec<u8>, unscaled: i128, scale: i8, path: &Path<'_>) -> Result<()> {
    if !(0..=38).contains(&scale) {
        return Err(invalid(
            path,
            format_smolstr!("expected a decimal scale between 0 and 38, got {scale}"),
        ));
    }
    if unscaled <= -DECIMAL_MAGNITUDE_LIMIT || unscaled >= DECIMAL_MAGNITUDE_LIMIT {
        return Err(invalid(
            path,
            format_smolstr!(
                "expected a decimal unscaled value of at most 38 digits, got {unscaled}"
            ),
        ));
    }
    let scale_byte = u8::try_from(scale).expect("the scale was bounded to 0..=38");
    if let Ok(narrow) = i32::try_from(unscaled) {
        push_primitive(target, PRIMITIVE_DECIMAL4);
        target.push(scale_byte);
        target.extend_from_slice(&narrow.to_le_bytes());
    } else if let Ok(narrow) = i64::try_from(unscaled) {
        push_primitive(target, PRIMITIVE_DECIMAL8);
        target.push(scale_byte);
        target.extend_from_slice(&narrow.to_le_bytes());
    } else {
        push_primitive(target, PRIMITIVE_DECIMAL16);
        target.push(scale_byte);
        target.extend_from_slice(&unscaled.to_le_bytes());
    }
    Ok(())
}

/// Append one string: the short-string form under 64 bytes, the length-
/// prefixed string primitive otherwise.
fn write_string(target: &mut Vec<u8>, value: &str, path: &Path<'_>) -> Result<()> {
    if value.len() < SHORT_STRING_LIMIT {
        let length = u8::try_from(value.len()).expect("a short string is under 64 bytes");
        target.push(length << 2 | BASIC_SHORT_STRING);
        target.extend_from_slice(value.as_bytes());
        return Ok(());
    }
    let length = u32::try_from(value.len()).map_err(|_| {
        invalid(
            path,
            format_smolstr!(
                "expected at most {} string bytes, got {}",
                u32::MAX,
                value.len()
            ),
        )
    })?;
    push_primitive(target, PRIMITIVE_STRING);
    target.extend_from_slice(&length.to_le_bytes());
    target.extend_from_slice(value.as_bytes());
    Ok(())
}

/// Append one epoch instant, widening seconds and milliseconds into the
/// microsecond primitive and keeping nanoseconds at their own.
fn write_instant(
    target: &mut Vec<u8>,
    count: i64,
    unit: TimeUnit,
    micros_id: u8,
    nanos_id: u8,
    path: &Path<'_>,
) -> Result<()> {
    if matches!(unit, TimeUnit::Nanosecond) {
        push_scalar(target, nanos_id, count.to_le_bytes());
        return Ok(());
    }
    let micros = widen_to_micros(count, unit, path)?;
    push_scalar(target, micros_id, micros.to_le_bytes());
    Ok(())
}

/// Widen a second, millisecond, or microsecond count to microseconds,
/// refusing overflow and the calendar interval units by name.
fn widen_to_micros(count: i64, unit: TimeUnit, path: &Path<'_>) -> Result<i64> {
    let factor = match unit {
        TimeUnit::Second => 1_000_000,
        TimeUnit::Millisecond => 1_000,
        TimeUnit::Microsecond => 1,
        TimeUnit::Nanosecond | TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => {
            return Err(invalid(
                path,
                format_smolstr!("expected a second through microsecond unit, got {unit}"),
            ));
        }
    };
    count.checked_mul(factor).ok_or_else(|| {
        invalid(
            path,
            format_smolstr!("expected a count representable in microseconds, got {count} {unit}"),
        )
    })
}

/// Append one array: header, count, `n + 1` offsets, then the field bytes,
/// at the smallest count and offset widths that hold them.
fn write_array(
    target: &mut Vec<u8>,
    offsets: &[usize],
    fields: &[u8],
    path: &Path<'_>,
) -> Result<()> {
    let count = container_count(offsets.len() - 1, path)?;
    let total = u32::try_from(fields.len()).map_err(|_| {
        invalid(
            path,
            format_smolstr!(
                "expected at most {} element bytes, got {}",
                u32::MAX,
                fields.len()
            ),
        )
    })?;
    let offset_size = byte_width(total);
    let offset_size_minus_one =
        u8::try_from(offset_size - 1).expect("an offset width is between one and four bytes");
    let is_large = count > u32::from(u8::MAX);
    target.push((u8::from(is_large) << 2 | offset_size_minus_one) << 2 | BASIC_ARRAY);
    push_count(target, count, is_large);
    for offset in offsets {
        let offset = u32::try_from(*offset).expect("the total length bounded every offset");
        push_unsigned(target, offset, offset_size);
    }
    target.extend_from_slice(fields);
    Ok(())
}

/// Append one object over name-sorted entries: header, count, field ids,
/// `n + 1` offsets, then the field bytes.
fn write_object(
    target: &mut Vec<u8>,
    ordered: &[(&str, &Value)],
    dictionary: &Dictionary,
    path: &Path<'_>,
    depth: usize,
) -> Result<()> {
    let mut fields = Vec::new();
    let mut offsets = Vec::with_capacity(ordered.len() + 1);
    let mut ids = Vec::with_capacity(ordered.len());
    for (name, child) in ordered {
        ids.push(dictionary.id(name));
        offsets.push(fields.len());
        let child_path = path.field(name);
        write_value(&mut fields, child, dictionary, &child_path, depth + 1)?;
    }
    offsets.push(fields.len());
    let count = container_count(ordered.len(), path)?;
    let total = u32::try_from(fields.len()).map_err(|_| {
        invalid(
            path,
            format_smolstr!(
                "expected at most {} field bytes, got {}",
                u32::MAX,
                fields.len()
            ),
        )
    })?;
    let offset_size = byte_width(total);
    let id_size = byte_width(ids.iter().copied().max().unwrap_or(0));
    let offset_size_minus_one =
        u8::try_from(offset_size - 1).expect("an offset width is between one and four bytes");
    let id_size_minus_one =
        u8::try_from(id_size - 1).expect("a field-id width is between one and four bytes");
    let is_large = count > u32::from(u8::MAX);
    target.push(
        (u8::from(is_large) << 4 | id_size_minus_one << 2 | offset_size_minus_one) << 2
            | BASIC_OBJECT,
    );
    push_count(target, count, is_large);
    for id in &ids {
        push_unsigned(target, *id, id_size);
    }
    for offset in &offsets {
        let offset = u32::try_from(*offset).expect("the total length bounded every offset");
        push_unsigned(target, offset, offset_size);
    }
    target.extend_from_slice(&fields);
    Ok(())
}

/// Bound one container's element count to what the format can spell.
fn container_count(count: usize, path: &Path<'_>) -> Result<u32> {
    u32::try_from(count).map_err(|_| {
        invalid(
            path,
            format_smolstr!("expected at most {} elements, got {count}", u32::MAX),
        )
    })
}

/// Append `num_elements` at the width `is_large` names: one byte, or four.
fn push_count(target: &mut Vec<u8>, count: u32, is_large: bool) {
    if is_large {
        target.extend_from_slice(&count.to_le_bytes());
    } else {
        target.push(u8::try_from(count).expect("a small container counts below 256"));
    }
}
