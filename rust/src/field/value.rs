//! Schema-directed validation and canonicalization of row values.
//!
//! A struct [`Field`] is the schema of the rows it describes, so validating a
//! row is validating one [`Value::Sequence`] against that field's children.
//! Canonicalization is the same walk with rewriting: it narrows integers,
//! floats, and nested containers into the exact representation the schema
//! declares, and returns the input untouched when nothing needed changing.

use std::collections::HashSet;

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::datatype::value_is_logically_null;
use crate::{DataType, Error, Field, Fields, Result, TimeUnit, Value};

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
pub(crate) fn validate_row(root: &Field, value: &Value) -> Result<()> {
    let expected = root.field_len();
    let values = value.as_sequence().ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new(root.name()),
        reason: format_smolstr!(
            "expected an ordered sequence of {expected} column values, got {}",
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
/// A [`crate::TypedValue`] is one value and one datatype with no field around
/// them, so it validates through the same walk a column value takes and
/// reports the same failures, rooted at the value itself. A null is accepted
/// by every datatype, because nullability belongs to the field that holds the
/// column rather than to the value in it.
pub(crate) fn validate_data_type_value_for(data_type: &DataType, value: &Value) -> Result<()> {
    if matches!(value, Value::Null) {
        return Ok(());
    }
    validate_data_type_value(data_type, value, 0).map_err(|failure| {
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
pub(crate) fn canonicalize_row(root: &Field, value: Value) -> Result<Value> {
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
        Ok(Value::from_sequence(canonical))
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

fn canonicalize_field_value(field: &Field, value: &Value) -> Result<(Value, bool)> {
    canonicalize_field_payload(field, value).map_err(|error| {
        prepend_canonical_error(error, PathSegment::Field(SmolStr::new(field.name())))
    })
}

fn canonicalize_field_payload(field: &Field, value: &Value) -> Result<(Value, bool)> {
    if matches!(value, Value::Null) {
        return Ok((Value::Null, false));
    }
    canonicalize_data_type_value(field.data_type(), value)
}

/// The physical count a self-describing value carries for one column.
///
/// A decimal and a temporal each remember the scale or unit they were built
/// with, so a column declaring another scale or unit restates them, and only
/// when the restatement is exact. Every other value already *is* the physical
/// count the column stores and answers `None`, as does a restatement that would
/// have dropped a digit - which then fails the ordinary check below, naming the
/// kind that did not fit.
fn restated(data_type: &DataType, value: &Value) -> Option<i128> {
    use DataType as D;
    match data_type {
        D::Decimal32 { scale, .. } | D::Decimal64 { scale, .. } | D::Decimal128 { scale, .. }
            if value.is_decimal() =>
        {
            value.decimal_unscaled_at(*scale)
        }
        D::Timestamp(unit, _) | D::Duration(unit) | D::Time32(unit) | D::Time64(unit)
            if value.is_temporal() =>
        {
            value.temporal_count_at(*unit).map(i128::from)
        }
        D::Date32 => value.as_date().map(i128::from),
        D::Date64 => value
            .as_date()
            .and_then(|days| i128::from(days).checked_mul(86_400_000)),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
fn canonicalize_data_type_value(data_type: &DataType, value: &Value) -> Result<(Value, bool)> {
    use DataType as D;
    if let Some(physical) = restated(data_type, value) {
        // A restatement always rewrote something, so it is always a change.
        let (canonical, _) = canonicalize_data_type_value(data_type, &Value::I128(physical))?;
        return Ok((canonical, true));
    }
    match data_type {
        D::Null | D::Boolean => Ok((value.clone(), false)),
        D::Int8
        | D::Int16
        | D::Int32
        | D::Int64
        | D::Timestamp(..)
        | D::Date32
        | D::Date64
        | D::Time32(_)
        | D::Time64(_)
        | D::Duration(_)
        | D::Interval(TimeUnit::YearMonth) => canonical_signed(value),
        D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => canonical_unsigned(value),
        D::Float16 => canonical_float(value, FloatWidth::Float16),
        D::Float32 => canonical_float(value, FloatWidth::Float32),
        D::Float64 => Ok((value.clone(), false)),
        D::Interval(TimeUnit::DayTime) => canonical_integer_sequence(value, 2),
        D::Interval(TimeUnit::MonthDayNano) => canonical_integer_sequence(value, 3),
        D::Interval(_) => Ok((value.clone(), false)),
        D::Binary
        | D::FixedSizeBinary(_)
        | D::LargeBinary
        | D::BinaryView
        | D::Utf8
        | D::LargeUtf8
        | D::Utf8View
        | D::Decimal256 { .. } => Ok((value.clone(), false)),
        D::List(field)
        | D::ListView(field)
        | D::FixedSizeList(field, _)
        | D::LargeList(field)
        | D::LargeListView(field) => {
            canonical_sequence(value, |value| canonicalize_field_value(field, value))
        }
        D::Struct(fields) => canonical_struct(fields, value),
        D::Union(fields, _) => canonical_union(fields, value),
        D::Dictionary(dictionary) => canonicalize_data_type_value(dictionary.value(), value),
        D::Decimal32 { .. } | D::Decimal64 { .. } | D::Decimal128 { .. } => {
            let Some(integer) = value.as_i128() else {
                return canonicalization_failure(data_type);
            };
            Ok((Value::I128(integer), !matches!(value, Value::I128(_))))
        }
        D::Map(map) => canonical_map(map, value),
        D::RunEndEncoded(encoded) => canonicalize_field_value(encoded.values(), value),
        // A variant value is any value: the tree describes itself.
        D::Variant => Ok((value.clone(), false)),
        // The canonical geospatial spelling is `Value::Geospatial`; plain
        // bytes are accepted on the way in and rewritten here.
        D::Geometry(_) | D::Geography(_) => match value {
            Value::Geospatial(_) => Ok((value.clone(), false)),
            Value::Bytes(bytes) => Ok((Value::Geospatial(Arc::from(bytes.as_ref())), true)),
            other => Ok((other.clone(), false)),
        },
    }
}

fn canonical_signed(value: &Value) -> Result<(Value, bool)> {
    let Some(integer) = value.as_i128().and_then(|value| i64::try_from(value).ok()) else {
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
    Ok((
        Value::I64(integer),
        !matches!(value, Value::I64(current) if *current == integer),
    ))
}

fn canonical_unsigned(value: &Value) -> Result<(Value, bool)> {
    let Some(integer) = value.as_u128().and_then(|value| u64::try_from(value).ok()) else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated unsigned value could not be canonicalized"),
        });
    };
    Ok((
        Value::U64(integer),
        !matches!(value, Value::U64(current) if *current == integer),
    ))
}

enum FloatWidth {
    Float16,
    Float32,
}

fn canonical_float(value: &Value, width: FloatWidth) -> Result<(Value, bool)> {
    let Some(number) = value.as_f64() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated float value could not be canonicalized"),
        });
    };
    let narrowed = match width {
        FloatWidth::Float16 => f64::from(half::f16::from_f64(number)),
        FloatWidth::Float32 => f64::from(number as f32),
    };
    let canonical = Value::from(narrowed);
    let changed = !matches!(
        (value, &canonical),
        (Value::F64(current), Value::F64(canonical)) if current == canonical
    );
    Ok((canonical, changed))
}

fn canonical_integer_sequence(value: &Value, length: usize) -> Result<(Value, bool)> {
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
    canonical_sequence(value, canonical_signed)
}

fn canonical_sequence(
    value: &Value,
    mut canonicalize: impl FnMut(&Value) -> Result<(Value, bool)>,
) -> Result<(Value, bool)> {
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
        Ok((Value::from_sequence(canonical), true))
    } else {
        Ok((value.clone(), false))
    }
}

fn canonical_struct(fields: &Fields, value: &Value) -> Result<(Value, bool)> {
    // A struct value reads back from Arrow as its typed record; both
    // spellings carry one value per field, so a record canonicalizes as the
    // plain sequence it wraps - the same two spellings the materializer and
    // the default planner already recognize.
    if let Some((_, values)) = value.as_record() {
        if values.len() != fields.len() {
            return canonicalization_failure(&DataType::Struct(fields.clone()));
        }
        let canonical = canonicalize_slice(values, |index, value| {
            canonicalize_field_value(&fields[index], value)
        })?
        .unwrap_or_else(|| values.to_vec());
        return Ok((Value::from_sequence(canonical), true));
    }
    let Some(values) = value.as_sequence() else {
        return canonicalization_failure(&DataType::Struct(fields.clone()));
    };
    if let Some(canonical) = canonicalize_slice(values, |index, value| {
        canonicalize_field_value(&fields[index], value)
    })? {
        Ok((Value::from_sequence(canonical), true))
    } else {
        Ok((value.clone(), false))
    }
}

fn canonical_union(fields: &crate::UnionFields, value: &Value) -> Result<(Value, bool)> {
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
        !matches!(type_id, Value::I64(current) if *current == i64::from(type_id_number));
    if id_changed || payload_changed {
        Ok((
            Value::from_sequence([Value::I64(i64::from(type_id_number)), payload]),
            true,
        ))
    } else {
        Ok((value.clone(), false))
    }
}

fn canonical_map(map: &crate::MapType, value: &Value) -> Result<(Value, bool)> {
    let Some(entries) = value.as_mapping() else {
        return canonicalization_failure(&DataType::Map(map.clone().into()));
    };
    let Some([key_field, value_field]) = map.entries().data_type().as_fields() else {
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
            let canonical = Value::from_mapping(canonical)?;
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

fn duplicate_mapping_key_index(entries: &[(Value, Value)]) -> Option<usize> {
    if entries.len() <= 16 {
        return (1..entries.len()).find(|index| {
            entries[..*index]
                .iter()
                .any(|(key, _)| key == &entries[*index].0)
        });
    }
    // `Value`'s hash reads canonical content only, never the
    // interior-mutable caches a datatype holds, so the key is stable.
    #[allow(clippy::mutable_key_type)]
    let mut seen = HashSet::with_capacity(entries.len());
    entries
        .iter()
        .enumerate()
        .find_map(|(index, (key, _))| (!seen.insert(key)).then_some(index))
}

fn canonicalize_slice(
    values: &[Value],
    mut canonicalize: impl FnMut(usize, &Value) -> Result<(Value, bool)>,
) -> Result<Option<Vec<Value>>> {
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

fn canonicalization_failure<T>(data_type: &DataType) -> Result<T> {
    Err(Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!(
            "validated {} value could not be canonicalized",
            data_type.name()
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
    value: &Value,
) -> std::result::Result<(), ValidationFailure> {
    validate_field_value_at_depth(field, value, 0)
}

fn validate_field_value_at_depth(
    field: &Field,
    value: &Value,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    validate_field_payload_at_depth(field, value, depth)
        .map_err(|failure| failure.prepend(PathSegment::Field(SmolStr::new(field.name()))))
}

fn validate_field_payload_at_depth(
    field: &Field,
    value: &Value,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    if depth >= DataType::PARSE_RECURSION_LIMIT {
        return Err(ValidationFailure::new(format_smolstr!(
            "record nesting exceeds the hard limit of {}",
            DataType::PARSE_RECURSION_LIMIT
        )));
    }
    if value_is_logically_null(field.data_type(), value) && !field.is_nullable() {
        return Err(ValidationFailure::new("non-nullable field received null"));
    }
    if matches!(value, Value::Null)
        && !matches!(
            field.data_type(),
            DataType::Union(..) | DataType::RunEndEncoded(_)
        )
    {
        return Ok(());
    }
    validate_data_type_value(field.data_type(), value, depth)
}

#[allow(clippy::too_many_lines)]
fn validate_data_type_value(
    data_type: &DataType,
    value: &Value,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    use DataType as D;
    if let Some(physical) = restated(data_type, value) {
        return validate_data_type_value(data_type, &Value::I128(physical), depth);
    }
    match data_type {
        D::Null => Err(expected("null", value)),
        D::Boolean => require(matches!(value, Value::Bool(_)), "boolean", value),
        D::Int8 => validate_signed(value, i128::from(i8::MIN), i128::from(i8::MAX), "int8"),
        D::Int16 => validate_signed(value, i128::from(i16::MIN), i128::from(i16::MAX), "int16"),
        D::Int32 => validate_signed(value, i128::from(i32::MIN), i128::from(i32::MAX), "int32"),
        D::Int64 => validate_signed(value, i128::from(i64::MIN), i128::from(i64::MAX), "int64"),
        D::UInt8 => validate_unsigned(value, u128::from(u8::MAX), "uint8"),
        D::UInt16 => validate_unsigned(value, u128::from(u16::MAX), "uint16"),
        D::UInt32 => validate_unsigned(value, u128::from(u32::MAX), "uint32"),
        D::UInt64 => validate_unsigned(value, u128::from(u64::MAX), "uint64"),
        D::Float16 | D::Float32 | D::Float64 => {
            require(matches!(value, Value::F64(_)), data_type.name(), value)
        }
        D::Timestamp(..) | D::Duration(_) => validate_signed(
            value,
            i128::from(i64::MIN),
            i128::from(i64::MAX),
            data_type.name(),
        ),
        D::Date32 => validate_signed(
            value,
            i128::from(i32::MIN),
            i128::from(i32::MAX),
            data_type.name(),
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
            require(matches!(value, Value::Bytes(_)), data_type.name(), value)
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
            require(matches!(value, Value::String(_)), data_type.name(), value)
        }
        D::List(field) | D::ListView(field) | D::LargeList(field) | D::LargeListView(field) => {
            validate_sequence(field, value, None, data_type.name(), depth + 1)
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
        D::Dictionary(dictionary) => validate_data_type_value(dictionary.value(), value, depth + 1),
        D::Decimal32 { precision, .. } => validate_decimal(value, *precision, 32),
        D::Decimal64 { precision, .. } => validate_decimal(value, *precision, 64),
        D::Decimal128 { precision, .. } => validate_decimal(value, *precision, 128),
        D::Decimal256 { precision, .. } => validate_decimal256(value, *precision),
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
                .map_err(|error| expected_because(data_type.name(), value, &error)),
            None => Err(expected(data_type.name(), value)),
        },
    }
}

fn validate_signed(
    value: &Value,
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
    value: &Value,
    maximum: u128,
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    match value.as_u128() {
        Some(value) if value <= maximum => Ok(()),
        _ => Err(expected(expected_name, value)),
    }
}

fn validate_date64(value: &Value) -> std::result::Result<(), ValidationFailure> {
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

fn validate_time(value: &Value, unit: TimeUnit) -> std::result::Result<(), ValidationFailure> {
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
    value: &Value,
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
    value: &Value,
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
    value: &Value,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
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

fn validate_union(
    fields: &crate::UnionFields,
    value: &Value,
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
    value: &Value,
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

fn validate_decimal256(value: &Value, precision: u8) -> std::result::Result<(), ValidationFailure> {
    let Some(encoded) = value.as_str() else {
        return Err(expected("canonical decimal256 coefficient string", value));
    };
    let (negative, digits) = encoded
        .strip_prefix('-')
        .map_or((false, encoded), |digits| (true, digits));
    if digits.is_empty()
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
        || (negative && digits == "0")
    {
        return Err(ValidationFailure::new(
            "decimal256 coefficient must be canonical signed base-10 text",
        ));
    }
    if digits.len() > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal256 value exceeds precision {precision}"
        )));
    }
    Ok(())
}

/// Report a value whose kind does not match what the schema declared.
fn expected(expected_name: &str, value: &Value) -> ValidationFailure {
    ValidationFailure::new(crate::text::expected_got(expected_name, value.kind()))
}

/// [`expected`], carrying the refusal that says why the payload failed.
fn expected_because(
    expected_name: &str,
    value: &Value,
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
    value: &Value,
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
    value: &Value,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    let entries = value
        .as_mapping()
        .ok_or_else(|| expected("map entries", value))?;
    let Some([key_field, value_field]) = map.entries().data_type().as_fields() else {
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
