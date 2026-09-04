//! Schema-directed validation and canonicalization of row values.
//!
//! A struct [`Field`] is the schema of the rows it describes, so validating a
//! row is validating one [`Scalar::Sequence`] against that field's children.
//! Canonicalization is the same walk with rewriting: it narrows integers,
//! floats, and nested containers into the exact representation the schema
//! declares, and returns the input untouched when nothing needed changing.

use std::collections::HashSet;

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::datatype::{
    ascii_bytes, ascii_free_text, ascii_text, code_cell_text, guid_bytes, guid_parse, guid_text,
    value_is_logically_null,
};
use crate::{DataType, Error, Field, Fields, Result, Scalar, TemporalFamily, TimeUnit, Timezone};

/// One failing value, with the path walked to reach it.
#[derive(Debug)]
struct ValidationFailure {
    path: Vec<PathSegment>,
    reason: SmolStr,
}

#[derive(Debug)]
enum PathSegment {
    Field(SmolStr),
    Index(usize),
    MapKey(usize),
    MapValue(usize),
    Union(i8),
}

impl ValidationFailure {
    fn new(reason: impl Into<SmolStr>) -> Self {
        Self {
            path: Vec::new(),
            reason: reason.into(),
        }
    }

    fn prepend(mut self, segment: PathSegment) -> Self {
        self.path.insert(0, segment);
        self
    }
}

/// Validate one row value against a struct root field.
pub(crate) fn validate_row(root: &Field, value: &Scalar) -> Result<()> {
    let expected = root.field_len();
    if let Some(record) = value.as_record() {
        validate_record_fields(root.fields(), record, 0)
            .map_err(|failure| validation_error(root.name(), failure))?;
        return Ok(());
    }
    let values = value.as_sequence().ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new(root.name()),
        reason: format_smolstr!(
            "expected a record or {expected} ordered values, got {}",
            value.kind()
        ),
    })?;
    if values.len() != expected {
        return Err(Error::InvalidRecord {
            path: SmolStr::new(root.name()),
            reason: format_smolstr!(
                "expected {expected} values for {expected} fields, got {}",
                values.len()
            ),
        });
    }
    for (field, value) in root.fields().iter().zip(values) {
        if let Err(failure) = validate_field_value(field, value) {
            return Err(validation_error(root.name(), failure));
        }
    }
    Ok(())
}

/// Validate one value against the datatype it claims, outside any row.
///
/// A [`crate::TypedScalar`] is one value and one datatype with no field around
/// them, so it validates through the same walk a column value takes and
/// reports the same failures, rooted at the value itself. A null is accepted
/// by every datatype, because nullability belongs to the field that holds the
/// column rather than to the value in it.
pub(crate) fn validate_dtype_value_for(dtype: &DataType, value: &Scalar) -> Result<()> {
    if matches!(value, Scalar::Null) {
        return Ok(());
    }
    validate_dtype_value(dtype, value, 0).map_err(|failure| {
        let mut path = String::from("$");
        for segment in failure.path {
            push_path_segment(&mut path, segment);
        }
        Error::InvalidRecord {
            path: SmolStr::from(path),
            reason: failure.reason,
        }
    })
}

/// Rewrite one row value into the exact representation a root field declares.
pub(crate) fn canonicalize_row(root: &Field, value: Scalar) -> Result<Scalar> {
    if let Some(record) = value.as_record() {
        let values = record_values(root.fields(), record)?;
        return canonicalize_row(root, Scalar::from_sequence(values));
    }
    let Some(values) = value.as_sequence() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::from(root_path(root.name())),
            reason: format_smolstr!(
                "expected an ordered sequence of column values, got {}",
                value.kind()
            ),
        });
    };
    let fields = root.fields();
    let canonical = canonicalize_slice(values, |index, value| {
        canonicalize_field_value(&fields[index], value)
    })
    .map_err(|error| {
        prepend_canonical_error(error, PathSegment::Field(SmolStr::new(root.name())))
    })?;
    if let Some(canonical) = canonical {
        Ok(Scalar::from_sequence(canonical))
    } else {
        Ok(value)
    }
}

/// Render the `$`-rooted path of a schema root.
fn root_path(name: &str) -> String {
    let mut path = String::from("$");
    crate::path::push_field_name(&mut path, name);
    path
}

fn canonicalize_field_value(field: &Field, value: &Scalar) -> Result<(Scalar, bool)> {
    canonicalize_field_payload(field, value).map_err(|error| {
        prepend_canonical_error(error, PathSegment::Field(SmolStr::new(field.name())))
    })
}

fn canonicalize_field_payload(field: &Field, value: &Scalar) -> Result<(Scalar, bool)> {
    if matches!(value, Scalar::Null) {
        return Ok((Scalar::Null, false));
    }
    canonicalize_dtype_value(field.dtype(), value)
}

/// The physical count a self-describing value carries for one column.
///
/// A decimal and a temporal each remember the scale or unit they were built
/// with, so a column declaring another scale or unit restates them, and only
/// when the restatement is exact. Every other value already *is* the physical
/// count the column stores and answers `None`, as does a restatement that would
/// have dropped a digit - which then fails the ordinary check below, naming the
/// kind that did not fit.
fn restated(dtype: &DataType, value: &Scalar) -> Option<i128> {
    use DataType as D;
    match dtype {
        D::Decimal32 { scale, .. } | D::Decimal64 { scale, .. } | D::Decimal128 { scale, .. }
            if value.is_decimal() =>
        {
            value.decimal_unscaled_at(*scale)
        }
        D::Timestamp(unit, zone)
            if temporal_matches(value, TemporalFamily::DateTime, zone.as_ref()) =>
        {
            value.temporal_count_at(*unit).map(i128::from)
        }
        D::Duration32(unit) | D::Duration64(unit)
            if temporal_matches(value, TemporalFamily::Duration, None) =>
        {
            value.temporal_count_at(*unit).map(i128::from)
        }
        D::Time32(unit) | D::Time64(unit)
            if temporal_matches(value, TemporalFamily::Time, None) =>
        {
            value.temporal_count_at(*unit).map(i128::from)
        }
        D::Date32 if temporal_matches(value, TemporalFamily::Date, None) => {
            value.temporal_count_at(TimeUnit::Day).map(i128::from)
        }
        D::Date64 if temporal_matches(value, TemporalFamily::Date, None) => value
            .temporal_count_at(TimeUnit::Millisecond)
            .map(i128::from),
        _ => None,
    }
}

