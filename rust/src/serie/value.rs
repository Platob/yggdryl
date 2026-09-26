//! Schema-directed scalar/array conversion.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::budget::{
    MAX_PHYSICAL_SLOTS, MaterializationBudget, checked_physical_mul, invalid_value,
    physical_limit_error, physical_union_branch, unsupported,
};
use crate::string::is_text_storage;
use crate::{
    BBG_WIDTH, Bytes, BytesType, CCY_WIDTH, CFI_WIDTH, COUNTRY_WIDTH, CUSIP_WIDTH, FIGI_WIDTH,
    ISIN_WIDTH, MIC_WIDTH, RIC_WIDTH, SEDOL_WIDTH, SIDE_WIDTH, STATE_WIDTH, Str, StringType,
    TIMEINFORCE_WIDTH, UNIT_WIDTH, ascii_bytes, code_cell_text, uuid_bytes, uuid_parse,
};
use crate::{DataType, Field, Scalar, TimeUnit, Timezone, UnionMode, i256};
use arrow_array::builder::{BinaryBuilder, LargeStringBuilder, StringBuilder, StringViewBuilder};
use arrow_array::types::{
    ArrowPrimitiveType, Date32Type, Date64Type, Decimal32Type, Decimal64Type, Decimal128Type,
    Decimal256Type, DurationMicrosecondType, DurationMillisecondType, DurationNanosecondType,
    DurationSecondType, Float16Type, Float32Type, Float64Type, Int8Type, Int16Type, Int32Type,
    Int64Type, IntervalDayTimeType, IntervalMonthDayNanoType, IntervalYearMonthType,
    Time32MillisecondType, Time32SecondType, Time64MicrosecondType, Time64NanosecondType,
    TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType,
    TimestampSecondType, UInt8Type, UInt16Type, UInt32Type, UInt64Type,
};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, Date32Array, Date64Array,
    Decimal32Array, Decimal64Array, Decimal128Array, Decimal256Array, DictionaryArray,
    DurationMicrosecondArray, DurationMillisecondArray, DurationNanosecondArray,
    DurationSecondArray, FixedSizeBinaryArray, FixedSizeListArray, Float16Array, Float32Array,
    Float64Array, GenericBinaryArray, GenericListArray, GenericListViewArray, Int8Array,
    Int16Array, Int16RunArray, Int32Array, Int32RunArray, Int64Array, Int64RunArray,
    IntervalDayTimeArray, IntervalMonthDayNanoArray, IntervalYearMonthArray, LargeBinaryArray,
    LargeListArray, LargeListViewArray, LargeStringArray, ListArray, ListViewArray, MapArray,
    NullArray, PrimitiveArray, StringArray, StringViewArray, StructArray, Time32MillisecondArray,
    Time32SecondArray, Time64MicrosecondArray, Time64NanosecondArray, TimestampMicrosecondArray,
    TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt8Array,
    UInt16Array, UInt32Array, UInt64Array, UnionArray, make_array, new_empty_array,
};
use arrow_buffer::{
    BooleanBufferBuilder, Buffer, IntervalDayTime, IntervalMonthDayNano, NullBuffer,
    NullBufferBuilder, OffsetBuffer, ScalarBuffer, i256 as ArrowI256,
};
use arrow_schema::{DataType as ArrowDataType, FieldRef};
use half::f16;

use crate::arrow::{Error, Result};

