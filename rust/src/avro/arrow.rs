//! The mapping between Avro schemas and the crate's `Field` schema.
//!
//! Reading maps every Avro construct the value model honors onto its Arrow
//! shape - logical dates, times, instants, decimals, and durations included -
//! and a union of `null` and one branch becomes that branch, nullable, which
//! is how an optional column is meant to read. Writing is the inverse walk,
//! and a datatype Avro cannot spell is refused by name rather than silently
//! narrowed.

use smol_str::{SmolStr, format_smolstr};

use crate::generic::TimeUnit;
use crate::{DataType, Field, Result, Scalar, Timezone};

use super::datum::invalid;
use super::schema::{Node, Schema};

/// Project an Avro schema as the crate's `Field` schema.
///
/// A record root keeps its own (bare) name; any other root becomes a struct
/// of one column called `value` under `root_name`, because the record surface
/// exchanges batches of rows and a row is a struct.
///
/// # Errors
///
/// Returns an error when the schema uses a construct the record surface does
/// not cover - a union that is not `null` plus one branch, or a recursive
/// type, which Arrow cannot represent.
pub(crate) fn field_from_schema(schema: &Schema, root_name: &str) -> Result<Field> {
    let mut visiting = Vec::new();
    if let Node::Record(record) = &schema.node {
        let name = record
            .name
            .rsplit_once('.')
            .map_or(record.name.as_str(), |(_, bare)| bare);
        let (dtype, _) = struct_of(record, schema, &mut visiting)?;
        return Ok(Field::new(name, dtype, false));
    }
    let (dtype, nullable) = dtype_from(&schema.node, schema, &mut visiting)?;
    let value = Field::new("value", dtype, nullable);
    Ok(Field::new(
        root_name,
        DataType::from_fields([value])?,
        false,
    ))
}

/// Map a record node onto a struct datatype.
fn struct_of(
    record: &super::schema::RecordType,
    schema: &Schema,
    visiting: &mut Vec<SmolStr>,
) -> Result<(DataType, bool)> {
    if visiting.contains(&record.name) {
        return Err(invalid(format_smolstr!(
            "expected a non-recursive Avro schema for the record surface, got {:?} inside itself",
            record.name
        )));
    }
    visiting.push(record.name.clone());
    let mut fields = Vec::with_capacity(record.fields.len());
    for field in &record.fields {
        let (dtype, nullable) = dtype_from(&field.schema, schema, visiting)?;
        let mut built = Field::new(field.name.clone(), dtype, nullable);
        // The Iceberg `field-id` rides into the same metadata slot the
        // Parquet path uses, so id-based column resolution works over both
        // data file formats.
        if let Some(id) = field.field_id {
            built.set_parquet_field_id(id);
        }
        fields.push(built);
    }
    visiting.pop();
    Ok((DataType::from_fields(fields)?, false))
}

/// Map one Avro node onto a datatype and its nullability.
fn dtype_from(
    node: &Node,
    schema: &Schema,
    visiting: &mut Vec<SmolStr>,
) -> Result<(DataType, bool)> {
    Ok(match node {
        Node::Null => (DataType::Null, true),
        Node::Boolean => (DataType::Boolean, false),
        Node::Int => (DataType::Int32, false),
        Node::Long => (DataType::Int64, false),
        Node::Float => (DataType::Float32, false),
        Node::Double => (DataType::Float64, false),
        Node::Bytes => (DataType::Binary, false),
        Node::String | Node::Uuid => (DataType::Utf8, false),
        Node::Date => (DataType::Date32, false),
        Node::TimeMillis => (DataType::Time32(TimeUnit::Millisecond), false),
        Node::TimeMicros => (DataType::Time64(TimeUnit::Microsecond), false),
        Node::TimestampMillis => (
            DataType::Timestamp(TimeUnit::Millisecond, Some(Timezone::UTC)),
            false,
        ),
        Node::TimestampMicros => (
            DataType::Timestamp(TimeUnit::Microsecond, Some(Timezone::UTC)),
            false,
        ),
        Node::TimestampNanos => (
            DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
            false,
        ),
        Node::LocalTimestampMillis => (DataType::Timestamp(TimeUnit::Millisecond, None), false),
        Node::LocalTimestampMicros => (DataType::Timestamp(TimeUnit::Microsecond, None), false),
        Node::LocalTimestampNanos => (DataType::Timestamp(TimeUnit::Nanosecond, None), false),
        Node::Decimal(decimal) => (
            DataType::decimal128(
                u8::try_from(decimal.precision).unwrap_or(38),
                i8::try_from(decimal.scale).unwrap_or(0),
            )?,
            false,
        ),
        Node::Duration(_) => (DataType::Interval(TimeUnit::MonthDayNano), false),
        Node::UuidFixed(fixed) | Node::Fixed(fixed) => (
            DataType::FixedSizeBinary(i32::try_from(fixed.size).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a fixed size within 32 bits, got {}",
                    fixed.size
                ))
            })?),
            false,
        ),
        Node::Enum(_) => (DataType::Utf8, false),
        Node::Record(record) => struct_of(record, schema, visiting)?,
        Node::Array(items) => {
            let (item_type, nullable) = dtype_from(items, schema, visiting)?;
            (
                DataType::list(Field::new("item", item_type, nullable)),
                false,
            )
        }
        Node::Map(values) => {
            let (value_type, nullable) = dtype_from(values, schema, visiting)?;
            let value = Field::new("value", value_type, nullable);
            let key = Field::new("key", DataType::Utf8, false);
            let entries = Field::new("entries", DataType::from_fields([key, value])?, false);
            (DataType::map(entries, false)?, false)
        }
        Node::Union(branches) => union_from(branches, schema, visiting)?,
        Node::Ref(name) => {
            let target = schema.names.get(name).cloned().ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a declared Avro type named {name:?}"
                ))
            })?;
            dtype_from(&target, schema, visiting)?
        }
    })
}