/// Check the logical temporal family and the zone a datatype can preserve.
fn temporal_matches(
    value: &Scalar,
    family: TemporalFamily,
    expected_zone: Option<&Timezone>,
) -> bool {
    let Some(temporal) = value.as_temporal() else {
        return false;
    };
    if temporal.family() != family {
        return false;
    }
    let zone = temporal.timezone();
    match (family, expected_zone) {
        (TemporalFamily::DateTime, Some(expected)) => zone == expected,
        (TemporalFamily::DateTime, None) => zone.is_naive(),
        _ => zone.is_naive(),
    }
}

#[allow(clippy::too_many_lines)]
fn canonicalize_dtype_value(dtype: &DataType, value: &Scalar) -> Result<(Scalar, bool)> {
    use DataType as D;
    match dtype {
        D::Decimal32 { scale, .. } | D::Decimal64 { scale, .. } | D::Decimal128 { scale, .. } => {
            let coefficient = if value.is_decimal() {
                value.decimal_unscaled_at(*scale)
            } else {
                value.as_i128()
            }
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!("expected a d128 representable at scale {scale}"),
            })?;
            let canonical = Scalar::d128(coefficient, *scale);
            return Ok((canonical.clone(), value != &canonical));
        }
        D::Decimal256 { scale, .. } => {
            let coefficient = if value.is_decimal() {
                value.decimal256_unscaled_at(*scale)
            } else {
                value.as_i128().map(crate::I256::from_i128)
            }
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!("expected a d256 representable at scale {scale}"),
            })?;
            let canonical = Scalar::d256(coefficient, *scale);
            return Ok((canonical.clone(), value != &canonical));
        }
        D::Date32 => {
            let count = temporal_or_integer(value, TimeUnit::Day, TemporalFamily::Date, None)?;
            let canonical = Scalar::date32(
                i32::try_from(count)
                    .map_err(|_| canonical_error("date32 count does not fit signed 32 bits"))?,
            );
            return Ok((canonical.clone(), value != &canonical));
        }
        D::Date64 => {
            let count =
                temporal_or_integer(value, TimeUnit::Millisecond, TemporalFamily::Date, None)?;
            let canonical = Scalar::date64(count);
            return Ok((canonical.clone(), value != &canonical));
        }
        D::Time32(unit) => {
            let count = temporal_or_integer(value, *unit, TemporalFamily::Time, None)?;
            let canonical = Scalar::time32(
                i32::try_from(count)
                    .map_err(|_| canonical_error("time32 count does not fit signed 32 bits"))?,
                *unit,
                Timezone::NAIVE,
            )?;
            return Ok((canonical.clone(), value != &canonical));
        }
        D::Time64(unit) => {
            let count = temporal_or_integer(value, *unit, TemporalFamily::Time, None)?;
            let canonical = Scalar::time64(count, *unit, Timezone::NAIVE)?;
            return Ok((canonical.clone(), value != &canonical));
        }
        D::Timestamp(unit, zone) => {
            let count = temporal_or_integer(value, *unit, TemporalFamily::DateTime, zone.as_ref())?;
            let canonical =
                Scalar::datetime64(count, *unit, zone.clone().unwrap_or(Timezone::NAIVE))?;
            return Ok((canonical.clone(), value != &canonical));
        }
        D::Duration32(unit) => {
            let count = temporal_or_integer(value, *unit, TemporalFamily::Duration, None)?;
            let canonical = Scalar::duration32(
                i32::try_from(count)
                    .map_err(|_| canonical_error("duration32 count does not fit signed 32 bits"))?,
                *unit,
            )?;
            return Ok((canonical.clone(), value != &canonical));
        }
        D::Duration64(unit) => {
            let count = temporal_or_integer(value, *unit, TemporalFamily::Duration, None)?;
            let canonical = Scalar::duration64(count, *unit)?;
            return Ok((canonical.clone(), value != &canonical));
        }
        _ => {}
    }
    if let Some(physical) = restated(dtype, value) {
        // A restatement always rewrote something, so it is always a change.
        let (canonical, _) = canonicalize_dtype_value(dtype, &Scalar::I128(physical))?;
        return Ok((canonical, true));
    }
    match dtype {
        D::Null | D::Boolean => Ok((value.clone(), false)),
        D::Int8 | D::Int16 | D::Int32 | D::Int64 => canonical_signed(dtype, value),
        // Interval tuples use the core's signed 64-bit component spelling;
        // they are not one of the physical integer widths selected above.
        D::Interval(TimeUnit::YearMonth) => canonical_signed(&D::Int64, value),
        D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => canonical_unsigned(dtype, value),
        D::Float16 => canonical_float(value, FloatWidth::Float16),
        D::Float32 => canonical_float(value, FloatWidth::Float32),
        D::Float64 => canonical_float(value, FloatWidth::Float64),
        D::Interval(TimeUnit::DayTime) => canonical_integer_sequence(value, 2),
        D::Interval(TimeUnit::MonthDayNano) => canonical_integer_sequence(value, 3),
        D::Interval(_) => Ok((value.clone(), false)),
        D::Binary
        | D::FixedSizeBinary(_)
        | D::LargeBinary
        | D::BinaryView
        | D::Utf8
        | D::LargeUtf8
        | D::Utf8View => Ok((value.clone(), false)),
        // The canonical ASCII spelling is the trimmed string; bytes and a
        // string carrying trailing NULs are rewritten here.
        D::Ascii | D::FixedAscii(_) => match ascii_bytes(value) {
            Some(bytes) => {
                let text = match dtype.ascii_width() {
                    Some(width) => ascii_text(width, bytes)?,
                    None => ascii_free_text(bytes)?,
                };
                if matches!(value, Scalar::String(current) if current == text) {
                    Ok((value.clone(), false))
                } else {
                    Ok((Scalar::from(text), true))
                }
            }
            None => canonicalization_failure(dtype),
        },
        // A code canonicalizes the same way, at the width its own type fixes.
        D::Country | D::Currency | D::Mic | D::Cfi => match ascii_bytes(value) {
            Some(bytes) => {
                let text = code_cell_text(dtype, bytes)?;
                if matches!(value, Scalar::String(current) if current == text) {
                    Ok((value.clone(), false))
                } else {
                    Ok((Scalar::from(text), true))
                }
            }
            None => canonicalization_failure(dtype),
        },
        // The canonical GUID spelling is the hyphenated text; the sixteen
        // stored bytes and the bare-hex spelling are rewritten here.
        D::Guid => match guid_bytes(value) {
            Some(bytes) => {
                let text = guid_text(&guid_parse(bytes)?);
                if matches!(value, Scalar::String(current) if *current == text) {
                    Ok((value.clone(), false))
                } else {
                    Ok((Scalar::String(text), true))
                }
            }
            None => canonicalization_failure(dtype),
        },
        D::List(field)
        | D::ListView(field)
        | D::FixedSizeList(field, _)
        | D::LargeList(field)
        | D::LargeListView(field) => {
            canonical_sequence(value, |value| canonicalize_field_value(field, value))
        }
        D::Struct(fields) => canonical_struct(fields, value),
        D::Union(fields, _) => canonical_union(fields, value),
        D::Dictionary(dictionary) => canonicalize_dtype_value(dictionary.value(), value),
        D::Decimal32 { .. }
        | D::Decimal64 { .. }
        | D::Decimal128 { .. }
        | D::Decimal256 { .. }
        | D::Timestamp(..)
        | D::Date32
        | D::Date64
        | D::Time32(_)
        | D::Time64(_)
        | D::Duration32(_)
        | D::Duration64(_) => unreachable!("typed scalars returned above"),
        D::Map(map) => canonical_map(map, value),
        D::RunEndEncoded(encoded) => canonicalize_field_value(encoded.values(), value),
        // A variant value is any value: the tree describes itself.
        D::Variant => Ok((value.clone(), false)),
        // The canonical geospatial spelling is `Scalar::Geospatial`; plain
        // bytes are accepted on the way in and rewritten here.
        D::Geometry(_) | D::Geography(_) => match value {
            Scalar::Geospatial(_) => Ok((value.clone(), false)),
            Scalar::Bytes(bytes) => Ok((Scalar::Geospatial(Arc::from(bytes.as_ref())), true)),
            other => Ok((other.clone(), false)),
        },
    }
}

