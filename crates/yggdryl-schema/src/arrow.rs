//! Fast, near-total conversion to and from [`arrow-schema`](https://docs.rs/arrow-schema)
//! (`DataType` / `Field` / `Schema`), behind the `arrow` feature.
//!
//! The mapping is structural. The simplified model maps onto Arrow's wider variant
//! set: a [`Varchar`](DataType::Varchar) becomes `Utf8` / `LargeUtf8` / `Utf8View`
//! by its `large` / `view` flags (a non-UTF-8 [`Charset`](crate::Charset) maps to
//! UTF-8 — Arrow has no charset, so that part is lossy), and a fixed-size
//! [`Binary`](DataType::Binary) / [`List`](DataType::List) becomes the matching
//! fixed-size Arrow type. The only value with no Arrow equivalent is
//! [`Any`](DataType::Any), which errors.
//!
//! A few attributes the simplified model does not carry are **normalised** on the
//! round-trip rather than preserved: a [`Union`](DataType::Union)'s type ids are
//! reassigned `0, 1, …` (so an imported union with non-contiguous external ids does
//! not round-trip), a [`Map`](DataType::Map)'s key/value entry-field names and
//! nullability follow the Arrow convention (`key` non-null, `value` nullable), an
//! Arrow timestamp timezone string that is not a recognised zone falls back to UTC
//! with a `warn` log, a fixed-length [`Varchar`](DataType::Varchar) loses its length
//! (Arrow has no fixed UTF-8), and [`Json`](DataType::Json) / [`Bson`](DataType::Bson)
//! map to their physical `Utf8` / `Binary` (the logical name is not recovered).
//!
//! ```
//! # #[cfg(feature = "arrow")] {
//! use yggdryl_schema::{DataType, Field};
//! let schema = Field::new("rec", DataType::struct_(vec![Field::new("id", DataType::int(64, true), false)]), false);
//! let arrow = schema.to_arrow_schema().unwrap();
//! assert_eq!(arrow.fields().len(), 1);
//! # }
//! ```

use std::sync::Arc;

use arrow_schema::{
    DataType as ADataType, Field as AField, Schema as ASchema, TimeUnit as ATimeUnit,
    UnionFields as AUnionFields, UnionMode as AUnionMode,
};

#[allow(unused_imports)]
use crate::log_event;
use crate::{Charset, DataType, Field, IntervalUnit, SchemaError, UnionMode};
use yggdryl_core::{TimeUnit, Timezone};

fn time_unit_to_arrow(unit: TimeUnit) -> ATimeUnit {
    match unit {
        TimeUnit::Second => ATimeUnit::Second,
        TimeUnit::Millisecond => ATimeUnit::Millisecond,
        TimeUnit::Microsecond => ATimeUnit::Microsecond,
        TimeUnit::Nanosecond => ATimeUnit::Nanosecond,
    }
}

fn time_unit_from_arrow(unit: &ATimeUnit) -> TimeUnit {
    match unit {
        ATimeUnit::Second => TimeUnit::Second,
        ATimeUnit::Millisecond => TimeUnit::Millisecond,
        ATimeUnit::Microsecond => TimeUnit::Microsecond,
        ATimeUnit::Nanosecond => TimeUnit::Nanosecond,
    }
}

fn interval_to_arrow(unit: IntervalUnit) -> arrow_schema::IntervalUnit {
    match unit {
        IntervalUnit::YearMonth => arrow_schema::IntervalUnit::YearMonth,
        IntervalUnit::DayTime => arrow_schema::IntervalUnit::DayTime,
        IntervalUnit::MonthDayNano => arrow_schema::IntervalUnit::MonthDayNano,
    }
}

fn interval_from_arrow(unit: &arrow_schema::IntervalUnit) -> IntervalUnit {
    match unit {
        arrow_schema::IntervalUnit::YearMonth => IntervalUnit::YearMonth,
        arrow_schema::IntervalUnit::DayTime => IntervalUnit::DayTime,
        arrow_schema::IntervalUnit::MonthDayNano => IntervalUnit::MonthDayNano,
    }
}