/// Map a union: `null` plus one branch is that branch, nullable.
fn union_from(
    branches: &[Node],
    schema: &Schema,
    visiting: &mut Vec<SmolStr>,
) -> Result<(DataType, bool)> {
    let nulls = branches
        .iter()
        .filter(|branch| matches!(branch, Node::Null))
        .count();
    let others: Vec<&Node> = branches
        .iter()
        .filter(|branch| !matches!(branch, Node::Null))
        .collect();
    match (nulls, others.as_slice()) {
        (_, []) => Ok((DataType::Null, true)),
        (0, [only]) => dtype_from(only, schema, visiting),
        (_, [only]) => {
            let (dtype, _) = dtype_from(only, schema, visiting)?;
            Ok((dtype, true))
        }
        _ => Err(invalid(format_smolstr!(
            "expected an Avro union of null and at most one branch for the record surface, got \
             {} branches",
            branches.len()
        ))),
    }
}

/// Render a `Field` schema as the Avro schema JSON it writes as.
///
/// # Errors
///
/// Returns an error naming any datatype Avro cannot spell.
pub(crate) fn schema_json_from_field(field: &Field) -> Result<Scalar> {
    let mut counter = 0_usize;
    let DataType::Struct(_) = field.dtype() else {
        return Err(invalid(format_smolstr!(
            "expected a struct root to write Avro records, got {}",
            field.dtype()
        )));
    };
    record_json(field.name(), field.fields(), &mut counter)
}

/// Render one record schema.
fn record_json(name: &str, fields: &[Field], counter: &mut usize) -> Result<Scalar> {
    let mut entries = Vec::with_capacity(fields.len());
    for field in fields {
        let mut declared = node_json(field.dtype(), field.name(), counter)
            .map_err(|error| locate(error, field.name()))?;
        // A null-typed column is already the null it would be wrapped in; a
        // ["null","null"] union is illegal, so the wrap is skipped.
        if field.is_nullable() && declared.as_str() != Some("null") {
            declared = Scalar::from_sequence([Scalar::from("null"), declared]);
        }
        let mut pairs = vec![("name", Scalar::from(field.name())), ("type", declared)];
        if field.is_nullable() {
            pairs.push(("default", Scalar::Null));
        }
        if let Ok(Some(id)) = field.parquet_field_id() {
            pairs.push(("field-id", Scalar::from(i64::from(id))));
        }
        entries.push(Scalar::from_record(pairs)?);
    }
    Scalar::from_record([
        ("type", Scalar::from("record")),
        ("name", Scalar::from(name)),
        ("fields", Scalar::from_sequence(entries)),
    ])
}