fn canonical_signed(dtype: &DataType, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(integer) = value.as_i128() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            // Naming the kind is what tells a caller that the temporal they
            // wrote was the wrong resolution rather than the wrong shape.
            reason: format_smolstr!(
                "validated signed value could not be canonicalized from {}",
                value.kind()
            ),
        });
    };
    let canonical = match dtype {
        DataType::Int8 => Scalar::I8(i8::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int16 => Scalar::I16(i16::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int32 => Scalar::I32(i32::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int64 | DataType::Interval(TimeUnit::YearMonth) => {
            Scalar::I64(i64::try_from(integer).map_err(canonical_integer_error)?)
        }
        _ => unreachable!("signed canonicalization requires a signed datatype"),
    };
    let changed = match &canonical {
        Scalar::I8(expected) => !matches!(value, Scalar::I8(current) if current == expected),
        Scalar::I16(expected) => !matches!(value, Scalar::I16(current) if current == expected),
        Scalar::I32(expected) => !matches!(value, Scalar::I32(current) if current == expected),
        Scalar::I64(expected) => !matches!(value, Scalar::I64(current) if current == expected),
        _ => unreachable!("signed canonical value has a signed kind"),
    };
    Ok((canonical, changed))
}

fn canonical_unsigned(dtype: &DataType, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(integer) = value.as_u128() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated unsigned value could not be canonicalized"),
        });
    };
    let canonical = match dtype {
        DataType::UInt8 => Scalar::U8(u8::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt16 => Scalar::U16(u16::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt32 => Scalar::U32(u32::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt64 => Scalar::U64(u64::try_from(integer).map_err(canonical_integer_error)?),
        _ => unreachable!("unsigned canonicalization requires an unsigned datatype"),
    };
    let changed = match &canonical {
        Scalar::U8(expected) => !matches!(value, Scalar::U8(current) if current == expected),
        Scalar::U16(expected) => !matches!(value, Scalar::U16(current) if current == expected),
        Scalar::U32(expected) => !matches!(value, Scalar::U32(current) if current == expected),
        Scalar::U64(expected) => !matches!(value, Scalar::U64(current) if current == expected),
        _ => unreachable!("unsigned canonical value has an unsigned kind"),
    };
    Ok((canonical, changed))
}

fn canonical_integer_error(_error: impl std::fmt::Display) -> Error {
    canonical_error("integer does not fit declared width")
}

enum FloatWidth {
    Float16,
    Float32,
    Float64,
}

fn canonical_float(value: &Scalar, width: FloatWidth) -> Result<(Scalar, bool)> {
    let Some(number) = value.as_f64() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated float value could not be canonicalized"),
        });
    };
    let canonical = match width {
        FloatWidth::Float16 => Scalar::from(half::f16::from_f64(number)),
        FloatWidth::Float32 => Scalar::from(number as f32),
        FloatWidth::Float64 => Scalar::from(number),
    };
    let changed = value != &canonical;
    Ok((canonical, changed))
}