#[allow(clippy::too_many_lines)]
pub(crate) fn array_of_rows(field: &Field, values: &[&Scalar]) -> Result<ArrayRef> {
    let dtype = field.dtype();
    // The field's own projection, built once into its cache and borrowed
    // here: every nested layout below takes its child fields out of it,
    // so no child is projected again on the way down.
    let arrow_type = field.as_arrow_field_ref()?.data_type();
    // Arrow owns the canonical empty representation for every validated
    // datatype. Taking this path before schema-directed value materialization
    // avoids inventing defaults for children that have no physical slots.
    if values.is_empty() {
        return Ok(new_empty_array(arrow_type));
    }
    macro_rules! primitive {
        ($native:ty, $conversion:expr) => {
            primitive_array::<$native>(values, arrow_type, $conversion)?
        };
    }
    let array = match dtype {
        DataType::Null => Arc::new(NullArray::new(values.len())) as ArrayRef,
        DataType::Boolean => boolean_array(values)?,
        DataType::Int8 => primitive!(Int8Type, |value: &Scalar| exact_i128(value)
            .and_then(|value| i8::try_from(value).map_err(|_| invalid_value("int8", value)))),
        DataType::Int16 => primitive!(Int16Type, |value: &Scalar| exact_i128(value)
            .and_then(|value| i16::try_from(value).map_err(|_| invalid_value("int16", value)))),
        DataType::Int32 => primitive!(Int32Type, |value: &Scalar| exact_i128(value)
            .and_then(|value| i32::try_from(value).map_err(|_| invalid_value("int32", value)))),
        DataType::Int64 => primitive!(Int64Type, |value: &Scalar| exact_i128(value)
            .and_then(|value| i64::try_from(value).map_err(|_| invalid_value("int64", value)))),
        DataType::UInt8 => primitive!(UInt8Type, |value: &Scalar| exact_u128(value)
            .and_then(|value| u8::try_from(value).map_err(|_| invalid_value("uint8", value)))),
        DataType::UInt16 => primitive!(UInt16Type, |value: &Scalar| exact_u128(value)
            .and_then(|value| u16::try_from(value).map_err(|_| invalid_value("uint16", value)))),
        DataType::UInt32 => primitive!(UInt32Type, |value: &Scalar| exact_u128(value)
            .and_then(|value| u32::try_from(value).map_err(|_| invalid_value("uint32", value)))),
        DataType::UInt64 => primitive!(UInt64Type, |value: &Scalar| exact_u128(value)
            .and_then(|value| u64::try_from(value).map_err(|_| invalid_value("uint64", value)))),
        DataType::Float16 => primitive!(Float16Type, |value: &Scalar| exact_f64(value)
            .map(f16::from_f64)),
        DataType::Float32 => primitive!(Float32Type, narrow_f32),
        DataType::Float64 => primitive!(Float64Type, |value: &Scalar| exact_f64(value)),
        DataType::DateTime64 { unit, .. } => match unit {
            TimeUnit::Second => primitive!(TimestampSecondType, temporal_i64(*unit)),
            TimeUnit::Millisecond => {
                primitive!(TimestampMillisecondType, temporal_i64(*unit))
            }
            TimeUnit::Microsecond => {
                primitive!(TimestampMicrosecondType, temporal_i64(*unit))
            }
            TimeUnit::Nanosecond => {
                primitive!(TimestampNanosecondType, temporal_i64(*unit))
            }
            _ => return Err(unsupported(dtype, "invalid timestamp unit")),
        },
        DataType::Date32 => primitive!(Date32Type, date_i32),
        DataType::Date64 => primitive!(Date64Type, date_i64),
        DataType::Time32(unit) => match unit {
            TimeUnit::Second => primitive!(Time32SecondType, temporal_i32(*unit)),
            TimeUnit::Millisecond => primitive!(Time32MillisecondType, temporal_i32(*unit)),
            _ => return Err(unsupported(dtype, "invalid time32 unit")),
        },
        DataType::Time64(unit) => match unit {
            TimeUnit::Microsecond => primitive!(Time64MicrosecondType, temporal_i64(*unit)),
            TimeUnit::Nanosecond => primitive!(Time64NanosecondType, temporal_i64(*unit)),
            _ => return Err(unsupported(dtype, "invalid time64 unit")),
        },
        DataType::Duration32(unit) => match unit {
            TimeUnit::Second => primitive!(DurationSecondType, |value: &Scalar| {
                temporal_i32(*unit)(value).map(i64::from)
            }),
            TimeUnit::Millisecond => primitive!(DurationMillisecondType, |value: &Scalar| {
                temporal_i32(*unit)(value).map(i64::from)
            }),
            TimeUnit::Microsecond => primitive!(DurationMicrosecondType, |value: &Scalar| {
                temporal_i32(*unit)(value).map(i64::from)
            }),
            TimeUnit::Nanosecond => primitive!(DurationNanosecondType, |value: &Scalar| {
                temporal_i32(*unit)(value).map(i64::from)
            }),
            _ => return Err(unsupported(dtype, "invalid duration32 unit")),
        },
        DataType::Duration64(unit) => match unit {
            TimeUnit::Second => primitive!(DurationSecondType, temporal_i64(*unit)),
            TimeUnit::Millisecond => primitive!(DurationMillisecondType, temporal_i64(*unit)),
            TimeUnit::Microsecond => primitive!(DurationMicrosecondType, temporal_i64(*unit)),
            TimeUnit::Nanosecond => primitive!(DurationNanosecondType, temporal_i64(*unit)),
            _ => return Err(unsupported(dtype, "invalid duration64 unit")),
        },
        DataType::Interval(TimeUnit::YearMonth) => {
            primitive!(IntervalYearMonthType, interval_year_month)
        }
        DataType::Interval(TimeUnit::DayTime) => {
            primitive!(IntervalDayTimeType, interval_day_time)
        }
        DataType::Interval(TimeUnit::MonthDayNano) => {
            primitive!(IntervalMonthDayNanoType, interval_month_day_nano)
        }
        DataType::Interval(_) => return Err(unsupported(dtype, "invalid interval layout")),
        crate::bytes_dtypes!() => {
            bytes_array(dtype.bytes_parameters().expect("a byte leaf"), values)?
        }
        crate::string_dtypes!() => {
            string_array(dtype.string_parameters().expect("a string leaf"), values)?
        }
        DataType::Country => code_array::<COUNTRY_WIDTH>(dtype, values)?,
        DataType::Ccy => code_array::<CCY_WIDTH>(dtype, values)?,
        DataType::Mic => code_array::<MIC_WIDTH>(dtype, values)?,
        DataType::Cfi => code_array::<CFI_WIDTH>(dtype, values)?,
        DataType::Isin => code_array::<ISIN_WIDTH>(dtype, values)?,
        DataType::Cusip => code_array::<CUSIP_WIDTH>(dtype, values)?,
        DataType::Sedol => code_array::<SEDOL_WIDTH>(dtype, values)?,
        DataType::Bbg => code_array::<BBG_WIDTH>(dtype, values)?,
        DataType::Ric => code_array::<RIC_WIDTH>(dtype, values)?,
        DataType::Figi => code_array::<FIGI_WIDTH>(dtype, values)?,
        DataType::Side => code_array::<SIDE_WIDTH>(dtype, values)?,
        DataType::State => code_array::<STATE_WIDTH>(dtype, values)?,
        DataType::TimeInForce => code_array::<TIMEINFORCE_WIDTH>(dtype, values)?,
        DataType::Unit => code_array::<UNIT_WIDTH>(dtype, values)?,
        DataType::Uuid => uuid_array(values)?,
        DataType::Version => Arc::new(StringArray::from(
            values
                .iter()
                .map(|value| match value {
                    Scalar::Null => Ok(None),
                    Scalar::Version(version) => Ok(Some(version.to_string())),
                    other => Err(invalid_value("version", other.kind())),
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::Url => Arc::new(StringArray::from(
            values
                .iter()
                .map(|value| match value {
                    Scalar::Null => Ok(None),
                    Scalar::Url(url) => Ok(Some(url.to_string())),
                    other => Err(invalid_value("url", other.kind())),
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::Urn => Arc::new(StringArray::from(
            values
                .iter()
                .map(|value| match value {
                    Scalar::Null => Ok(None),
                    Scalar::Urn(urn) => Ok(Some(urn.to_string())),
                    other => Err(invalid_value("urn", other.kind())),
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        // Each canonical text datatype writes the one spelling its value
        // renders, so the column holds what the value says it is.
        DataType::Timezone => Arc::new(StringArray::from(
            values
                .iter()
                .map(|value| match value {
                    Scalar::Null => Ok(None),
                    Scalar::Timezone(zone) => Ok(Some(zone.as_str().to_owned())),
                    other => Err(invalid_value("timezone", other.kind())),
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::MimeType => Arc::new(StringArray::from(
            values
                .iter()
                .map(|value| match value {
                    Scalar::Null => Ok(None),
                    Scalar::MimeType(mime) => Ok(Some(mime.as_str().to_owned())),
                    other => Err(invalid_value("mimetype", other.kind())),
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::MediaType => Arc::new(StringArray::from(
            values
                .iter()
                .map(|value| match value {
                    Scalar::Null => Ok(None),
                    Scalar::MediaType(media) => Ok(Some(media.to_string())),
                    other => Err(invalid_value("mediatype", other.kind())),
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::Serie(child) => list_array::<i32>(child, arrow_type, values)?,
        DataType::SerieView(child) => list_view_array::<i32>(child, arrow_type, values)?,
        DataType::FixedSizeSerie(child, size) => {
            fixed_size_list_array(child, arrow_type, *size, values)?
        }
        DataType::LargeSerie(child) => list_array::<i64>(child, arrow_type, values)?,
        DataType::LargeSerieView(child) => list_view_array::<i64>(child, arrow_type, values)?,
        DataType::Struct(fields) => struct_array(fields, arrow_type, values)?,
        DataType::Union(fields, mode) => union_array(fields, *mode, values)?,
        DataType::Dictionary(dictionary) => dictionary_array(dictionary, values)?,
        DataType::Decimal32 { scale, .. } => {
            primitive!(Decimal32Type, |value: &Scalar| i32::try_from(
                unscaled_i128(value, *scale)?
            )
            .map_err(|_| invalid_value("decimal32", value.kind())))
        }
        DataType::Decimal64 { scale, .. } => {
            primitive!(Decimal64Type, |value: &Scalar| i64::try_from(
                unscaled_i128(value, *scale)?
            )
            .map_err(|_| invalid_value("decimal64", value.kind())))
        }
        DataType::Decimal128 { scale, .. } => {
            primitive!(Decimal128Type, |value: &Scalar| unscaled_i128(
                value, *scale
            ))
        }
        DataType::Decimal256 { scale, .. } => {
            primitive!(Decimal256Type, |value: &Scalar| decimal256(value, *scale))
        }
        DataType::Decimal => {
            primitive!(Decimal128Type, |value: &Scalar| unscaled_i128(
                value,
                crate::Decimal::SCALE
            ))
        }
        DataType::BigDecimal => {
            primitive!(Decimal256Type, |value: &Scalar| decimal256(
                value,
                crate::BigDecimal::SCALE
            ))
        }
        map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
            let map = &map_dtype
                .as_mapping()
                .expect("the variant was just matched");
            map_array(map, arrow_type, values)?
        }
        DataType::RunEndEncoded(encoded) => run_array(encoded, arrow_type, values)?,
        // A geospatial value *is* its WKB payload, so the array is the bytes;
        // both the canonical `Geospatial` spelling and plain bytes build it.
        DataType::Geometry(_) | DataType::Geography(_) => Arc::new(BinaryArray::from(
            values
                .iter()
                .map(|value| optional_wkb(value))
                .collect::<Result<Vec<_>>>()?,
        )),
        // A variant value crosses this boundary as the two binaries the
        // encoding is, `metadata` and `value`. A bare null is an absent cell;
        // an encoded variant null remains present.
        DataType::Variant => {
            let native_count = values
                .iter()
                .filter(|value| !matches!(value, Scalar::Variant(_) | Scalar::Null))
                .count();
            let mut native = Vec::with_capacity(native_count);
            let mut metadata_bytes = 0_usize;
            let mut value_bytes = 0_usize;
            for value in values {
                let variant = match value {
                    Scalar::Null => continue,
                    Scalar::Variant(held) => held,
                    held => {
                        native.push(crate::Variant::encode(held)?);
                        native.last().expect("the encoded variant was retained")
                    }
                };
                metadata_bytes = metadata_bytes
                    .checked_add(variant.metadata().len())
                    .ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "variant metadata payload exceeds this address space".to_owned(),
                        )
                    })?;
                value_bytes = value_bytes
                    .checked_add(variant.value().len())
                    .ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "variant value payload exceeds this address space".to_owned(),
                        )
                    })?;
            }
            i32::try_from(metadata_bytes).map_err(|_| {
                invalid_value("a variant metadata column within int32", metadata_bytes)
            })?;
            i32::try_from(value_bytes)
                .map_err(|_| invalid_value("a variant value column within int32", value_bytes))?;

            let mut metadata = BinaryBuilder::with_capacity(values.len(), metadata_bytes);
            let mut payloads = BinaryBuilder::with_capacity(values.len(), value_bytes);
            let mut native = native.iter();
            let mut validity = NullBufferBuilder::new(values.len());
            for value in values {
                if matches!(value, Scalar::Null) {
                    metadata.append_value([]);
                    payloads.append_value([]);
                    validity.append_null();
                    continue;
                }
                validity.append_non_null();
                // Already encoded values lend their two buffers directly to
                // Arrow's final builders. A native fallback was encoded once
                // in the sizing pass and is retained until both copies land.
                let variant = match value {
                    Scalar::Variant(held) => held,
                    _ => native.next().expect("every native value was encoded once"),
                };
                metadata.append_value(variant.metadata());
                payloads.append_value(variant.value());
            }
            Arc::new(StructArray::new(
                crate::variant_fields(),
                vec![
                    Arc::new(metadata.finish()) as ArrayRef,
                    Arc::new(payloads.finish()) as ArrayRef,
                ],
                validity.finish(),
            ))
        }
    };
    Ok(array)
}

// ------------------------------------------------------------------------
// Cell readings: one slot of a typed buffer read as the value its field
// declares. A column leaf resolves its reading from its field once, where it
// lands, and calls it per cell with the native slot it already holds; the
// codec below reads through the same functions after its own downcast, so
// each reading has one owner.
// ------------------------------------------------------------------------

/// How one fixed-width slot reads as its field's value.
pub(crate) type Reading<N> = fn(&DataType, N) -> Result<Scalar>;

/// How one run of bytes - text or binary storage - reads as its field's value.
pub(crate) type RunReading<N> = fn(&DataType, &N) -> Result<Scalar>;

/// The field disagreeing with the reading its column resolved: this module
/// disagreeing with itself, since the landing chose the reading from the
/// field.
fn misread(dtype: &DataType, reading: &'static str) -> Error {
    unsupported(dtype, reading)
}

/// A slot whose native value is the datatype's whole value.
pub(crate) fn read_native<N: Into<Scalar>>(_: &DataType, value: N) -> Result<Scalar> {
    Ok(value.into())
}

pub(crate) fn read_decimal32(dtype: &DataType, value: i32) -> Result<Scalar> {
    match dtype {
        DataType::Decimal32 { scale, .. } => {
            Ok(Scalar::Decimal32(crate::Decimal32::new(value, *scale)))
        }
        _ => Err(misread(dtype, "a decimal32 slot")),
    }
}

pub(crate) fn read_decimal64(dtype: &DataType, value: i64) -> Result<Scalar> {
    match dtype {
        DataType::Decimal64 { scale, .. } => {
            Ok(Scalar::Decimal64(crate::Decimal64::new(value, *scale)))
        }
        _ => Err(misread(dtype, "a decimal64 slot")),
    }
}

pub(crate) fn read_decimal128(dtype: &DataType, value: i128) -> Result<Scalar> {
    match dtype {
        DataType::Decimal128 { scale, .. } => Ok(Scalar::d128(value, *scale)),
        // The fixed leaf reads its own value off the same slot; a coefficient
        // past thirty-eight digits is one the storage held and the datatype
        // does not.
        DataType::Decimal => crate::Decimal::from_units(value)
            .map(Scalar::Decimal)
            .ok_or_else(|| misread(dtype, "a decimal128 slot within 38 digits")),
        _ => Err(misread(dtype, "a decimal128 slot")),
    }
}

pub(crate) fn read_decimal256(dtype: &DataType, value: ArrowI256) -> Result<Scalar> {
    match dtype {
        DataType::Decimal256 { scale, .. } => Ok(Scalar::d256(
            i256::from_le_bytes(value.to_le_bytes()),
            *scale,
        )),
        DataType::BigDecimal => {
            crate::BigDecimal::from_units(i256::from_le_bytes(value.to_le_bytes()))
                .map(Scalar::BigDecimal)
                .ok_or_else(|| misread(dtype, "a decimal256 slot within 76 digits"))
        }
        _ => Err(misread(dtype, "a decimal256 slot")),
    }
}

pub(crate) fn read_date32(_: &DataType, value: i32) -> Result<Scalar> {
    Ok(Scalar::date32(value))
}

pub(crate) fn read_date64(_: &DataType, value: i64) -> Result<Scalar> {
    Ok(Scalar::date64(value))
}

pub(crate) fn read_time32(dtype: &DataType, value: i32) -> Result<Scalar> {
    match dtype {
        DataType::Time32(unit) => Ok(Scalar::time32(value, *unit, Timezone::NAIVE)?),
        _ => Err(misread(dtype, "a time32 slot")),
    }
}

pub(crate) fn read_time64(dtype: &DataType, value: i64) -> Result<Scalar> {
    match dtype {
        DataType::Time64(unit) => Ok(Scalar::time64(value, *unit, Timezone::NAIVE)?),
        _ => Err(misread(dtype, "a time64 slot")),
    }
}

pub(crate) fn read_datetime(dtype: &DataType, value: i64) -> Result<Scalar> {
    match dtype {
        DataType::DateTime64 { unit, timezone } => Ok(Scalar::datetime64(value, *unit, *timezone)?),
        _ => Err(misread(dtype, "a datetime slot")),
    }
}

/// A `duration32` slot: Arrow lays out one duration width, so the count is
/// narrowed back to the width the field declares.
pub(crate) fn read_duration32(dtype: &DataType, value: i64) -> Result<Scalar> {
    match dtype {
        DataType::Duration32(unit) => {
            let count = i32::try_from(value).map_err(|_| invalid_value("duration32", value))?;
            Ok(Scalar::duration32(count, *unit)?)
        }
        _ => Err(misread(dtype, "a duration32 slot")),
    }
}

pub(crate) fn read_duration64(dtype: &DataType, value: i64) -> Result<Scalar> {
    match dtype {
        DataType::Duration64(unit) => Ok(Scalar::duration64(value, *unit)?),
        _ => Err(misread(dtype, "a duration64 slot")),
    }
}

/// The reading a duration column takes, at the width its field declares.
///
/// # Errors
///
/// Returns an error for a field that declares no duration.
pub(crate) fn duration_reading(dtype: &DataType) -> Result<Reading<i64>> {
    match dtype {
        DataType::Duration32(_) => Ok(read_duration32),
        DataType::Duration64(_) => Ok(read_duration64),
        _ => Err(misread(dtype, "no duration slot holds it")),
    }
}

pub(crate) fn read_year_month(_: &DataType, months: i32) -> Result<Scalar> {
    Ok(Scalar::Interval(crate::Interval::new(
        months,
        0,
        0,
        TimeUnit::YearMonth,
    )?))
}

pub(crate) fn read_day_time(_: &DataType, value: IntervalDayTime) -> Result<Scalar> {
    Ok(Scalar::Interval(crate::Interval::new(
        0,
        value.days,
        i64::from(value.milliseconds) * 1_000_000,
        TimeUnit::DayTime,
    )?))
}

pub(crate) fn read_month_day_nano(_: &DataType, value: IntervalMonthDayNano) -> Result<Scalar> {
    Ok(Scalar::Interval(crate::Interval::new(
        value.months,
        value.days,
        value.nanoseconds,
        TimeUnit::MonthDayNano,
    )?))
}

/// The text a registered code's column holds, checked once more at the
/// width its own standard fixes.
macro_rules! read_code {
    ($($name:ident => $code:ident),+ $(,)?) => {$(
        fn $name(dtype: &DataType, cell: &str) -> Result<Scalar> {
            Ok(Scalar::$code(crate::$code::new(code_cell_text(dtype, cell.as_bytes())?)?))
        }
    )+};
}

read_code!(
    read_country => Country,
    read_ccy => Ccy,
    read_mic => Mic,
    read_cfi => Cfi,
    read_isin => Isin,
    read_cusip => Cusip,
    read_sedol => Sedol,
    read_bbg => Bbg,
    read_ric => Ric,
    read_figi => Figi,
    read_side => Side,
    read_state => State,
    read_time_in_force => TimeInForce,
    read_unit => Unit,
);

/// Emit one reader per unnumbered leaf in text or binary storage.
///
/// The storage was validated when it was written, so the cell is adopted as
/// it stands, and the reader is chosen once per run: a cell pays for no leaf
/// dispatch at all.
macro_rules! leaf_readers {
    ($cell:ty, $held:ident: $($reader:ident => $leaf:ident,)+) => {
        $(
            fn $reader(_: &DataType, cell: &$cell) -> Result<Scalar> {
                Ok(Scalar::$leaf($held::from(cell)))
            }
        )+
    };
}

leaf_readers!(
    str, Str:
    read_utf8 => Utf8String,
    read_large_utf8 => LargeUtf8String,
    read_utf8_view => Utf8StringView,
    read_large_utf8_view => LargeUtf8StringView,
    read_ascii => AsciiString,
    read_large_ascii => LargeAsciiString,
    read_ascii_view => AsciiStringView,
    read_large_ascii_view => LargeAsciiStringView,
);

leaf_readers!(
    [u8], Bytes:
    read_binary => Binary,
    read_large_binary => LargeBinary,
    read_binary_view => BinaryView,
    read_large_binary_view => LargeBinaryView,
);

/// A numbered leaf in text storage - a sized one - reads its number from
/// the column once per cell, and adopts the cell as it stands.
fn read_numbered_text(dtype: &DataType, cell: &str) -> Result<Scalar> {
    match dtype.string_parameters() {
        Some(leaf) => Ok(leaf.adopt(Str::from(cell))),
        None => Err(misread(dtype, "a text run")),
    }
}

fn read_version(_: &DataType, cell: &str) -> Result<Scalar> {
    Ok(Scalar::Version(cell.parse().map_err(Error::from)?))
}

fn read_url(_: &DataType, cell: &str) -> Result<Scalar> {
    Ok(Scalar::Url(Arc::new(
        crate::Url::from_str(cell).map_err(Error::from)?,
    )))
}

fn read_urn(_: &DataType, cell: &str) -> Result<Scalar> {
    Ok(Scalar::Urn(Arc::new(
        crate::Urn::from_str(cell).map_err(Error::from)?,
    )))
}

fn read_timezone(_: &DataType, cell: &str) -> Result<Scalar> {
    Ok(Scalar::Timezone(
        crate::Timezone::from_str(cell).map_err(Error::from)?,
    ))
}

fn read_mime_type(_: &DataType, cell: &str) -> Result<Scalar> {
    Ok(Scalar::MimeType(
        crate::MimeType::from_str(cell).map_err(Error::from)?,
    ))
}

fn read_media_type(_: &DataType, cell: &str) -> Result<Scalar> {
    Ok(Scalar::from(
        crate::MediaType::from_str(cell).map_err(Error::from)?,
    ))
}

/// The reading a column in text storage - `utf8`, `large_utf8`,
/// `utf8_view` - takes each run through.
///
/// # Errors
///
/// Returns an error for a datatype no text storage holds.
pub(crate) fn text_reading(dtype: &DataType) -> Result<RunReading<str>> {
    Ok(match dtype {
        DataType::Utf8String => read_utf8,
        DataType::LargeUtf8String => read_large_utf8,
        DataType::Utf8StringView => read_utf8_view,
        DataType::LargeUtf8StringView => read_large_utf8_view,
        DataType::AsciiString => read_ascii,
        DataType::LargeAsciiString => read_large_ascii,
        DataType::AsciiStringView => read_ascii_view,
        DataType::LargeAsciiStringView => read_large_ascii_view,
        DataType::SizedUtf8String(_) | DataType::SizedAsciiString(_) => read_numbered_text,
        DataType::Country => read_country,
        DataType::Ccy => read_ccy,
        DataType::Mic => read_mic,
        DataType::Cfi => read_cfi,
        DataType::Isin => read_isin,
        DataType::Cusip => read_cusip,
        DataType::Sedol => read_sedol,
        DataType::Bbg => read_bbg,
        DataType::Ric => read_ric,
        DataType::Figi => read_figi,
        DataType::Side => read_side,
        DataType::State => read_state,
        DataType::TimeInForce => read_time_in_force,
        DataType::Unit => read_unit,
        DataType::Version => read_version,
        DataType::Url => read_url,
        DataType::Urn => read_urn,
        DataType::Timezone => read_timezone,
        DataType::MimeType => read_mime_type,
        DataType::MediaType => read_media_type,
        _ => return Err(misread(dtype, "no text storage holds it")),
    })
}

/// The cell is the column's own storage, so it is adopted as it stands: a
/// short payload is copied inline and a long one is shared once, with no
/// `Vec` on the way. The numbered leaves read their number from the column.
fn read_bytes(dtype: &DataType, cell: &[u8]) -> Result<Scalar> {
    match dtype.bytes_parameters() {
        Some(leaf) => Ok(leaf.adopt(Bytes::from(cell))),
        None => Err(misread(dtype, "a byte run")),
    }
}

/// Binary storage goes through [`StringType::scalar_from_bytes`], the one
/// door bytes take into a string value: a fixed slot is trimmed of its
/// padding, and a legacy charset is transcribed rather than refused.
fn read_binary_string(dtype: &DataType, cell: &[u8]) -> Result<Scalar> {
    match dtype.string_parameters() {
        Some(leaf) => Ok(leaf.scalar_from_bytes(cell)?),
        None => Err(misread(dtype, "a string's byte run")),
    }
}

/// An identifier reads back as its exact packed scalar leaf.
fn read_uuid(_: &DataType, cell: &[u8]) -> Result<Scalar> {
    Ok(Scalar::Uuid(crate::Uuid::new(u128::from_be_bytes(
        uuid_parse(cell)?,
    ))))
}

/// A geospatial column reads back in its canonical value spelling.
fn read_geometry(_: &DataType, cell: &[u8]) -> Result<Scalar> {
    Ok(Scalar::Geometry(crate::Geometry::new(Arc::<[u8]>::from(
        cell,
    ))?))
}

fn read_geography(_: &DataType, cell: &[u8]) -> Result<Scalar> {
    Ok(Scalar::Geography(crate::Geography::new(
        Arc::<[u8]>::from(cell),
    )?))
}

/// The reading a column in binary storage - `binary`, `large_binary`,
/// `binary_view`, `fixed_size_binary` - takes each run through.
///
/// # Errors
///
/// Returns an error for a datatype no binary storage holds.
pub(crate) fn binary_reading(dtype: &DataType) -> Result<RunReading<[u8]>> {
    Ok(match dtype {
        DataType::Binary => read_binary,
        DataType::LargeBinary => read_large_binary,
        DataType::BinaryView => read_binary_view,
        DataType::LargeBinaryView => read_large_binary_view,
        DataType::FixedBinary(_) | DataType::SizedBinary(_) => read_bytes,
        crate::string_dtypes!() => read_binary_string,
        DataType::Uuid => read_uuid,
        DataType::Geometry(_) => read_geometry,
        DataType::Geography(_) => read_geography,
        _ => return Err(misread(dtype, "no binary storage holds it")),
    })
}

#[allow(clippy::too_many_lines)]
pub(crate) fn value_from_array(
    dtype: &DataType,
    array: &dyn Array,
    index: usize,
) -> Result<Scalar> {
    if index >= array.len() {
        return Err(Error::IncompatibleSchema(format!(
            "array index {index} exceeds length {}",
            array.len()
        )));
    }
    if array.is_null(index) && !matches!(dtype, DataType::Union(..) | DataType::RunEndEncoded(_)) {
        return Ok(Scalar::Null);
    }
    // A leaf slot reads through the column's own reading, over the typed
    // array its layout downcasts to.
    macro_rules! cell {
        ($array:ty, $read:expr) => {
            $read(dtype, downcast::<$array>(array)?.value(index))?
        };
    }
    let value = match dtype {
        DataType::Null => Scalar::Null,
        DataType::Boolean => Scalar::from(downcast::<BooleanArray>(array)?.value(index)),
        DataType::Int8 => cell!(Int8Array, read_native),
        DataType::Int16 => cell!(Int16Array, read_native),
        DataType::Int32 => cell!(Int32Array, read_native),
        DataType::Int64 => cell!(Int64Array, read_native),
        DataType::UInt8 => cell!(UInt8Array, read_native),
        DataType::UInt16 => cell!(UInt16Array, read_native),
        DataType::UInt32 => cell!(UInt32Array, read_native),
        DataType::UInt64 => cell!(UInt64Array, read_native),
        DataType::Float16 => cell!(Float16Array, read_native),
        DataType::Float32 => cell!(Float32Array, read_native),
        DataType::Float64 => cell!(Float64Array, read_native),
        // Every temporal reads as its typed value: the count alone is not
        // the datum, the unit and zone are, and the typed spelling is what
        // serializes losslessly and compares across resolutions.
        DataType::DateTime64 { unit, .. } => match unit {
            TimeUnit::Second => cell!(TimestampSecondArray, read_datetime),
            TimeUnit::Millisecond => cell!(TimestampMillisecondArray, read_datetime),
            TimeUnit::Microsecond => cell!(TimestampMicrosecondArray, read_datetime),
            TimeUnit::Nanosecond => cell!(TimestampNanosecondArray, read_datetime),
            _ => return Err(unsupported(dtype, "invalid timestamp unit")),
        },
        DataType::Date32 => cell!(Date32Array, read_date32),
        DataType::Date64 => cell!(Date64Array, read_date64),
        DataType::Time32(unit) => match unit {
            TimeUnit::Second => cell!(Time32SecondArray, read_time32),
            TimeUnit::Millisecond => cell!(Time32MillisecondArray, read_time32),
            _ => return Err(unsupported(dtype, "invalid time32 unit")),
        },
        DataType::Time64(unit) => match unit {
            TimeUnit::Microsecond => cell!(Time64MicrosecondArray, read_time64),
            TimeUnit::Nanosecond => cell!(Time64NanosecondArray, read_time64),
            _ => return Err(unsupported(dtype, "invalid time64 unit")),
        },
        DataType::Duration32(unit) | DataType::Duration64(unit) => match unit {
            TimeUnit::Second => cell!(DurationSecondArray, duration_reading(dtype)?),
            TimeUnit::Millisecond => cell!(DurationMillisecondArray, duration_reading(dtype)?),
            TimeUnit::Microsecond => cell!(DurationMicrosecondArray, duration_reading(dtype)?),
            TimeUnit::Nanosecond => cell!(DurationNanosecondArray, duration_reading(dtype)?),
            _ => return Err(unsupported(dtype, "invalid duration unit")),
        },
        DataType::Interval(TimeUnit::YearMonth) => cell!(IntervalYearMonthArray, read_year_month),
        DataType::Interval(TimeUnit::DayTime) => cell!(IntervalDayTimeArray, read_day_time),
        DataType::Interval(TimeUnit::MonthDayNano) => {
            cell!(IntervalMonthDayNanoArray, read_month_day_nano)
        }
        DataType::Interval(_) => return Err(unsupported(dtype, "invalid interval layout")),
        crate::bytes_dtypes!() => {
            let leaf = dtype.bytes_parameters().expect("a byte leaf");
            leaf.adopt(Bytes::from(bytes_cell(leaf, array, index)?))
        }
        crate::string_dtypes!() => string_value(
            dtype.string_parameters().expect("a string leaf"),
            array,
            index,
        )?,
        DataType::Uuid => cell!(FixedSizeBinaryArray, read_uuid),
        // A code and every rendered text datatype is stored as one `utf8`
        // run.
        DataType::Version
        | DataType::Url
        | DataType::Urn
        | DataType::Timezone
        | DataType::MimeType
        | DataType::MediaType
        | DataType::Country
        | DataType::Ccy
        | DataType::Mic
        | DataType::Cfi
        | DataType::Isin
        | DataType::Cusip
        | DataType::Sedol
        | DataType::Bbg
        | DataType::Ric
        | DataType::Figi
        | DataType::Side
        | DataType::State
        | DataType::TimeInForce
        | DataType::Unit => cell!(StringArray, text_reading(dtype)?),
        DataType::Serie(child) => {
            list_value(child, downcast::<ListArray>(array)?.value(index).as_ref())?
        }
        DataType::SerieView(child) => list_value(
            child,
            downcast::<ListViewArray>(array)?.value(index).as_ref(),
        )?,
        DataType::FixedSizeSerie(child, _) => list_value(
            child,
            downcast::<FixedSizeListArray>(array)?.value(index).as_ref(),
        )?,
        DataType::LargeSerie(child) => list_value(
            child,
            downcast::<LargeListArray>(array)?.value(index).as_ref(),
        )?,
        DataType::LargeSerieView(child) => list_value(
            child,
            downcast::<LargeListViewArray>(array)?.value(index).as_ref(),
        )?,
        DataType::Struct(fields) => {
            let array = downcast::<StructArray>(array)?;
            // A downcast answers the layout and nothing else, and the zip
            // below reads children by position: a declaration naming fewer or
            // more children than the array stores would answer a value quietly
            // narrower than the column, and one naming the same children in
            // another order would answer the neighbour's value under this
            // one's name. Names fold the way every other lookup in the crate
            // folds them, so only a real disagreement refuses.
            let stored = array.fields();
            let aligned = fields.len() == stored.len()
                && fields
                    .iter()
                    .zip(stored.iter())
                    .all(|(field, child)| field.name().eq_ignore_ascii_case(child.name()));
            if !aligned {
                return Err(Error::IncompatibleSchema(format!(
                    "a struct of [{}] does not describe an Arrow struct array of [{}]",
                    fields
                        .iter()
                        .map(Field::name)
                        .collect::<Vec<_>>()
                        .join(", "),
                    stored
                        .iter()
                        .map(|child| child.name().as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                )));
            }
            let values = fields
                .iter()
                .zip(array.columns())
                .map(|(field, child)| value_from_array(field.dtype(), child.as_ref(), index))
                .collect::<Result<Vec<_>>>()?;
            Scalar::from_sequence(values)
        }
        DataType::Union(fields, _) => {
            let array = downcast::<UnionArray>(array)?;
            let type_id = array.type_id(index);
            let (_, field) = fields
                .iter()
                .find(|(candidate, _)| *candidate == type_id)
                .ok_or_else(|| {
                    Error::IncompatibleSchema(format!("unknown union type id {type_id}"))
                })?;
            let payload = value_from_array(
                field.dtype(),
                array.child(type_id).as_ref(),
                array.value_offset(index),
            )?;
            Scalar::from_sequence([Scalar::from(i64::from(type_id)), payload])
        }
        DataType::Dictionary(dictionary) => dictionary_value(dictionary, array, index)?,
        DataType::Decimal32 { .. } => cell!(Decimal32Array, read_decimal32),
        DataType::Decimal64 { .. } => cell!(Decimal64Array, read_decimal64),
        DataType::Decimal128 { .. } => cell!(Decimal128Array, read_decimal128),
        DataType::Decimal256 { .. } => cell!(Decimal256Array, read_decimal256),
        DataType::Decimal => cell!(Decimal128Array, read_decimal128),
        DataType::BigDecimal => cell!(Decimal256Array, read_decimal256),
        map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
            let map = &map_dtype
                .as_mapping()
                .expect("the variant was just matched");
            let entries = downcast::<MapArray>(array)?.value(index);
            let fields = map
                .entries()
                .dtype()
                .as_fields()
                .ok_or_else(|| unsupported(dtype, "map entries are not a struct"))?;
            let pairs = (0..entries.len())
                .map(|entry| {
                    Ok((
                        value_from_array(fields[0].dtype(), entries.column(0).as_ref(), entry)?,
                        value_from_array(fields[1].dtype(), entries.column(1).as_ref(), entry)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            Scalar::from_mapping(pairs)?
        }
        DataType::RunEndEncoded(encoded) => run_value(encoded, array, index)?,
        DataType::Geometry(_) => cell!(BinaryArray, read_geometry),
        DataType::Geography(_) => cell!(BinaryArray, read_geography),
        DataType::Variant => {
            let stored = downcast::<StructArray>(array)?;
            let metadata = variant_child(stored, crate::VARIANT_METADATA_FIELD, index)?;
            let payload = variant_child(stored, crate::VARIANT_VALUE_FIELD, index)?;
            Scalar::Variant(crate::Variant::new(metadata, payload)?)
        }
    };
    Ok(dtype.declared_layout(value))
}

fn nulls(validity: Vec<bool>) -> Option<NullBuffer> {
    validity.iter().any(|valid| !valid).then(|| validity.into())
}

/// One primitive column, its values written straight into the buffer it
/// publishes and its absence into the bitmap - sized once, with no vector
/// of options between the rows and the array, and no bitmap at all for a
/// column with nothing absent. `arrow_type` is the field's projection,
/// which names the datatype a physical storage carries - a decimal's
/// precision and scale, a timestamp's zone - where it is not the native
/// type's own.
fn primitive_array<T: ArrowPrimitiveType>(
    values: &[&Scalar],
    arrow_type: &ArrowDataType,
    conversion: impl Fn(&Scalar) -> Result<T::Native>,
) -> Result<ArrayRef> {
    let mut native = Vec::with_capacity(values.len());
    let mut validity = NullBufferBuilder::new(values.len());
    for value in values {
        if matches!(value, Scalar::Null) {
            native.push(T::Native::default());
            validity.append_null();
        } else {
            native.push(conversion(value)?);
            validity.append_non_null();
        }
    }
    let array = PrimitiveArray::<T>::try_new(ScalarBuffer::from(native), validity.finish())?;
    Ok(Arc::new(if arrow_type == &T::DATA_TYPE {
        array
    } else {
        array.with_data_type(arrow_type.clone())
    }))
}

/// One boolean column, written bit by bit into the two bitmaps it is.
fn boolean_array(values: &[&Scalar]) -> Result<ArrayRef> {
    let mut bits = BooleanBufferBuilder::new(values.len());
    let mut validity = NullBufferBuilder::new(values.len());
    for value in values {
        match optional_bool(value)? {
            Some(bit) => {
                bits.append(bit);
                validity.append_non_null();
            }
            None => {
                bits.append(false);
                validity.append_null();
            }
        }
    }
    Ok(Arc::new(BooleanArray::new(
        bits.finish(),
        validity.finish(),
    )))
}

trait Offset: arrow_array::OffsetSizeTrait + TryFrom<usize> {}
impl Offset for i32 {}
impl Offset for i64 {}

/// One list element's items: the rows to lay out, or the element's own
/// array where it is a column of the item field itself.
enum Items<'a> {
    Rows(Cow<'a, [Scalar]>),
    Column(ArrayRef),
}

impl Items<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Rows(rows) => rows.len(),
            Self::Column(array) => array.len(),
        }
    }
}

/// Each element's items read through the sequence door, `None` for a null
/// row; a column of the item field itself is kept as its array, because
/// its door proved every row and its buffers are the items.
fn list_items<'a>(
    child: &Field,
    values: &[&'a Scalar],
    what: &'static str,
) -> Result<Vec<Option<Items<'a>>>> {
    let mut items = Vec::with_capacity(values.len());
    for value in values {
        if matches!(value, Scalar::Null) {
            items.push(None);
            continue;
        }
        let serie = value
            .as_serie()
            .ok_or_else(|| invalid_value_kind(what, value))?;
        let held = crate::value::column_fits_item(serie, child)
            .then(|| serie.into_arrow_array())
            .flatten()
            .map_or_else(|| Items::Rows(serie.rows()), Items::Column);
        items.push(Some(held));
    }
    Ok(items)
}

/// The child array under a list, assembled in element order: runs of rows
/// are laid out together, a column's own array is appended as it is, and
/// the segments are joined once at the end - one build, and no join, where
/// every element lent rows.
struct ItemsArray<'a, 'f> {
    child: &'f Field,
    segments: Vec<ArrayRef>,
    pending: Vec<&'a Scalar>,
}

impl<'a, 'f> ItemsArray<'a, 'f> {
    const fn new(child: &'f Field) -> Self {
        Self {
            child,
            segments: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Reserve `slots` rows ahead of laying them out, refusing what memory
    /// cannot hold rather than aborting on it.
    fn reserve(&mut self, slots: usize, what: &'static str) -> Result<()> {
        self.pending
            .try_reserve_exact(slots)
            .map_err(|error| allocation_error(what, slots, &error))
    }

    fn rows(&mut self, rows: impl IntoIterator<Item = &'a Scalar>) {
        self.pending.extend(rows);
    }

    fn column(&mut self, array: &ArrayRef) -> Result<()> {
        self.lay_out_pending()?;
        self.segments.push(Arc::clone(array));
        Ok(())
    }

    fn lay_out_pending(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let segment = array_of_rows(self.child, &self.pending)?;
        self.pending.clear();
        self.segments.push(segment);
        Ok(())
    }

    fn finish(mut self) -> Result<ArrayRef> {
        if self.segments.is_empty() {
            return array_of_rows(self.child, &self.pending);
        }
        self.lay_out_pending()?;
        if let [segment] = &*self.segments {
            return Ok(Arc::clone(segment));
        }
        let segments = self
            .segments
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&dyn Array>>();
        Ok(arrow_select::concat::concat(&segments)?)
    }
}

type ListParts<O> = (Vec<O>, Vec<O>, ArrayRef, Option<NullBuffer>);

/// The offsets, the sizes - filled only where `with_sizes` says a view
/// wants them - the child array and the validity of a list column.
fn list_parts<O: Offset>(
    child: &Field,
    values: &[&Scalar],
    with_sizes: bool,
) -> Result<ListParts<O>> {
    let items = list_items(child, values, "a sequence for a serie column")?;
    let mut offsets = Vec::with_capacity(values.len() + 1);
    let mut sizes = Vec::with_capacity(if with_sizes { values.len() } else { 0 });
    let mut validity = Vec::with_capacity(values.len());
    let mut array = ItemsArray::new(child);
    // Every lent run is laid out from one vector, reserved once for all of
    // them rather than grown a doubling at a time.
    let lent = items
        .iter()
        .flatten()
        .filter_map(|held| match held {
            Items::Rows(rows) => Some(rows.len()),
            Items::Column(_) => None,
        })
        .sum::<usize>();
    array.reserve(lent, "serie child slots")?;
    let mut total = 0_usize;
    offsets.push(
        O::try_from(0).map_err(|_| invalid_value("a list offset within the offset type", 0))?,
    );
    for held in &items {
        let size = match held {
            None => {
                validity.push(false);
                0
            }
            Some(held) => {
                validity.push(true);
                match held {
                    Items::Rows(rows) => array.rows(rows.iter()),
                    Items::Column(column) => array.column(column)?,
                }
                held.len()
            }
        };
        if with_sizes {
            sizes.push(
                O::try_from(size)
                    .map_err(|_| invalid_value("a list size within the offset type", size))?,
            );
        }
        total += size;
        offsets.push(
            O::try_from(total)
                .map_err(|_| invalid_value("a list offset within the offset type", total))?,
        );
    }
    Ok((offsets, sizes, array.finish()?, nulls(validity)))
}

/// The item field an Arrow list layout carries, taken off the parent's own
/// projection rather than projected again.
fn list_item(arrow_type: &ArrowDataType) -> Result<&FieldRef> {
    match arrow_type {
        ArrowDataType::List(item)
        | ArrowDataType::LargeList(item)
        | ArrowDataType::ListView(item)
        | ArrowDataType::LargeListView(item)
        | ArrowDataType::FixedSizeList(item, _) => Ok(item),
        _ => Err(Error::internal("list_item::projection")),
    }
}

fn list_array<O: Offset>(
    child: &Field,
    arrow_type: &ArrowDataType,
    values: &[&Scalar],
) -> Result<ArrayRef> {
    let (offsets, _, child_array, nulls) = list_parts::<O>(child, values, false)?;
    // The offsets are already the layout's own width, refused where they
    // were computed if the width could not hold them.
    Ok(Arc::new(GenericListArray::<O>::try_new(
        Arc::clone(list_item(arrow_type)?),
        OffsetBuffer::new(ScalarBuffer::from(offsets)),
        child_array,
        nulls,
    )?))
}

fn list_view_array<O: Offset>(
    child: &Field,
    arrow_type: &ArrowDataType,
    values: &[&Scalar],
) -> Result<ArrayRef> {
    let (mut offsets, sizes, child_array, nulls) = list_parts::<O>(child, values, true)?;
    offsets.truncate(values.len());
    Ok(Arc::new(GenericListViewArray::<O>::try_new(
        Arc::clone(list_item(arrow_type)?),
        ScalarBuffer::from(offsets),
        ScalarBuffer::from(sizes),
        child_array,
        nulls,
    )?))
}

fn fixed_size_list_array(
    child: &Field,
    arrow_type: &ArrowDataType,
    size: i32,
    values: &[&Scalar],
) -> Result<ArrayRef> {
    let size_usize = usize::try_from(size)
        .map_err(|_| invalid_value("a fixed serie size within usize", size))?;
    let physical_len = values.len().checked_mul(size_usize).ok_or_else(|| {
        physical_limit_error("fixed-size-serie slots", values.len(), MAX_PHYSICAL_SLOTS)
    })?;
    let null_rows = values
        .iter()
        .filter(|value| matches!(value, Scalar::Null))
        .count();
    let hidden_rows = checked_physical_mul(
        null_rows,
        size_usize,
        "fixed-size-serie slots",
        MAX_PHYSICAL_SLOTS,
    )?;
    if hidden_rows != 0 {
        let mut budget = MaterializationBudget::default();
        budget.add_array(child.dtype(), hidden_rows)?;
    }

    let has_parent_null = null_rows != 0;
    let placeholder = has_parent_null
        .then(|| physical_placeholder_for_field(child))
        .transpose()?;
    let items = list_items(child, values, "a sequence for a fixed-size-serie column")?;
    let mut array = ItemsArray::new(child);
    array.reserve(physical_len, "fixed-size-serie child slots")?;
    let mut validity = Vec::with_capacity(values.len());
    for held in &items {
        let Some(held) = held else {
            validity.push(false);
            let placeholder = placeholder
                .as_ref()
                .ok_or_else(|| Error::internal("fixed_size_list_array::null_placeholder"))?;
            array.rows(std::iter::repeat_n(placeholder, size_usize));
            continue;
        };
        if held.len() != size_usize {
            return Err(invalid_value(
                &format!("a fixed serie of exactly {size_usize} items"),
                held.len(),
            ));
        }
        validity.push(true);
        match held {
            Items::Rows(rows) => array.rows(rows.iter()),
            Items::Column(column) => array.column(column)?,
        }
    }
    let child_array = array.finish()?;
    Ok(Arc::new(FixedSizeListArray::try_new_with_length(
        Arc::clone(list_item(arrow_type)?),
        size,
        child_array,
        nulls(validity),
        values.len(),
    )?))
}

fn struct_array(
    fields: &crate::StructType,
    arrow_type: &ArrowDataType,
    values: &[&Scalar],
) -> Result<ArrayRef> {
    let ArrowDataType::Struct(arrow_fields) = arrow_type else {
        return Err(Error::internal("struct_array::projection"));
    };
    let null_rows = values
        .iter()
        .filter(|value| matches!(value, Scalar::Null))
        .count();
    if null_rows != 0 {
        let mut budget = MaterializationBudget::default();
        for field in fields {
            budget.add_array(field.dtype(), null_rows)?;
        }
    }
    let has_parent_null = null_rows != 0;
    let mut rows: Vec<Option<Cow<'_, [Scalar]>>> = Vec::with_capacity(values.len());
    let mut validity = NullBufferBuilder::new(values.len());
    for value in values {
        if matches!(value, Scalar::Null) {
            rows.push(None);
            validity.append_null();
            continue;
        }
        let row = value
            .sequence_rows()
            .ok_or_else(|| invalid_value_kind("a sequence for a struct column", value))?;
        rows.push(Some(row));
        validity.append_non_null();
    }
    if fields.is_empty() {
        return Ok(Arc::new(StructArray::new_empty_fields(
            values.len(),
            validity.finish(),
        )));
    }
    // A hidden slot under an absent row is null in every layout that owns
    // a bitmap; only a union and a run-end encoding, which own none, need
    // a physical filler built for them.
    let null = Scalar::Null;
    let placeholders: Vec<Option<Scalar>> = if has_parent_null {
        fields
            .iter()
            .map(|field| {
                matches!(
                    field.dtype(),
                    DataType::Union(..) | DataType::RunEndEncoded(_)
                )
                .then(|| physical_placeholder_for_field(field))
                .transpose()
            })
            .collect::<Result<_>>()?
    } else {
        Vec::new()
    };
    // One scratch of cell references, sized once and refilled per column.
    let mut cells: Vec<&Scalar> = Vec::with_capacity(values.len());
    let mut columns = Vec::with_capacity(fields.len());
    for (column, field) in fields.iter().enumerate() {
        cells.clear();
        let placeholder = placeholders
            .get(column)
            .and_then(Option::as_ref)
            .unwrap_or(&null);
        for (row, value) in rows.iter().zip(values) {
            cells.push(match row {
                None => placeholder,
                Some(row) => row
                    .get(column)
                    .ok_or_else(|| invalid_value_kind("a sequence for a struct column", value))?,
            });
        }
        columns.push(array_of_rows(field, &cells)?);
    }
    Ok(Arc::new(StructArray::try_new_with_length(
        arrow_fields.clone(),
        columns,
        validity.finish(),
        values.len(),
    )?))
}

#[allow(clippy::too_many_lines)]
fn union_array(
    fields: &crate::UnionFields,
    mode: UnionMode,
    values: &[&Scalar],
) -> Result<ArrayRef> {
    if matches!(mode, UnionMode::Sparse) {
        // Sparse layout forces every child to the parent length, including
        // the selected child. Bound the complete aggregate before allocating
        // parsing vectors or constructing the first inactive placeholder.
        let mut budget = MaterializationBudget::default();
        for (_, field) in fields {
            budget.add_array(field.dtype(), values.len())?;
        }
    }

    let mut type_ids = Vec::new();
    type_ids
        .try_reserve_exact(values.len())
        .map_err(|error| allocation_error("union type IDs", values.len(), &error))?;
    let mut selections = Vec::new();
    selections
        .try_reserve_exact(values.len())
        .map_err(|error| allocation_error("union selections", values.len(), &error))?;
    let mut active_counts = vec![0_usize; fields.len()];
    let pairs = values
        .iter()
        .map(|value| {
            value
                .sequence_rows()
                .ok_or_else(|| invalid_value_kind("a union [type_id, payload] sequence", value))
        })
        .collect::<Result<Vec<_>>>()?;
    for pair in &pairs {
        let [type_id, payload] = &**pair else {
            return Err(invalid_value(
                "a union [type_id, payload] sequence of exactly 2 items",
                pair.len(),
            ));
        };
        let type_id = i8::try_from(exact_i128(type_id)?).map_err(|_| {
            invalid_value(
                "a union type id within int8",
                exact_i128(type_id).unwrap_or_default(),
            )
        })?;
        let position = fields
            .iter()
            .position(|(candidate, _)| candidate == type_id)
            .ok_or_else(|| invalid_value("a declared union type id", type_id))?;
        type_ids.push(type_id);
        selections.push((position, payload));
        active_counts[position] = active_counts[position].checked_add(1).ok_or_else(|| {
            physical_limit_error("union child rows", values.len(), MAX_PHYSICAL_SLOTS)
        })?;
    }

    let placeholders = fields
        .iter()
        .enumerate()
        .map(|(index, (_, field))| {
            if matches!(mode, UnionMode::Sparse) && active_counts[index] < values.len() {
                physical_placeholder_for_field(field).map(Some)
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let mut children = (0..fields.len())
        .map(|_| Vec::<&Scalar>::new())
        .collect::<Vec<_>>();
    for (index, child) in children.iter_mut().enumerate() {
        let capacity = match mode {
            UnionMode::Dense => active_counts[index],
            UnionMode::Sparse => values.len(),
        };
        child
            .try_reserve_exact(capacity)
            .map_err(|error| allocation_error("union child slots", capacity, &error))?;
    }
    let mut offsets = Vec::new();
    if matches!(mode, UnionMode::Dense) {
        offsets
            .try_reserve_exact(values.len())
            .map_err(|error| allocation_error("union offsets", values.len(), &error))?;
    }
    for (position, payload) in selections {
        match mode {
            UnionMode::Dense => {
                offsets.push(i32::try_from(children[position].len()).map_err(|_| {
                    invalid_value("a union offset within int32", children[position].len())
                })?);
                children[position].push(payload);
            }
            UnionMode::Sparse => {
                for (index, child) in children.iter_mut().enumerate() {
                    if index == position {
                        child.push(payload);
                    } else {
                        child.push(
                            placeholders[index].as_ref().ok_or_else(|| {
                                Error::internal("union_array::sparse_placeholder")
                            })?,
                        );
                    }
                }
            }
        }
    }
    let child_arrays = fields
        .iter()
        .zip(children)
        .map(|((_, field), values)| array_of_rows(field, &values))
        .collect::<Result<Vec<_>>>()?;
    let ArrowDataType::Union(arrow_fields, _) = fields_to_arrow_union(fields, mode)? else {
        return Err(Error::internal("union_array::union_fields"));
    };
    Ok(Arc::new(UnionArray::try_new(
        arrow_fields,
        ScalarBuffer::from(type_ids),
        matches!(mode, UnionMode::Dense).then(|| ScalarBuffer::from(offsets)),
        child_arrays,
    )?))
}

fn fields_to_arrow_union(fields: &crate::UnionFields, mode: UnionMode) -> Result<ArrowDataType> {
    let dtype = DataType::union(fields.iter().map(|(id, field)| (id, field.clone())), mode)?;
    dtype.into_arrow_datatype().map_err(Into::into)
}

fn dictionary_array(dictionary: &crate::DictionaryType, values: &[&Scalar]) -> Result<ArrayRef> {
    let unique = values
        .iter()
        .filter(|value| !matches!(value, Scalar::Null))
        .map(|value| (*value).clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let value_field = Field::new("dictionary", dictionary.value().clone(), true);
    let value_refs = unique.iter().collect::<Vec<_>>();
    let dictionary_values = array_of_rows(&value_field, &value_refs)?;
    macro_rules! dictionary {
        ($key:ty, $native:ty) => {{
            let keys = values
                .iter()
                .map(|value| {
                    if matches!(value, Scalar::Null) {
                        Ok(None)
                    } else {
                        let index = unique
                            .binary_search(value)
                            .map_err(|_| Error::internal("dictionary_array::value_index"))?;
                        <$native>::try_from(index)
                            .map(Some)
                            .map_err(|_| Error::internal("dictionary_array::key_capacity"))
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Arc::new(DictionaryArray::<$key>::try_new(
                PrimitiveArray::<$key>::from(keys),
                dictionary_values,
            )?) as ArrayRef)
        }};
    }
    match dictionary.key() {
        DataType::Int8 => dictionary!(Int8Type, i8),
        DataType::Int16 => dictionary!(Int16Type, i16),
        DataType::Int32 => dictionary!(Int32Type, i32),
        DataType::Int64 => dictionary!(Int64Type, i64),
        DataType::UInt8 => dictionary!(UInt8Type, u8),
        DataType::UInt16 => dictionary!(UInt16Type, u16),
        DataType::UInt32 => dictionary!(UInt32Type, u32),
        DataType::UInt64 => dictionary!(UInt64Type, u64),
        key => Err(unsupported(
            key,
            format!(
                "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        )),
    }
}

fn map_array(
    map: &crate::MappingType,
    arrow_type: &ArrowDataType,
    values: &[&Scalar],
) -> Result<ArrayRef> {
    let ArrowDataType::Map(entries_field, keys_sorted) = arrow_type else {
        return Err(Error::internal("map_array::projection"));
    };
    let ArrowDataType::Struct(entry_fields) = entries_field.data_type() else {
        return Err(Error::internal("map_array::entries_projection"));
    };
    let mut offsets = Vec::with_capacity(values.len() + 1);
    let mut validity = Vec::with_capacity(values.len());
    let mut entries = Vec::new();
    offsets.push(0_i32);
    for value in values {
        if matches!(value, Scalar::Null) {
            validity.push(false);
        } else {
            validity.push(true);
            entries.extend(
                value
                    .as_mapping()
                    .ok_or_else(|| invalid_value_kind("a sequence of map entries", value))?
                    .iter(),
            );
        }
        offsets.push(
            i32::try_from(entries.len())
                .map_err(|_| invalid_value("a map offset within int32", entries.len()))?,
        );
    }
    let fields = map
        .entries()
        .dtype()
        .as_fields()
        .ok_or_else(|| Error::internal("map_array::entries_struct"))?;
    let keys = entries.iter().map(|(key, _)| key).collect::<Vec<_>>();
    let vals = entries.iter().map(|(_, value)| value).collect::<Vec<_>>();
    let entries_array = StructArray::try_new_with_length(
        entry_fields.clone(),
        vec![
            array_of_rows(&fields[0], &keys)?,
            array_of_rows(&fields[1], &vals)?,
        ],
        None,
        entries.len(),
    )?;
    Ok(Arc::new(MapArray::try_new(
        Arc::clone(entries_field),
        OffsetBuffer::new(ScalarBuffer::from(offsets)),
        entries_array,
        nulls(validity),
        *keys_sorted,
    )?))
}

fn run_array(
    encoded: &crate::RunEndEncodedType,
    arrow_type: &ArrowDataType,
    values: &[&Scalar],
) -> Result<ArrayRef> {
    let mut run_values = Vec::new();
    let mut run_ends = Vec::new();
    for (index, value) in values.iter().enumerate() {
        if run_values.last().is_none_or(|previous| *previous != *value) {
            run_values.push(*value);
            run_ends.push(index + 1);
        } else if let Some(run_end) = run_ends.last_mut() {
            *run_end = index + 1;
        }
    }
    let values_array = array_of_rows(encoded.values(), &run_values)?;
    macro_rules! run {
        ($key:ty, $array:ty) => {{
            let run_ends = run_ends
                .iter()
                .map(|value| {
                    <$key>::try_from(*value)
                        .map_err(|_| invalid_value("a run end within the run-end type", value))
                })
                .collect::<Result<Vec<_>>>()?;
            let array = <$array>::try_new(&PrimitiveArray::from(run_ends), values_array.as_ref())?;
            let data = array
                .to_data()
                .into_builder()
                .data_type(arrow_type.clone())
                .build()?;
            Ok(make_array(data))
        }};
    }
    match encoded.run_ends().dtype() {
        DataType::Int16 => run!(i16, Int16RunArray),
        DataType::Int32 => run!(i32, Int32RunArray),
        DataType::Int64 => run!(i64, Int64RunArray),
        dtype => Err(unsupported(dtype, "invalid run-end type")),
    }
}

pub(crate) fn physical_placeholder_for_field(field: &Field) -> Result<Scalar> {
    // These values occupy physically required slots hidden by a parent null
    // bitmap or an inactive sparse-union type ID. They need a valid physical
    // representation, not a logically inhabitable value: a required Null
    // grandchild is legal when an ancestor masks the entire slot.
    physical_placeholder(field.dtype())
}

fn physical_placeholder(dtype: &DataType) -> Result<Scalar> {
    match dtype {
        DataType::Union(fields, _) => {
            let (type_id, field) = physical_union_branch(dtype, fields)?;
            Ok(Scalar::from_sequence([
                Scalar::from(i64::from(type_id)),
                physical_placeholder(field.dtype())?,
            ]))
        }
        DataType::RunEndEncoded(encoded) => physical_placeholder(encoded.values().dtype()),
        // Every other Arrow layout owns a validity bitmap (or is Null
        // itself). Marking the hidden slot null lets that physical container
        // mask its own required descendants before its parent masks it in
        // turn, and avoids allocating a logical filler that is never visible.
        _ => Ok(Scalar::Null),
    }
}

fn list_value(field: &Field, array: &dyn Array) -> Result<Scalar> {
    (0..array.len())
        .map(|index| value_from_array(field.dtype(), array, index))
        .collect::<Result<Vec<_>>>()
        .map(Scalar::from_sequence)
}

fn dictionary_value(
    dictionary: &crate::DictionaryType,
    array: &dyn Array,
    index: usize,
) -> Result<Scalar> {
    macro_rules! dictionary {
        ($key:ty) => {{
            let array = downcast::<DictionaryArray<$key>>(array)?;
            if array.keys().is_null(index) {
                return Ok(Scalar::Null);
            }
            let key = usize::try_from(array.keys().value(index))
                .map_err(|_| Error::internal("dictionary_array::key_index"))?;
            value_from_array(dictionary.value(), array.values().as_ref(), key)
        }};
    }
    match dictionary.key() {
        DataType::Int8 => dictionary!(Int8Type),
        DataType::Int16 => dictionary!(Int16Type),
        DataType::Int32 => dictionary!(Int32Type),
        DataType::Int64 => dictionary!(Int64Type),
        DataType::UInt8 => dictionary!(UInt8Type),
        DataType::UInt16 => dictionary!(UInt16Type),
        DataType::UInt32 => dictionary!(UInt32Type),
        DataType::UInt64 => dictionary!(UInt64Type),
        key => Err(unsupported(key, "invalid dictionary key")),
    }
}

fn run_value(
    encoded: &crate::RunEndEncodedType,
    array: &dyn Array,
    index: usize,
) -> Result<Scalar> {
    macro_rules! run {
        ($key:ty, $array:ty) => {{
            let array = downcast::<$array>(array)?;
            value_from_array(
                encoded.values().dtype(),
                array.values().as_ref(),
                array.get_physical_index(index),
            )
        }};
    }
    match encoded.run_ends().dtype() {
        DataType::Int16 => run!(Int16Type, Int16RunArray),
        DataType::Int32 => run!(Int32Type, Int32RunArray),
        DataType::Int64 => run!(Int64Type, Int64RunArray),
        dtype => Err(unsupported(dtype, "invalid run-end type")),
    }
}

fn downcast<T: Array + 'static>(array: &dyn Array) -> Result<&T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "expected Arrow array {}, got {}",
            std::any::type_name::<T>(),
            array.data_type()
        ))
    })
}

/// One binary cell of a variant column's named child.
///
/// The child is found by name, never by position, which is what the
/// Parquet, Avro and ORC spellings of a variant all state; any of Arrow's
/// three binary layouts holds it, because a foreign writer chooses its own.
fn variant_child<'a>(stored: &'a StructArray, name: &str, index: usize) -> Result<&'a [u8]> {
    let child = stored.column_by_name(name).ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "expected a variant column with a {name:?} child, got {}",
            stored.data_type()
        ))
    })?;
    if child.is_null(index) {
        return Ok(&[]);
    }
    if let Some(binary) = child.as_any().downcast_ref::<BinaryArray>() {
        return Ok(binary.value(index));
    }
    if let Some(binary) = child.as_any().downcast_ref::<LargeBinaryArray>() {
        return Ok(binary.value(index));
    }
    if let Some(binary) = child.as_any().downcast_ref::<BinaryViewArray>() {
        return Ok(binary.value(index));
    }
    Err(Error::IncompatibleSchema(format!(
        "expected binary storage for a variant's {name:?} child, got {}",
        child.data_type()
    )))
}

fn exact_i128(value: &Scalar) -> Result<i128> {
    value
        .as_i128()
        .ok_or_else(|| invalid_value_kind("signed integer", value))
}

fn exact_u128(value: &Scalar) -> Result<u128> {
    value
        .as_u128()
        .ok_or_else(|| invalid_value_kind("unsigned integer", value))
}

fn signed_i32(value: &Scalar) -> Result<i32> {
    i32::try_from(exact_i128(value)?)
        .map_err(|_| invalid_value("int32", exact_i128(value).unwrap_or_default()))
}

fn signed_i64(value: &Scalar) -> Result<i64> {
    i64::try_from(exact_i128(value)?)
        .map_err(|_| invalid_value("int64", exact_i128(value).unwrap_or_default()))
}

/// Read the coefficient a decimal column of `scale` stores.
///
/// A decimal [`Scalar`] knows its own scale, so it is restated at the column's
/// scale and refused when that would drop a digit. A bare integer is already
/// the coefficient, which is how a decimal column has always been written and
/// what a caller who never built a decimal value still means.
fn unscaled_i128(value: &Scalar, scale: i8) -> Result<i128> {
    if value.is_decimal() {
        return value.decimal_unscaled_at(scale).ok_or_else(|| {
            invalid_value(
                &format!("a decimal representable at scale {scale}"),
                value.kind(),
            )
        });
    }
    exact_i128(value)
}

/// Build the reader for a temporal column of `unit` at 64-bit width.
///
/// A temporal value carries its own unit, so it is restated at the column's
/// unit and refused when that would drop a digit; anything else is read as the
/// physical count the column stores, which is what an integer already is.
fn temporal_i64(unit: TimeUnit) -> impl Fn(&Scalar) -> Result<i64> {
    move |value| match value.temporal_count_at(unit) {
        Some(count) => Ok(count),
        None if value.is_temporal() => Err(invalid_value(
            &format!("a temporal representable in {unit}"),
            value.kind(),
        )),
        None => signed_i64(value),
    }
}

/// Build the reader for a temporal column of `unit` at 32-bit width.
fn temporal_i32(unit: TimeUnit) -> impl Fn(&Scalar) -> Result<i32> {
    let wide = temporal_i64(unit);
    move |value| {
        let count = wide(value)?;
        i32::try_from(count).map_err(|_| invalid_value("int32", count))
    }
}

/// Read the day count a `Date32` column stores.
fn date_i32(value: &Scalar) -> Result<i32> {
    match value.temporal_count_at(TimeUnit::Day) {
        Some(days) => i32::try_from(days).map_err(|_| invalid_value("date32", days)),
        None => signed_i32(value),
    }
}

/// Read the whole-day milliseconds a `Date64` column stores.
fn date_i64(value: &Scalar) -> Result<i64> {
    match value.temporal_count_at(TimeUnit::Millisecond) {
        Some(milliseconds) => Ok(milliseconds),
        None => signed_i64(value),
    }
}

/// Read the bytes a binary column stores, refusing anything that is not bytes.
///
/// Reading through `as_bytes` alone turned every non-byte value into a null,
/// so a string written into a binary column disappeared instead of being
/// reported. A null is still a null; nothing else is silently one.
fn optional_bytes(value: &Scalar) -> Result<Option<&[u8]>> {
    match value {
        Scalar::Null => Ok(None),
        crate::bytes_scalars!(bytes) => Ok(Some(bytes.as_bytes())),
        _ => Err(invalid_value_kind("bytes", value)),
    }
}

/// Build one byte column in the layout it declares.
///
/// The storage is the layout, so every value's bytes go in as they are: a
/// maximum is the column's rule and was checked at the value door, and a
/// fixed width is exactly what each value must hold, which Arrow checks as
/// it builds the slots.
fn bytes_array(parameters: BytesType, values: &[&Scalar]) -> Result<ArrayRef> {
    parameters.validate()?;
    let mut cells = Vec::with_capacity(values.len());
    for value in values {
        cells.push(optional_bytes(value)?);
    }
    if let Some(width) = parameters.fixed() {
        let width =
            i32::try_from(width).map_err(|_| invalid_value("a byte width within i32", width))?;
        return Ok(Arc::new(
            FixedSizeBinaryArray::try_from_sparse_iter_with_size(cells.into_iter(), width)?,
        ));
    }
    Ok(match parameters {
        BytesType::LargeBinary => Arc::new(LargeBinaryArray::from(cells)),
        BytesType::BinaryView | BytesType::LargeBinaryView => {
            Arc::new(cells.into_iter().collect::<BinaryViewArray>())
        }
        // The fixed layout answered above: `validate` gave it its width.
        BytesType::Binary | BytesType::SizedBinary(_) | BytesType::FixedBinary(_) => {
            Arc::new(BinaryArray::from(cells))
        }
    })
}

/// One cell of a byte column where its layout stores it.
fn bytes_cell(parameters: BytesType, array: &dyn Array, index: usize) -> Result<&[u8]> {
    Ok(match parameters {
        BytesType::FixedBinary(_) => downcast::<FixedSizeBinaryArray>(array)?.value(index),
        BytesType::LargeBinary => downcast::<LargeBinaryArray>(array)?.value(index),
        BytesType::BinaryView | BytesType::LargeBinaryView => {
            downcast::<BinaryViewArray>(array)?.value(index)
        }
        // A maximum is the column's rule; the storage it fills is plain.
        BytesType::Binary | BytesType::SizedBinary(_) => {
            downcast::<BinaryArray>(array)?.value(index)
        }
    })
}

/// Build the sixteen-byte storage of a UUID column.
///
/// Every present value passes the one UUID rule and stores as its sixteen
/// bytes, so the array is exactly what the field reads back as a UUID.
fn uuid_array(values: &[&Scalar]) -> Result<ArrayRef> {
    let mut bytes = vec![0_u8; values.len() * 16];
    let mut validity = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        match value {
            Scalar::Uuid(uuid) => {
                bytes[index * 16..][..16].copy_from_slice(&uuid.into_bytes());
                validity.push(true);
            }
            _ if matches!(value, Scalar::Null) => validity.push(false),
            _ => match uuid_bytes(value) {
                Some(raw) => {
                    bytes[index * 16..][..16].copy_from_slice(&uuid_parse(raw)?);
                    validity.push(true);
                }
                None => return Err(invalid_value_kind("a UUID", value)),
            },
        }
    }
    Ok(Arc::new(FixedSizeBinaryArray::try_new(
        16,
        Buffer::from(bytes),
        nulls(validity),
    )?))
}

/// Build the text storage of a registered code column.
///
/// A value never outgrows `WIDTH`, which is a compile-time length, so the
/// payload is bounded before a byte of it is copied.
fn code_array<const WIDTH: usize>(dtype: &DataType, values: &[&Scalar]) -> Result<ArrayRef> {
    let mut builder = StringBuilder::with_capacity(values.len(), values.len() * WIDTH);
    for value in values {
        match ascii_bytes(value) {
            Some(raw) => builder.append_value(code_cell_text(dtype, raw)?),
            None if matches!(value, Scalar::Null) => builder.append_null(),
            None => return Err(invalid_value_kind("ASCII text", value)),
        }
    }
    Ok(Arc::new(builder.finish()))
}

/// Build the storage of one string column, in the charset it declares.
///
/// UTF-8 and US-ASCII text is stored as Arrow's own string layouts, because
/// that is what they hold. Every other charset is stored as the matching
/// *binary* layout: the bytes are not UTF-8, and an Arrow reader told
/// otherwise would read mojibake and call it text.
fn string_array(parameters: StringType, values: &[&Scalar]) -> Result<ArrayRef> {
    let charset = parameters.charset();
    // A fixed width pads into its slot; nothing else in this family does.
    if let Some(width) = parameters.fixed() {
        let slot = usize::try_from(width)
            .map_err(|_| invalid_value("a string width within usize", width))?;
        let cells = values
            .len()
            .checked_mul(slot)
            .ok_or_else(|| invalid_value("a string column within usize", width))?;
        let mut bytes = Vec::with_capacity(cells);
        let mut validity = Vec::with_capacity(values.len());
        for value in values {
            let start = bytes.len();
            match optional_str(value)? {
                Some(text) => {
                    charset.encode_into(text, &mut bytes)?;
                    let encoded = bytes.len() - start;
                    if encoded > slot {
                        return Err(invalid_value(
                            &format!("at most {slot} bytes of {charset}"),
                            encoded,
                        ));
                    }
                    validity.push(true);
                }
                None => validity.push(false),
            }
            bytes.resize(start + slot, 0);
        }
        let width =
            i32::try_from(width).map_err(|_| invalid_value("a string width within i32", width))?;
        return Ok(Arc::new(FixedSizeBinaryArray::try_new(
            width,
            Buffer::from(bytes),
            nulls(validity),
        )?));
    }
    if is_text_storage(parameters) {
        return utf8_array(parameters, values);
    }
    // The stored length is a property of the text and the charset, so the whole
    // payload is measured before a byte of it is built: one buffer sized once,
    // rather than a `Vec<u8>` per row - which `encode` would have had to build
    // even where it borrows, which is every all-ASCII cell - and then an Arrow
    // buffer that starts at a kilobyte and doubles its way up.
    let mut payload = 0_usize;
    for value in values {
        if let Some(text) = optional_str(value)? {
            payload = payload
                .checked_add(charset.encoded_len(text))
                .ok_or_else(|| invalid_value("a string column within usize", payload))?;
        }
    }
    let mut bytes = Vec::with_capacity(payload);
    let mut validity = Vec::with_capacity(values.len());
    let mut ends = Vec::with_capacity(values.len());
    for value in values {
        match optional_str(value)? {
            Some(text) => {
                charset.encode_into(text, &mut bytes)?;
                validity.push(true);
            }
            None => validity.push(false),
        }
        ends.push(bytes.len());
    }
    let nulls = nulls(validity);
    Ok(match (parameters.is_view(), parameters.is_large()) {
        // Arrow's view layout holds its own prefix per cell, so it is built
        // from the finished payload rather than from offsets.
        (true, _) => Arc::new(binary_view_from_parts(&ends, &bytes, nulls.as_ref())),
        (false, true) => Arc::new(binary_from_parts::<i64>(&ends, bytes, nulls)?),
        // A maximum is the column's rule; the storage it fills is plain.
        (false, false) => Arc::new(binary_from_parts::<i32>(&ends, bytes, nulls)?) as ArrayRef,
    })
}

/// Build one UTF-8 column straight into a builder sized for its whole payload.
///
/// The bytes a UTF-8 column stores are the characters it already holds, so the
/// payload can be measured before any of it is built and the array needs one
/// buffer rather than one that starts at a kilobyte and doubles - and no
/// intermediate `Vec` of borrowed cells to hand it.
fn utf8_array(parameters: StringType, values: &[&Scalar]) -> Result<ArrayRef> {
    let mut payload = 0_usize;
    for value in values {
        if let Some(text) = optional_str(value)? {
            payload = payload
                .checked_add(text.len())
                .ok_or_else(|| invalid_value("a string column within usize", payload))?;
        }
    }
    macro_rules! filled {
        ($builder:expr) => {{
            let mut builder = $builder;
            for value in values {
                match optional_str(value)? {
                    Some(text) => builder.append_value(text),
                    None => builder.append_null(),
                }
            }
            Arc::new(builder.finish()) as ArrayRef
        }};
    }
    Ok(match (parameters.is_view(), parameters.is_large()) {
        // Arrow's view layout carries a prefix per cell rather than offsets,
        // so it takes the row count and grows its own payload blocks.
        (true, _) => filled!(StringViewBuilder::with_capacity(values.len())),
        (false, true) => filled!(LargeStringBuilder::with_capacity(values.len(), payload)),
        // A maximum is the column's rule; the storage it fills is plain.
        (false, false) => filled!(StringBuilder::with_capacity(values.len(), payload)),
    })
}

/// One binary array from the payload and the end offset of every cell.
fn binary_from_parts<O: arrow_array::OffsetSizeTrait>(
    ends: &[usize],
    bytes: Vec<u8>,
    nulls: Option<NullBuffer>,
) -> Result<GenericBinaryArray<O>> {
    let mut offsets = Vec::with_capacity(ends.len() + 1);
    offsets.push(O::zero());
    for end in ends {
        offsets.push(
            O::from_usize(*end)
                .ok_or_else(|| invalid_value("a string column within its offset width", *end))?,
        );
    }
    Ok(GenericBinaryArray::try_new(
        arrow_buffer::OffsetBuffer::new(offsets.into()),
        arrow_buffer::Buffer::from_vec(bytes),
        nulls,
    )?)
}

/// One binary view array over the same payload.
fn binary_view_from_parts(
    ends: &[usize],
    bytes: &[u8],
    nulls: Option<&NullBuffer>,
) -> BinaryViewArray {
    let mut builder = arrow_array::builder::BinaryViewBuilder::with_capacity(ends.len());
    let mut start = 0;
    for (index, end) in ends.iter().enumerate() {
        let present = nulls.is_none_or(|nulls| nulls.is_valid(index));
        if present {
            builder.append_value(&bytes[start..*end]);
        } else {
            builder.append_null();
        }
        start = *end;
    }
    builder.finish()
}

/// Read one cell of a string column, under the leaf it declares: text
/// storage adopted as it stands, binary storage - a fixed slot, a legacy
/// charset - through [`StringType::scalar_from_bytes`].
fn string_value(parameters: StringType, array: &dyn Array, index: usize) -> Result<Scalar> {
    if parameters.is_fixed() {
        let cell = downcast::<FixedSizeBinaryArray>(array)?.value(index);
        return Ok(parameters.scalar_from_bytes(cell)?);
    }
    if is_text_storage(parameters) {
        let cell = match (parameters.is_view(), parameters.is_large()) {
            (true, _) => downcast::<StringViewArray>(array)?.value(index),
            (false, true) => downcast::<LargeStringArray>(array)?.value(index),
            // A maximum is the column's rule; the storage it fills is plain.
            (false, false) => downcast::<StringArray>(array)?.value(index),
        };
        return Ok(parameters.adopt(Str::from(cell)));
    }
    let cell = match (parameters.is_view(), parameters.is_large()) {
        (true, _) => downcast::<BinaryViewArray>(array)?.value(index),
        (false, true) => downcast::<LargeBinaryArray>(array)?.value(index),
        (false, false) => downcast::<BinaryArray>(array)?.value(index),
    };
    Ok(parameters.scalar_from_bytes(cell)?)
}

/// Read the WKB a geospatial column stores, in either value spelling.
fn optional_wkb(value: &Scalar) -> Result<Option<&[u8]>> {
    match value {
        Scalar::Null => Ok(None),
        Scalar::Geometry(value) => Ok(Some(value.as_bytes())),
        Scalar::Geography(value) => Ok(Some(value.as_bytes())),
        crate::bytes_scalars!(bytes) => Ok(Some(bytes.as_bytes())),
        _ => Err(invalid_value_kind("well-known binary", value)),
    }
}

/// Read the text a string column stores, refusing anything that is not text.
fn optional_str(value: &Scalar) -> Result<Option<&str>> {
    match value {
        Scalar::Null => Ok(None),
        crate::string_scalars!(text) => Ok(Some(text.as_str())),
        _ => Err(invalid_value_kind("string", value)),
    }
}

/// Read the boolean a boolean column stores, refusing anything that is not one.
fn optional_bool(value: &Scalar) -> Result<Option<bool>> {
    match value {
        Scalar::Null => Ok(None),
        Scalar::Boolean(value) => Ok(Some(value.get())),
        _ => Err(invalid_value_kind("boolean", value)),
    }
}

fn exact_f64(value: &Scalar) -> Result<f64> {
    value
        .as_f64()
        .ok_or_else(|| invalid_value_kind("float", value))
}

#[allow(clippy::cast_possible_truncation)]
fn narrow_f32(value: &Scalar) -> Result<f32> {
    exact_f64(value).map(|value| value as f32)
}

fn interval_day_time(value: &Scalar) -> Result<IntervalDayTime> {
    let value = interval_value(value, TimeUnit::DayTime)?;
    let milliseconds = i32::try_from(value.nanoseconds() / 1_000_000).map_err(|_| {
        invalid_value(
            "a day_time interval with a signed 32-bit millisecond count",
            value.nanoseconds(),
        )
    })?;
    Ok(IntervalDayTime::new(value.days(), milliseconds))
}

fn interval_month_day_nano(value: &Scalar) -> Result<IntervalMonthDayNano> {
    let value = interval_value(value, TimeUnit::MonthDayNano)?;
    Ok(IntervalMonthDayNano::new(
        value.months(),
        value.days(),
        value.nanoseconds(),
    ))
}

fn interval_year_month(value: &Scalar) -> Result<i32> {
    Ok(interval_value(value, TimeUnit::YearMonth)?.months())
}

fn interval_value(value: &Scalar, unit: TimeUnit) -> Result<&crate::Interval> {
    match value {
        Scalar::Interval(interval) if interval.unit() == unit => Ok(interval),
        _ => Err(invalid_value_kind(
            "an interval in the declared layout",
            value,
        )),
    }
}

fn decimal256(value: &Scalar, scale: i8) -> Result<ArrowI256> {
    value
        .decimal256_unscaled_at(scale)
        .map(|coefficient| ArrowI256::from_le_bytes(coefficient.into_le_bytes()))
        .ok_or_else(|| {
            invalid_value(
                &format!("a decimal256 representable at scale {scale}"),
                value.kind(),
            )
        })
}

/// Reports a rejected value whose only observable detail is its kind.
fn invalid_value_kind(expected: &str, value: &Scalar) -> Error {
    invalid_value(expected, value.kind())
}

fn allocation_error(
    context: &'static str,
    requested: usize,
    error: &std::collections::TryReserveError,
) -> Error {
    Error::allocation(context, requested, error.clone())
}