/// Render one non-nullable datatype as its Avro schema.
fn node_json(dtype: &DataType, name: &str, counter: &mut usize) -> Result<Scalar> {
    let plain = |kind: &'static str| Ok(Scalar::from(kind));
    let logical = |kind: &'static str, annotation: &'static str| {
        Scalar::from_record([
            ("type", Scalar::from(kind)),
            ("logicalType", Scalar::from(annotation)),
        ])
    };
    match dtype {
        DataType::Null => plain("null"),
        DataType::Boolean => plain("boolean"),
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::UInt8 | DataType::UInt16 => {
            plain("int")
        }
        DataType::Int64 | DataType::UInt32 => plain("long"),
        DataType::Float16 | DataType::Float32 => plain("float"),
        DataType::Float64 => plain("double"),
        // An ASCII width, and a code over one, is text on the wire; the cast
        // plan trims the padding before the encoder sees a value.
        DataType::Utf8
        | DataType::LargeUtf8
        | DataType::Utf8View
        | DataType::Ascii16
        | DataType::Ascii24
        | DataType::Ascii32
        | DataType::Ascii64
        | DataType::Ascii96
        | DataType::Ascii128
        | DataType::Country
        | DataType::Currency
        | DataType::Mic
        | DataType::Cfi => plain("string"),
        // Avro's `uuid` annotates a string with the hyphenated spelling,
        // which is what a GUID value already is.
        DataType::Guid => logical("string", "uuid"),
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => plain("bytes"),
        DataType::Date32 => logical("int", "date"),
        DataType::Time32(TimeUnit::Millisecond) => logical("int", "time-millis"),
        DataType::Time64(TimeUnit::Microsecond) => logical("long", "time-micros"),
        DataType::Timestamp(unit, zone) => {
            let annotation = match (unit, zone.is_some()) {
                (TimeUnit::Millisecond, true) => "timestamp-millis",
                (TimeUnit::Microsecond, true) => "timestamp-micros",
                (TimeUnit::Nanosecond, true) => "timestamp-nanos",
                (TimeUnit::Millisecond, false) => "local-timestamp-millis",
                (TimeUnit::Microsecond, false) => "local-timestamp-micros",
                (TimeUnit::Nanosecond, false) => "local-timestamp-nanos",
                _ => return Err(unspellable(dtype)),
            };
            logical("long", annotation)
        }
        DataType::Interval(TimeUnit::MonthDayNano) => {
            *counter += 1;
            Scalar::from_record([
                ("type", Scalar::from("fixed")),
                ("name", Scalar::from(unique_name(name, counter))),
                ("size", Scalar::from(12_i64)),
                ("logicalType", Scalar::from("duration")),
            ])
        }
        DataType::FixedSizeBinary(size) => {
            *counter += 1;
            Scalar::from_record([
                ("type", Scalar::from("fixed")),
                ("name", Scalar::from(unique_name(name, counter))),
                ("size", Scalar::from(i64::from(*size))),
            ])
        }
        DataType::Decimal128 { precision, scale } => {
            if *scale < 0 {
                return Err(unspellable(dtype));
            }
            Scalar::from_record([
                ("type", Scalar::from("bytes")),
                ("logicalType", Scalar::from("decimal")),
                ("precision", Scalar::from(i64::from(*precision))),
                ("scale", Scalar::from(i64::from(*scale))),
            ])
        }
        DataType::Struct(_) => {
            *counter += 1;
            let record_name = unique_name(name, counter);
            let fields = dtype.as_fields().ok_or_else(|| unspellable(dtype))?;
            record_json(&record_name, fields, counter)
        }
        DataType::List(item) | DataType::LargeList(item) => {
            let mut items = node_json(item.dtype(), item.name(), counter)?;
            if item.is_nullable() && items.as_str() != Some("null") {
                items = Scalar::from_sequence([Scalar::from("null"), items]);
            }
            Scalar::from_record([("type", Scalar::from("array")), ("items", items)])
        }
        DataType::Map(map) => {
            let entries = map.entries().fields();
            let key = entries.first().ok_or_else(|| unspellable(dtype))?;
            let value = entries.get(1).ok_or_else(|| unspellable(dtype))?;
            if !matches!(
                key.dtype(),
                DataType::Utf8
                    | DataType::LargeUtf8
                    | DataType::Utf8View
                    | DataType::Ascii16
                    | DataType::Ascii24
                    | DataType::Ascii32
                    | DataType::Ascii64
                    | DataType::Ascii96
                    | DataType::Ascii128
                    | DataType::Country
                    | DataType::Currency
                    | DataType::Mic
                    | DataType::Cfi
            ) {
                return Err(unspellable(dtype));
            }
            let mut values = node_json(value.dtype(), value.name(), counter)?;
            if value.is_nullable() && values.as_str() != Some("null") {
                values = Scalar::from_sequence([Scalar::from("null"), values]);
            }
            Scalar::from_record([("type", Scalar::from("map")), ("values", values)])
        }
        other => Err(unspellable(other)),
    }
}

/// Derive a schema-unique Avro type name from a column name.
fn unique_name(name: &str, counter: &usize) -> SmolStr {
    let mut cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if cleaned
        .chars()
        .next()
        .is_none_or(|first| first.is_ascii_digit())
    {
        cleaned.insert(0, 'r');
    }
    format_smolstr!("{cleaned}_{counter}")
}

/// Report a datatype the Avro format cannot spell.
fn unspellable(dtype: &DataType) -> crate::Error {
    invalid(format_smolstr!(
        "expected a datatype Avro can spell, got {dtype}"
    ))
}

/// Locate a schema-rendering failure at its column.
fn locate(error: crate::Error, column: &str) -> crate::Error {
    match error {
        crate::Error::Codec {
            format,
            position,
            reason,
        } => crate::Error::Codec {
            format,
            position,
            reason: format_smolstr!("{column}: {reason}"),
        },
        other => other,
    }
}