fn temporal_or_integer(
    value: &Scalar,
    unit: TimeUnit,
    family: TemporalFamily,
    zone: Option<&Timezone>,
) -> Result<i64> {
    if value.is_temporal() {
        if !temporal_matches(value, family, zone) {
            return Err(canonical_error(
                "temporal family or timezone does not match the declared datatype",
            ));
        }
        return value.temporal_count_at(unit).ok_or_else(|| {
            canonical_error("temporal count cannot be represented in the declared unit")
        });
    }
    value
        .as_i128()
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| canonical_error("expected a signed 64-bit temporal count"))
}

fn canonical_error(reason: &'static str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: reason.into(),
    }
}

fn canonical_integer_sequence(value: &Scalar, length: usize) -> Result<(Scalar, bool)> {
    let Some(values) = value.as_sequence() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated integer tuple could not be canonicalized"),
        });
    };
    if values.len() != length {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated integer tuple changed length"),
        });
    }
    canonical_sequence(value, |value| canonical_signed(&DataType::Int64, value))
}

fn canonical_sequence(
    value: &Scalar,
    mut canonicalize: impl FnMut(&Scalar) -> Result<(Scalar, bool)>,
) -> Result<(Scalar, bool)> {
    let Some(values) = value.as_sequence() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated sequence could not be canonicalized"),
        });
    };
    if let Some(canonical) = canonicalize_slice(values, |index, value| {
        canonicalize(value)
            .map_err(|error| prepend_canonical_error(error, PathSegment::Index(index)))
    })? {
        Ok((Scalar::from_sequence(canonical), true))
    } else {
        Ok((value.clone(), false))
    }
}

fn canonical_struct(fields: &Fields, value: &Scalar) -> Result<(Scalar, bool)> {
    if let Some(record) = value.as_record() {
        let values = record_values(fields, record)?;
        let sequence = Scalar::from_sequence(values);
        return canonical_struct(fields, &sequence).map(|(value, _)| (value, true));
    }
    let Some(values) = value.as_sequence() else {
        return canonicalization_failure(&DataType::Struct(fields.clone()));
    };
    if let Some(canonical) = canonicalize_slice(values, |index, value| {
        canonicalize_field_value(&fields[index], value)
    })? {
        Ok((Scalar::from_sequence(canonical), true))
    } else {
        Ok((value.clone(), false))
    }
}

fn canonical_union(fields: &crate::UnionFields, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some([type_id, payload]) = value.as_sequence() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated union could not be canonicalized"),
        });
    };
    let Some(type_id_number) = type_id.as_i128().and_then(|value| i8::try_from(value).ok()) else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated union type id could not be canonicalized"),
        });
    };
    let Some((_, field)) = fields
        .iter()
        .find(|(candidate, _)| *candidate == type_id_number)
    else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated union branch could not be canonicalized"),
        });
    };
    let (payload, payload_changed) = canonicalize_field_value(field, payload)
        .map_err(|error| prepend_canonical_error(error, PathSegment::Union(type_id_number)))?;
    let id_changed =
        !matches!(type_id, Scalar::I64(current) if *current == i64::from(type_id_number));
    if id_changed || payload_changed {
        Ok((
            Scalar::from_sequence([Scalar::I64(i64::from(type_id_number)), payload]),
            true,
        ))
    } else {
        Ok((value.clone(), false))
    }
}

fn canonical_map(map: &crate::MapType, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(entries) = value.as_mapping() else {
        return canonicalization_failure(&DataType::Map(map.clone().into()));
    };
    let Some([key_field, value_field]) = map.entries().dtype().as_fields() else {
        return canonicalization_failure(&DataType::Map(map.clone().into()));
    };
    for (index, (key, entry_value)) in entries.iter().enumerate() {
        let (canonical_key, key_changed) = canonicalize_field_payload(key_field, key)
            .map_err(|error| prepend_canonical_error(error, PathSegment::MapKey(index)))?;
        let (canonical_value, value_changed) = canonicalize_field_payload(value_field, entry_value)
            .map_err(|error| prepend_canonical_error(error, PathSegment::MapValue(index)))?;
        if key_changed || value_changed {
            let mut canonical = Vec::with_capacity(entries.len());
            canonical.extend_from_slice(&entries[..index]);
            canonical.push((canonical_key, canonical_value));
            for (offset, (key, entry_value)) in entries[index + 1..].iter().enumerate() {
                let entry_index = index + 1 + offset;
                canonical.push((
                    canonicalize_field_payload(key_field, key)
                        .map_err(|error| {
                            prepend_canonical_error(error, PathSegment::MapKey(entry_index))
                        })?
                        .0,
                    canonicalize_field_payload(value_field, entry_value)
                        .map_err(|error| {
                            prepend_canonical_error(error, PathSegment::MapValue(entry_index))
                        })?
                        .0,
                ));
            }
            if let Some(duplicate) = duplicate_mapping_key_index(&canonical) {
                return Err(Error::InvalidRecord {
                    path: SmolStr::from(format!("$[{duplicate}].key")),
                    reason: SmolStr::new_static(
                        "map keys collide after schema-directed physical normalization",
                    ),
                });
            }
            let canonical = Scalar::from_mapping(canonical)?;
            let canonical_entries = canonical.as_mapping().unwrap_or_default();
            if map.keys_sorted() {
                if let Some(index) = canonical_entries
                    .windows(2)
                    .position(|pair| pair[0].0 > pair[1].0)
                    .map(|index| index + 1)
                {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::from(format!("$[{index}].key")),
                        reason: SmolStr::new_static(
                            "map keys are not sorted after schema-directed physical normalization",
                        ),
                    });
                }
            }
            return Ok((canonical, true));
        }
    }
    Ok((value.clone(), false))
}