impl DataType {
    /// Converts to the matching [`arrow_schema::DataType`], or
    /// [`SchemaError::Unsupported`] for [`Any`](DataType::Any).
    pub fn to_arrow(&self) -> Result<ADataType, SchemaError> {
        use DataType::*;
        Ok(match self {
            Any => {
                return Err(SchemaError::Unsupported(
                    "the `any` type has no Arrow equivalent; resolve it before converting".into(),
                ))
            }
            Null => ADataType::Null,
            Boolean => ADataType::Boolean,
            Int { bits, signed } => match (bits, signed) {
                (8, true) => ADataType::Int8,
                (16, true) => ADataType::Int16,
                (32, true) => ADataType::Int32,
                (64, true) => ADataType::Int64,
                (8, false) => ADataType::UInt8,
                (16, false) => ADataType::UInt16,
                (32, false) => ADataType::UInt32,
                (64, false) => ADataType::UInt64,
                // Arrow only has the standard widths; a flexible width has no mapping.
                (bits, signed) => {
                    return Err(SchemaError::Unsupported(format!(
                        "Arrow has no {}int{bits}; use a standard width (8/16/32/64)",
                        if *signed { "" } else { "u" }
                    )))
                }
            },
            Float { bits } => match bits {
                16 => ADataType::Float16,
                32 => ADataType::Float32,
                64 => ADataType::Float64,
                // Arrow only has the IEEE widths; a custom width has no mapping.
                other => {
                    return Err(SchemaError::Unsupported(format!(
                        "Arrow has no float{other}; use a standard width (16/32/64)"
                    )))
                }
            },
            Varchar {
                large, view, size, ..
            } => {
                if size.is_some() {
                    log_event!(
                        warn,
                        "to_arrow: Arrow has no fixed-size UTF-8 string; dropping the fixed length"
                    );
                }
                match (large, view) {
                    (true, _) => ADataType::LargeUtf8,
                    (_, true) => ADataType::Utf8View,
                    _ => ADataType::Utf8,
                }
            }
            Binary { large, view, size } => match (large, view, size) {
                (_, _, Some(n)) => ADataType::FixedSizeBinary(*n),
                (true, _, None) => ADataType::LargeBinary,
                (_, true, None) => ADataType::BinaryView,
                _ => ADataType::Binary,
            },
            Decimal {
                precision,
                scale,
                bits,
            } => match bits {
                32 => ADataType::Decimal32(*precision, *scale),
                64 => ADataType::Decimal64(*precision, *scale),
                256 => ADataType::Decimal256(*precision, *scale),
                _ => ADataType::Decimal128(*precision, *scale),
            },
            Date { large } => {
                if *large {
                    ADataType::Date64
                } else {
                    ADataType::Date32
                }
            }
            Time { unit } => match unit {
                TimeUnit::Second | TimeUnit::Millisecond => {
                    ADataType::Time32(time_unit_to_arrow(*unit))
                }
                _ => ADataType::Time64(time_unit_to_arrow(*unit)),
            },
            Timestamp { unit, timezone } => ADataType::Timestamp(
                time_unit_to_arrow(*unit),
                timezone.as_ref().map(|tz| Arc::from(tz.name().as_str())),
            ),
            Duration { unit } => ADataType::Duration(time_unit_to_arrow(*unit)),
            Interval { unit } => ADataType::Interval(interval_to_arrow(*unit)),
            Dictionary { key, value } => {
                ADataType::Dictionary(Box::new(key.to_arrow()?), Box::new(value.to_arrow()?))
            }
            // Arrow has no first-class JSON/BSON/timezone; map to the physical string /
            // binary (the logical name is not recovered on `from_arrow`).
            Json => ADataType::Utf8,
            Bson => ADataType::Binary,
            Timezone => ADataType::Utf8,
            List {
                item,
                large,
                view,
                size,
            } => match (large, view, size) {
                (_, _, Some(n)) => ADataType::FixedSizeList(Arc::new(item.to_arrow()?), *n),
                (true, true, None) => ADataType::LargeListView(Arc::new(item.to_arrow()?)),
                (true, false, None) => ADataType::LargeList(Arc::new(item.to_arrow()?)),
                (false, true, None) => ADataType::ListView(Arc::new(item.to_arrow()?)),
                _ => ADataType::List(Arc::new(item.to_arrow()?)),
            },
            Struct(fields) => ADataType::Struct(fields_to_arrow(fields)?),
            Map { key, value, sorted } => {
                let entries = AField::new(
                    "entries",
                    ADataType::Struct(
                        vec![
                            AField::new("key", key.to_arrow()?, false),
                            AField::new("value", value.to_arrow()?, true),
                        ]
                        .into(),
                    ),
                    false,
                );
                ADataType::Map(Arc::new(entries), *sorted)
            }
            Union { fields, mode } => {
                // Arrow union type ids are i8 (0..=127); reject rather than wrap.
                if fields.len() > 128 {
                    return Err(SchemaError::Unsupported(format!(
                        "union has {} alternatives; Arrow allows at most 128 type ids",
                        fields.len()
                    )));
                }
                let pairs = fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| Ok::<_, SchemaError>((i as i8, Arc::new(f.to_arrow()?))))
                    .collect::<Result<Vec<_>, _>>()?;
                let arrow_mode = match mode {
                    UnionMode::Sparse => AUnionMode::Sparse,
                    UnionMode::Dense => AUnionMode::Dense,
                };
                ADataType::Union(pairs.into_iter().collect::<AUnionFields>(), arrow_mode)
            }
            RunEndEncoded { run_ends, values } => ADataType::RunEndEncoded(
                Arc::new(AField::new("run_ends", run_ends.to_arrow()?, false)),
                Arc::new(AField::new("values", values.to_arrow()?, true)),
            ),
        })
    }

    /// Converts from an [`arrow_schema::DataType`]. Total (Arrow has no `Any`), but
    /// union type ids and map entry-field names/nullability are normalised, not
    /// preserved (see the [module docs](self)).
    pub fn from_arrow(data_type: &ADataType) -> DataType {
        match data_type {
            ADataType::Null => DataType::Null,
            ADataType::Boolean => DataType::Boolean,
            ADataType::Int8 => DataType::int(8, true),
            ADataType::Int16 => DataType::int(16, true),
            ADataType::Int32 => DataType::int(32, true),
            ADataType::Int64 => DataType::int(64, true),
            ADataType::UInt8 => DataType::int(8, false),
            ADataType::UInt16 => DataType::int(16, false),
            ADataType::UInt32 => DataType::int(32, false),
            ADataType::UInt64 => DataType::int(64, false),
            ADataType::Float16 => DataType::float(16),
            ADataType::Float32 => DataType::float(32),
            ADataType::Float64 => DataType::float(64),
            ADataType::Utf8 => DataType::varchar(),
            ADataType::LargeUtf8 => DataType::varchar_with(Charset::Utf8, true, false, None),
            ADataType::Utf8View => DataType::varchar_with(Charset::Utf8, false, true, None),
            ADataType::Binary => DataType::binary(),
            ADataType::LargeBinary => DataType::Binary {
                large: true,
                view: false,
                size: None,
            },
            ADataType::BinaryView => DataType::Binary {
                large: false,
                view: true,
                size: None,
            },
            ADataType::FixedSizeBinary(n) => DataType::fixed_size_binary(*n),
            ADataType::Decimal32(p, s) => DataType::decimal_with(*p, *s, 32),
            ADataType::Decimal64(p, s) => DataType::decimal_with(*p, *s, 64),
            ADataType::Decimal128(p, s) => DataType::decimal_with(*p, *s, 128),
            ADataType::Decimal256(p, s) => DataType::decimal_with(*p, *s, 256),
            ADataType::Date32 => DataType::date(),
            ADataType::Date64 => DataType::Date { large: true },
            ADataType::Time32(u) | ADataType::Time64(u) => DataType::Time {
                unit: time_unit_from_arrow(u),
            },
            ADataType::Timestamp(u, tz) => DataType::Timestamp {
                unit: time_unit_from_arrow(u),
                timezone: tz.as_ref().map(|s| {
                    Timezone::from_str(s).unwrap_or_else(|_| {
                        log_event!(
                            warn,
                            "from_arrow: timestamp timezone {s:?} is not a recognised zone, defaulting to UTC"
                        );
                        Timezone::Utc
                    })
                }),
            },
            ADataType::Duration(u) => DataType::Duration {
                unit: time_unit_from_arrow(u),
            },
            ADataType::Interval(u) => DataType::Interval {
                unit: interval_from_arrow(u),
            },
            ADataType::Dictionary(k, v) => {
                DataType::dictionary(DataType::from_arrow(k), DataType::from_arrow(v))
            }
            ADataType::List(f) => DataType::list(Field::from_arrow(f)),
            ADataType::ListView(f) => DataType::List {
                item: Box::new(Field::from_arrow(f)),
                large: false,
                view: true,
                size: None,
            },
            ADataType::LargeList(f) => DataType::large_list(Field::from_arrow(f)),
            ADataType::LargeListView(f) => DataType::List {
                item: Box::new(Field::from_arrow(f)),
                large: true,
                view: true,
                size: None,
            },
            ADataType::FixedSizeList(f, n) => DataType::fixed_size_list(Field::from_arrow(f), *n),
            ADataType::Struct(fields) => {
                DataType::Struct(fields.iter().map(|f| Field::from_arrow(f)).collect())
            }
            ADataType::Map(entries, sorted) => {
                if let ADataType::Struct(kv) = entries.data_type() {
                    if kv.len() >= 2 {
                        return DataType::map(
                            DataType::from_arrow(kv[0].data_type()),
                            DataType::from_arrow(kv[1].data_type()),
                            *sorted,
                        );
                    }
                }
                DataType::map(DataType::Any, DataType::Any, *sorted)
            }
            ADataType::Union(union_fields, mode) => DataType::Union {
                fields: union_fields
                    .iter()
                    .map(|(_, f)| Field::from_arrow(f))
                    .collect(),
                mode: match mode {
                    AUnionMode::Sparse => UnionMode::Sparse,
                    AUnionMode::Dense => UnionMode::Dense,
                },
            },
            ADataType::RunEndEncoded(run_ends, values) => DataType::run_end_encoded(
                DataType::from_arrow(run_ends.data_type()),
                DataType::from_arrow(values.data_type()),
            ),
        }
    }
}