fn duplicate_mapping_key_index(entries: &[(Scalar, Scalar)]) -> Option<usize> {
    if entries.len() <= 16 {
        return (1..entries.len()).find(|index| {
            entries[..*index]
                .iter()
                .any(|(key, _)| key == &entries[*index].0)
        });
    }
    // `Scalar`'s hash reads canonical content only, never the
    // interior-mutable caches a datatype holds, so the key is stable.
    #[allow(clippy::mutable_key_type)]
    let mut seen = HashSet::with_capacity(entries.len());
    entries
        .iter()
        .enumerate()
        .find_map(|(index, (key, _))| (!seen.insert(key)).then_some(index))
}

fn canonicalize_slice(
    values: &[Scalar],
    mut canonicalize: impl FnMut(usize, &Scalar) -> Result<(Scalar, bool)>,
) -> Result<Option<Vec<Scalar>>> {
    for (index, value) in values.iter().enumerate() {
        let (canonical_value, changed) = canonicalize(index, value)?;
        if changed {
            let mut canonical = Vec::with_capacity(values.len());
            canonical.extend_from_slice(&values[..index]);
            canonical.push(canonical_value);
            for (remaining_index, value) in values[index + 1..].iter().enumerate() {
                canonical.push(canonicalize(index + 1 + remaining_index, value)?.0);
            }
            return Ok(Some(canonical));
        }
    }
    Ok(None)
}

fn canonicalization_failure<T>(dtype: &DataType) -> Result<T> {
    Err(Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!(
            "validated {} value could not be canonicalized",
            dtype.name()
        ),
    })
}

fn prepend_canonical_error(error: Error, segment: PathSegment) -> Error {
    let Error::InvalidRecord { path, reason } = error else {
        return error;
    };
    let mut prefixed = String::from("$");
    push_path_segment(&mut prefixed, segment);
    prefixed.push_str(path.strip_prefix('$').unwrap_or(path.as_str()));
    Error::InvalidRecord {
        path: SmolStr::from(prefixed),
        reason,
    }
}

fn validation_error(root: &str, failure: ValidationFailure) -> Error {
    let mut path = root_path(root);
    for segment in failure.path {
        push_path_segment(&mut path, segment);
    }
    Error::InvalidRecord {
        path: SmolStr::from(path),
        reason: failure.reason,
    }
}

fn push_path_segment(path: &mut String, segment: PathSegment) {
    // Owned segments are accumulated while a validation failure unwinds, so
    // this cannot borrow `crate::path::Path`; it shares the spelling instead.
    use crate::path::{Segment, push_segment};
    match segment {
        PathSegment::Field(name) => push_segment(path, Segment::Field(&name)),
        PathSegment::Index(index) => push_segment(path, Segment::Index(index)),
        PathSegment::MapKey(index) => push_segment(path, Segment::MapKey(index)),
        PathSegment::MapValue(index) => push_segment(path, Segment::MapValue(index)),
        PathSegment::Union(type_id) => push_segment(path, Segment::UnionType(type_id)),
    }
}

fn validate_field_value(
    field: &Field,
    value: &Scalar,
) -> std::result::Result<(), ValidationFailure> {
    validate_field_value_at_depth(field, value, 0)
}

fn validate_field_value_at_depth(
    field: &Field,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    validate_field_payload_at_depth(field, value, depth)
        .map_err(|failure| failure.prepend(PathSegment::Field(SmolStr::new(field.name()))))
}

fn validate_field_payload_at_depth(
    field: &Field,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    if depth >= DataType::PARSE_RECURSION_LIMIT {
        return Err(ValidationFailure::new(format_smolstr!(
            "record nesting exceeds the hard limit of {}",
            DataType::PARSE_RECURSION_LIMIT
        )));
    }
    if value_is_logically_null(field.dtype(), value) && !field.is_nullable() {
        return Err(ValidationFailure::new("non-nullable field received null"));
    }
    if matches!(value, Scalar::Null)
        && !matches!(
            field.dtype(),
            DataType::Union(..) | DataType::RunEndEncoded(_)
        )
    {
        return Ok(());
    }
    validate_dtype_value(field.dtype(), value, depth)
}

#[allow(clippy::too_many_lines)]
fn validate_dtype_value(
    dtype: &DataType,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    use DataType as D;
    if let Some(physical) = restated(dtype, value) {
        return validate_dtype_value(dtype, &Scalar::I128(physical), depth);
    }
    match dtype {
        D::Null => Err(expected("null", value)),
        D::Boolean => require(matches!(value, Scalar::Bool(_)), "boolean", value),
        D::Int8 => validate_signed(value, i128::from(i8::MIN), i128::from(i8::MAX), "int8"),
        D::Int16 => validate_signed(value, i128::from(i16::MIN), i128::from(i16::MAX), "int16"),
        D::Int32 => validate_signed(value, i128::from(i32::MIN), i128::from(i32::MAX), "int32"),
        D::Int64 => validate_signed(value, i128::from(i64::MIN), i128::from(i64::MAX), "int64"),
        D::UInt8 => validate_unsigned(value, u128::from(u8::MAX), "uint8"),
        D::UInt16 => validate_unsigned(value, u128::from(u16::MAX), "uint16"),
        D::UInt32 => validate_unsigned(value, u128::from(u32::MAX), "uint32"),
        D::UInt64 => validate_unsigned(value, u128::from(u64::MAX), "uint64"),
        D::Float16 | D::Float32 | D::Float64 => {
            require(value.as_f64().is_some(), dtype.name(), value)
        }
        D::Timestamp(..) | D::Duration64(_) => validate_signed(
            value,
            i128::from(i64::MIN),
            i128::from(i64::MAX),
            dtype.name(),
        ),
        D::Duration32(_) => validate_signed(
            value,
            i128::from(i32::MIN),
            i128::from(i32::MAX),
            dtype.name(),
        ),
        D::Date32 => validate_signed(
            value,
            i128::from(i32::MIN),
            i128::from(i32::MAX),
            dtype.name(),
        ),
        D::Date64 => validate_date64(value),
        D::Time32(unit) | D::Time64(unit) => validate_time(value, *unit),
        D::Interval(TimeUnit::YearMonth) => validate_signed(
            value,
            i128::from(i32::MIN),
            i128::from(i32::MAX),
            "interval year_month",
        ),
        D::Interval(TimeUnit::DayTime) => {
            validate_integer_tuple(value, &[32, 32], "interval day_time")
        }
        D::Interval(TimeUnit::MonthDayNano) => {
            validate_integer_tuple(value, &[32, 32, 64], "interval month_day_nano")
        }
        D::Interval(_) => Err(ValidationFailure::new("invalid interval layout")),
        D::Binary | D::LargeBinary | D::BinaryView => {
            require(matches!(value, Scalar::Bytes(_)), dtype.name(), value)
        }
        D::FixedSizeBinary(width) => match value.as_bytes() {
            Some(bytes) if usize::try_from(*width).ok() == Some(bytes.len()) => Ok(()),
            Some(bytes) => Err(ValidationFailure::new(format_smolstr!(
                "fixed_size_binary({width}) requires {width} bytes, got {}",
                bytes.len()
            ))),
            None => Err(expected("fixed-size binary bytes", value)),
        },
        D::Utf8 | D::LargeUtf8 | D::Utf8View => {
            require(matches!(value, Scalar::String(_)), dtype.name(), value)
        }
        // Text or bytes, both under the one ASCII rule naming the width.
        D::Ascii | D::FixedAscii(_) => match ascii_bytes(value) {
            Some(bytes) => match dtype.ascii_width() {
                Some(width) => ascii_text(width, bytes).map(|_| ()).map_err(ascii_failure),
                None => ascii_free_text(bytes).map(|_| ()).map_err(ascii_failure),
            },
            None => Err(expected(dtype.name(), value)),
        },
        D::Country | D::Currency | D::Mic | D::Cfi => match ascii_bytes(value) {
            Some(bytes) => code_cell_text(dtype, bytes)
                .map(|_| ())
                .map_err(ascii_failure),
            None => Err(expected(dtype.name(), value)),
        },
        D::Guid => match guid_bytes(value).map(guid_parse) {
            Some(Ok(_)) => Ok(()),
            _ => Err(expected("guid", value)),
        },
        D::List(field) | D::ListView(field) | D::LargeList(field) | D::LargeListView(field) => {
            validate_sequence(field, value, None, dtype.name(), depth + 1)
        }
        D::FixedSizeList(field, size) => validate_sequence(
            field,
            value,
            usize::try_from(*size).ok(),
            "fixed_size_list",
            depth + 1,
        ),
        D::Struct(fields) => validate_struct(fields, value, depth + 1),
        D::Union(fields, _) => validate_union(fields, value, depth + 1),
        D::Dictionary(dictionary) => validate_dtype_value(dictionary.value(), value, depth + 1),
        D::Decimal32 { precision, .. } => validate_decimal(value, *precision, 32),
        D::Decimal64 { precision, .. } => validate_decimal(value, *precision, 64),
        D::Decimal128 { precision, .. } => validate_decimal(value, *precision, 128),
        D::Decimal256 { precision, scale } => validate_decimal256(value, *precision, *scale),
        D::Map(map) => validate_map(map, value, depth + 1),
        D::RunEndEncoded(encoded) => {
            validate_field_value_at_depth(encoded.values(), value, depth + 1)
        }
        // A variant column validates any value, null included: the variant
        // null is a *value* the encoding can spell, so a required variant
        // holding `Null` is present, not absent.
        D::Variant => Ok(()),
        // A geospatial value is Well-Known Binary, and the payload's own
        // framing is the validation: a buffer the WKB reader refuses is not a
        // geometry, whatever bytes it carries.
        D::Geometry(_) | D::Geography(_) => match value.as_wkb() {
            Some(bytes) => crate::generic::wkb::Geometry::from_slice(bytes)
                .map(|_| ())
                .map_err(|error| expected_because(dtype.name(), value, &error)),
            None => Err(expected(dtype.name(), value)),
        },
    }
}

fn validate_signed(
    value: &Scalar,
    minimum: i128,
    maximum: i128,
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    match value.as_i128() {
        Some(value) if (minimum..=maximum).contains(&value) => Ok(()),
        _ => Err(expected(expected_name, value)),
    }
}

fn validate_unsigned(
    value: &Scalar,
    maximum: u128,
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    match value.as_u128() {
        Some(value) if value <= maximum => Ok(()),
        _ => Err(expected(expected_name, value)),
    }
}

fn validate_date64(value: &Scalar) -> std::result::Result<(), ValidationFailure> {
    const MILLIS_PER_DAY: i128 = 86_400_000;
    let Some(number) = value.as_i128() else {
        return Err(expected("date64 whole-day milliseconds", value));
    };
    if i64::try_from(number).is_err() || number % MILLIS_PER_DAY != 0 {
        return Err(ValidationFailure::new(
            "date64 must be signed 64-bit whole-day milliseconds",
        ));
    }
    Ok(())
}

fn validate_time(value: &Scalar, unit: TimeUnit) -> std::result::Result<(), ValidationFailure> {
    let maximum = match unit {
        TimeUnit::Second => 86_400_i128,
        TimeUnit::Millisecond => 86_400_000_i128,
        TimeUnit::Microsecond => 86_400_000_000_i128,
        TimeUnit::Nanosecond => 86_400_000_000_000_i128,
        _ => return Err(ValidationFailure::new("invalid time-of-day unit")),
    };
    let Some(number) = value.as_i128() else {
        return Err(expected("time-of-day integer", value));
    };
    if !(0..maximum).contains(&number) {
        return Err(ValidationFailure::new(format_smolstr!(
            "time-of-day value must be in 0..{maximum} for {unit}"
        )));
    }
    Ok(())
}