/// Converts a slice of [`Field`]s to [`arrow_schema::Fields`].
fn fields_to_arrow(fields: &[Field]) -> Result<arrow_schema::Fields, SchemaError> {
    let converted = fields
        .iter()
        .map(Field::to_arrow)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(converted.into())
}

impl Field {
    /// Converts to an [`arrow_schema::Field`] (errors if the type is [`Any`](DataType::Any)).
    pub fn to_arrow(&self) -> Result<AField, SchemaError> {
        let field = AField::new(
            self.name(),
            self.data_type().to_arrow()?,
            self.is_nullable(),
        );
        Ok(if self.metadata().is_empty() {
            field
        } else {
            field.with_metadata(
                self.metadata()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        })
    }

    /// Converts from an [`arrow_schema::Field`].
    pub fn from_arrow(field: &AField) -> Field {
        let converted = Field::new(
            field.name(),
            DataType::from_arrow(field.data_type()),
            field.is_nullable(),
        );
        if field.metadata().is_empty() {
            converted
        } else {
            converted.with_metadata(
                field
                    .metadata()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        }
    }

    /// Converts a struct-typed field to an [`arrow_schema::Schema`] (its fields plus
    /// the field's own metadata as schema metadata). Errors with
    /// [`SchemaError::NotAStruct`] if the type is not a struct.
    pub fn to_arrow_schema(&self) -> Result<ASchema, SchemaError> {
        let DataType::Struct(fields) = self.data_type() else {
            return Err(SchemaError::NotAStruct(self.name().to_string()));
        };
        Ok(ASchema::new_with_metadata(
            fields_to_arrow(fields)?,
            self.metadata()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ))
    }

    /// Builds a named struct field from an [`arrow_schema::Schema`], restoring the
    /// `name` / `nullable` an Arrow `Schema` does not carry.
    pub fn from_arrow_schema(name: impl Into<String>, schema: &ASchema, nullable: bool) -> Field {
        let fields = schema
            .fields()
            .iter()
            .map(|f| Field::from_arrow(f))
            .collect();
        Field::new(name, DataType::Struct(fields), nullable).with_metadata(
            schema
                .metadata()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }
}

impl TryFrom<&DataType> for ADataType {
    type Error = SchemaError;
    fn try_from(value: &DataType) -> Result<ADataType, SchemaError> {
        value.to_arrow()
    }
}

impl From<&ADataType> for DataType {
    fn from(value: &ADataType) -> DataType {
        DataType::from_arrow(value)
    }
}

impl TryFrom<&Field> for AField {
    type Error = SchemaError;
    fn try_from(value: &Field) -> Result<AField, SchemaError> {
        value.to_arrow()
    }
}

impl From<&AField> for Field {
    fn from(value: &AField) -> Field {
        Field::from_arrow(value)
    }
}