fn validate_integer_tuple(
    value: &Scalar,
    widths: &[u8],
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    let values = value
        .as_sequence()
        .ok_or_else(|| expected(expected_name, value))?;
    if values.len() != widths.len() {
        return Err(ValidationFailure::new(format_smolstr!(
            "{expected_name} requires {} integer components, got {}",
            widths.len(),
            values.len()
        )));
    }
    for (index, (value, width)) in values.iter().zip(widths).enumerate() {
        let (minimum, maximum) = if *width == 32 {
            (i128::from(i32::MIN), i128::from(i32::MAX))
        } else {
            (i128::from(i64::MIN), i128::from(i64::MAX))
        };
        validate_signed(value, minimum, maximum, expected_name)
            .map_err(|failure| failure.prepend(PathSegment::Index(index)))?;
    }
    Ok(())
}

fn validate_sequence(
    field: &Field,
    value: &Scalar,
    expected_len: Option<usize>,
    expected_name: &str,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    let values = value
        .as_sequence()
        .ok_or_else(|| expected(expected_name, value))?;
    if let Some(expected_len) = expected_len {
        if values.len() != expected_len {
            return Err(ValidationFailure::new(format_smolstr!(
                "{expected_name} requires {expected_len} items, got {}",
                values.len()
            )));
        }
    }
    for (index, value) in values.iter().enumerate() {
        validate_field_value_at_depth(field, value, depth)
            .map_err(|failure| failure.prepend(PathSegment::Index(index)))?;
    }
    Ok(())
}

fn validate_struct(
    fields: &Fields,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    if let Some(record) = value.as_record() {
        return validate_record_fields(fields, record, depth);
    }
    let values = value
        .as_sequence()
        .ok_or_else(|| expected("struct sequence", value))?;
    if values.len() != fields.len() {
        return Err(ValidationFailure::new(format_smolstr!(
            "struct requires {} fields, got {} values",
            fields.len(),
            values.len()
        )));
    }
    fields
        .iter()
        .zip(values)
        .try_for_each(|(field, value)| validate_field_value_at_depth(field, value, depth))
}

fn validate_record_fields(
    fields: &[Field],
    record: &std::collections::BTreeMap<SmolStr, Scalar>,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    if let Some(name) = record
        .keys()
        .find(|name| !fields.iter().any(|field| field.name() == name.as_str()))
    {
        return Err(ValidationFailure::new(format_smolstr!(
            "record contains unknown field {name:?}"
        )));
    }
    for field in fields.iter() {
        if let Some(value) = record.get(field.name()) {
            validate_field_value_at_depth(field, value, depth)?;
        } else {
            let default = field
                .default_value()
                .map_err(|error| ValidationFailure::new(error.to_string()))?;
            validate_field_value_at_depth(field, &default, depth)?;
        }
    }
    Ok(())
}

fn record_values(
    fields: &[Field],
    record: &std::collections::BTreeMap<SmolStr, Scalar>,
) -> Result<Vec<Scalar>> {
    if let Some(name) = record
        .keys()
        .find(|name| !fields.iter().any(|field| field.name() == name.as_str()))
    {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("record contains unknown field {name:?}"),
        });
    }
    fields
        .iter()
        .map(|field| {
            record
                .get(field.name())
                .cloned()
                .map_or_else(|| field.default_value(), Ok)
        })
        .collect()
}

fn validate_union(
    fields: &crate::UnionFields,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    let values = value
        .as_sequence()
        .ok_or_else(|| expected("union [type_id, payload] sequence", value))?;
    let [type_id, payload] = values else {
        return Err(ValidationFailure::new(
            "union value must contain exactly [type_id, payload]",
        ));
    };
    let type_id = type_id
        .as_i128()
        .and_then(|value| i8::try_from(value).ok())
        .ok_or_else(|| ValidationFailure::new("union type_id must fit in signed int8"))?;
    let (_, field) = fields
        .iter()
        .find(|(candidate, _)| *candidate == type_id)
        .ok_or_else(|| {
            ValidationFailure::new(format_smolstr!("unknown union type id {type_id}"))
        })?;
    validate_field_value_at_depth(field, payload, depth)
        .map_err(|failure| failure.prepend(PathSegment::Union(type_id)))
}

fn validate_decimal(
    value: &Scalar,
    precision: u8,
    width: u16,
) -> std::result::Result<(), ValidationFailure> {
    let Some(integer) = value.as_i128() else {
        return Err(expected("unscaled decimal integer", value));
    };
    let fits_width = match width {
        32 => i32::try_from(integer).is_ok(),
        64 => i64::try_from(integer).is_ok(),
        _ => true,
    };
    if !fits_width || decimal_digits(integer.unsigned_abs()) > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal value exceeds precision {precision} or physical width {width}"
        )));
    }
    Ok(())
}

fn validate_decimal256(
    value: &Scalar,
    precision: u8,
    scale: i8,
) -> std::result::Result<(), ValidationFailure> {
    let Some(coefficient) = (if value.is_decimal() {
        value.decimal256_unscaled_at(scale)
    } else {
        value.as_i128().map(crate::I256::from_i128)
    }) else {
        return Err(expected("d256", value));
    };
    let encoded = coefficient.to_string();
    let digits = encoded.trim_start_matches('-');
    if digits.len() > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal256 value exceeds precision {precision}"
        )));
    }
    Ok(())
}

/// The reason an ASCII refusal carries; the walk re-roots its path.
fn ascii_failure(error: Error) -> ValidationFailure {
    ValidationFailure::new(match error {
        Error::InvalidRecord { reason, .. } => reason,
        other => format_smolstr!("{other}"),
    })
}

/// Report a value whose kind does not match what the schema declared.
fn expected(expected_name: &str, value: &Scalar) -> ValidationFailure {
    ValidationFailure::new(crate::text::expected_got(expected_name, value.kind()))
}

/// [`expected`], carrying the refusal that says why the payload failed.
fn expected_because(
    expected_name: &str,
    value: &Scalar,
    because: &crate::Error,
) -> ValidationFailure {
    ValidationFailure::new(format_smolstr!(
        "expected {expected_name}, got {}: {because}",
        value.kind()
    ))
}

/// Accept a value when `matched`, and otherwise report what was expected.
fn require(
    matched: bool,
    expected_name: &str,
    value: &Scalar,
) -> std::result::Result<(), ValidationFailure> {
    if matched {
        Ok(())
    } else {
        Err(expected(expected_name, value))
    }
}

/// Validate one map value against its key and value fields.
///
/// A map is an ordered mapping whose entries struct declares exactly a key and
/// a value field; every entry is validated against those two.
fn validate_map(
    map: &crate::MapType,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    let entries = value
        .as_mapping()
        .ok_or_else(|| expected("map entries", value))?;
    let Some([key_field, value_field]) = map.entries().dtype().as_fields() else {
        return Err(ValidationFailure::new(
            "map entries must declare exactly a key and a value field",
        ));
    };
    for (index, (key, entry_value)) in entries.iter().enumerate() {
        validate_field_value_at_depth(key_field, key, depth)
            .map_err(|failure| failure.prepend(PathSegment::MapKey(index)))?;
        validate_field_value_at_depth(value_field, entry_value, depth)
            .map_err(|failure| failure.prepend(PathSegment::MapValue(index)))?;
    }
    Ok(())
}

/// Count the base-10 digits of an unsigned decimal coefficient.
///
/// Zero has one digit, which is what a precision check expects.
fn decimal_digits(value: u128) -> usize {
    let mut digits = 1;
    let mut remaining = value / 10;
    while remaining > 0 {
        digits += 1;
        remaining /= 10;
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        DataType::from_fields(fields).unwrap().required_field("row")
    }

    #[test]
    fn a_record_maps_names_to_schema_order_and_fills_field_defaults() {
        let schema = root([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("venue"),
        ]);
        let record = Scalar::from_record([("id", Scalar::I8(7))]).unwrap();

        schema.validate_value(&record).unwrap();
        assert_eq!(
            schema.canonicalize_value(record).unwrap(),
            Scalar::from_sequence([Scalar::I64(7), Scalar::Null])
        );
    }

    #[test]
    fn a_record_refuses_unknown_names() {
        let schema = root([DataType::Int64.required_field("id")]);
        let record =
            Scalar::from_record([("id", Scalar::I64(7)), ("unknown", Scalar::I64(1))]).unwrap();

        let validation = schema.validate_value(&record).unwrap_err().to_string();
        let canonical = schema.canonicalize_value(record).unwrap_err().to_string();
        assert!(validation.contains("unknown field"), "{validation}");
        assert!(canonical.contains("unknown field"), "{canonical}");
    }

    #[test]
    fn integer_canonicalization_preserves_every_declared_width() {
        let schema = root([
            DataType::Int8.required_field("i8"),
            DataType::Int16.required_field("i16"),
            DataType::Int32.required_field("i32"),
            DataType::Int64.required_field("i64"),
            DataType::UInt8.required_field("u8"),
            DataType::UInt16.required_field("u16"),
            DataType::UInt32.required_field("u32"),
            DataType::UInt64.required_field("u64"),
        ]);
        let natural = Scalar::from_sequence([
            Scalar::I64(-1),
            Scalar::U64(2),
            Scalar::I64(-3),
            Scalar::I8(-4),
            Scalar::U64(1),
            Scalar::U64(2),
            Scalar::U64(3),
            Scalar::U8(4),
        ]);
        let canonical = schema.canonicalize_value(natural).unwrap();
        let values = canonical.as_sequence().unwrap();
        assert!(matches!(values[0], Scalar::I8(-1)));
        assert!(matches!(values[1], Scalar::I16(2)));
        assert!(matches!(values[2], Scalar::I32(-3)));
        assert!(matches!(values[3], Scalar::I64(-4)));
        assert!(matches!(values[4], Scalar::U8(1)));
        assert!(matches!(values[5], Scalar::U16(2)));
        assert!(matches!(values[6], Scalar::U32(3)));
        assert!(matches!(values[7], Scalar::U64(4)));
    }

    #[test]
    fn year_month_interval_keeps_its_signed_64_bit_component_spelling() {
        let schema = root([DataType::Interval(TimeUnit::YearMonth).required_field("months")]);

        assert_eq!(
            schema
                .canonicalize_value(Scalar::from_sequence([Scalar::I8(18)]))
                .unwrap(),
            Scalar::from_sequence([Scalar::I64(18)])
        );
    }

    #[test]
    fn temporal_casts_preserve_family_and_timezone() {
        let schema = root([
            DataType::Timestamp(TimeUnit::Millisecond, Some(Timezone::UTC)).required_field("at"),
            DataType::Time32(TimeUnit::Second).required_field("clock"),
            DataType::Duration32(TimeUnit::Millisecond).required_field("elapsed"),
        ]);
        let valid = Scalar::from_sequence([
            Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
            Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration64(3, TimeUnit::Second).unwrap(),
        ]);
        assert_eq!(
            schema.canonicalize_value(valid).unwrap(),
            Scalar::from_sequence([
                Scalar::datetime64(1_000, TimeUnit::Millisecond, Timezone::UTC).unwrap(),
                Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::duration32(3_000, TimeUnit::Millisecond).unwrap(),
            ])
        );

        for invalid in [
            Scalar::from_sequence([
                Scalar::datetime64(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
            ]),
            Scalar::from_sequence([
                Scalar::duration64(1, TimeUnit::Second).unwrap(),
                Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
            ]),
            Scalar::from_sequence([
                Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
                Scalar::Time32(2, TimeUnit::Second, Timezone::UTC),
                Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
            ]),
        ] {
            assert!(schema.validate_value(&invalid).is_err());
            assert!(schema.canonicalize_value(invalid).is_err());
        }
    }
}
