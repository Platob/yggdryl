//! Total conversion between a [`ScalarValue`] and Apache Arrow: a scalar renders to a
//! **length-1 [`ArrayRef`]** ([`to_array`](ScalarValue::to_array)) or an
//! [`arrow_array::Scalar`] ([`to_arrow_scalar`](ScalarValue::to_arrow_scalar)), and any Arrow
//! array cell reads back into a [`ScalarValue`] ([`from_array`](ScalarValue::from_array) /
//! [`from_arrow_scalar`](ScalarValue::from_arrow_scalar)).
//!
//! Logical refinements the Arrow type system does not carry are **normalised** on the
//! round-trip, exactly as in [`yggdryl-schema`](yggdryl_schema)'s Arrow conversion:
//! a [`Json`](crate::ScalarValue::Json) reads back as a [`Utf8`](crate::ScalarValue::Utf8), a
//! fixed-size string loses its length, a non-UTF-8 charset maps to UTF-8. For a fully
//! lossless round-trip carrying the exact logical type use
//! [`to_str`](ScalarValue::to_str) / [`to_json`](ScalarValue::to_json).

use std::sync::Arc;

use arrow_array::{new_empty_array, new_null_array, GenericListArray, OffsetSizeTrait};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, Date32Array, Date64Array, Datum,
    Decimal128Array, Decimal256Array, Decimal32Array, Decimal64Array, DurationMicrosecondArray,
    DurationMillisecondArray, DurationNanosecondArray, DurationSecondArray, FixedSizeBinaryArray,
    FixedSizeListArray, Float16Array, Float32Array, Float64Array, Int16Array, Int32Array,
    Int64Array, Int8Array, IntervalDayTimeArray, IntervalMonthDayNanoArray, IntervalYearMonthArray,
    LargeBinaryArray, LargeListArray, LargeStringArray, ListArray, MapArray, StringArray,
    StringViewArray, StructArray, Time32MillisecondArray, Time32SecondArray,
    Time64MicrosecondArray, Time64NanosecondArray, TimestampMicrosecondArray,
    TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt16Array,
    UInt32Array, UInt64Array, UInt8Array,
};
use arrow_buffer::{i256, IntervalDayTime, IntervalMonthDayNano, OffsetBuffer};
use arrow_schema::{
    DataType as ADataType, Field as AField, Fields, IntervalUnit as AIntervalUnit,
    TimeUnit as ATimeUnit,
};
use yggdryl_core::{TimeUnit, Timezone};
use yggdryl_schema::{DataType, Field};

use crate::error::{ScalarError, ScalarResult};
#[allow(unused_imports)]
use crate::log_event;
use crate::value::{Interval, ScalarValue, F64};

impl ScalarValue {
    /// Renders this value as a **length-1 Arrow [`ArrayRef`]** of its
    /// [`DataType`](ScalarValue::data_type)'s Arrow type — the inverse of
    /// [`from_array`](ScalarValue::from_array) at index 0.
    ///
    /// ```
    /// use yggdryl_scalar::ScalarValue;
    /// let value = ScalarValue::utf8("hi");
    /// let array = value.to_array().unwrap();
    /// assert_eq!(array.len(), 1);
    /// assert_eq!(ScalarValue::from_array(array.as_ref(), 0).unwrap(), value);
    /// ```
    pub fn to_array(&self) -> ScalarResult<ArrayRef> {
        use ScalarValue::*;
        log_event!(trace, "ScalarValue::to_array {}", self.data_type().to_str());
        Ok(match self {
            Null(dt) => new_null_array(&dt.to_arrow()?, 1),
            Boolean(v) => Arc::new(BooleanArray::from(vec![*v])),
            Int {
                value,
                bits,
                signed,
            } => int_array(*value, *bits, *signed)?,
            Float { value, bits } => float_array(value.0, *bits)?,
            Decimal {
                value,
                precision,
                scale,
                bits,
            } => decimal_array(*value, *precision, *scale, *bits)?,
            Utf8 {
                value, large, view, ..
            } => string_array(value, *large, *view),
            Binary {
                value,
                large,
                view,
                size,
            } => binary_array(value, *large, *view, *size)?,
            // Json / Bson / Timezone lower to their physical Utf8 / Binary array (the
            // logical name is not recoverable from the array — see the module docs).
            Json(v) => string_array(v, false, false),
            Bson(v) => binary_array(v, false, false, None)?,
            Timezone(tz) => string_array(&tz.name(), false, false),
            Date { value, large } => {
                if *large {
                    Arc::new(Date64Array::from(vec![*value]))
                } else {
                    Arc::new(Date32Array::from(vec![*value as i32]))
                }
            }
            Time { value, unit } => time_array(*value, *unit),
            Timestamp {
                value,
                unit,
                timezone,
            } => timestamp_array(*value, *unit, timezone.as_ref()),
            Duration { value, unit } => duration_array(*value, *unit),
            Interval(iv) => interval_array(*iv),
            List {
                values,
                field,
                large,
                view,
                size,
            } => list_array(values, field, *large, *view, *size)?,
            Struct { fields, values } => struct_array(fields, values)?,
            Map {
                key,
                value,
                sorted,
                entries,
            } => map_array(key, value, *sorted, entries)?,
        })
    }

    /// Wraps [`to_array`](ScalarValue::to_array) in an [`arrow_array::Scalar`] — the broadcast
    /// marker Arrow's compute kernels treat as a single value rather than a length-1
    /// column.
    pub fn to_arrow_scalar(&self) -> ScalarResult<arrow_array::Scalar<ArrayRef>> {
        Ok(arrow_array::Scalar::new(self.to_array()?))
    }

    /// Reads the cell of `array` at `index` into a [`ScalarValue`]. A null cell or an
    /// out-of-bounds index yields a typed [`Null`](ScalarValue::Null) of the array's type.
    ///
    /// ```
    /// use yggdryl_scalar::ScalarValue;
    /// use yggdryl_scalar::arrow_array::{Int32Array, ArrayRef};
    /// use std::sync::Arc;
    /// let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(7), None]));
    /// assert_eq!(ScalarValue::from_array(array.as_ref(), 0).unwrap(), ScalarValue::int(7, 32, true));
    /// assert!(ScalarValue::from_array(array.as_ref(), 1).unwrap().is_null());
    /// ```
    pub fn from_array(array: &dyn Array, index: usize) -> ScalarResult<ScalarValue> {
        if index >= array.len() || array.is_null(index) {
            return Ok(ScalarValue::Null(DataType::from_arrow(array.data_type())));
        }

        /// Downcasts `array` to `$ty` and reads its value at `index`.
        macro_rules! get {
            ($ty:ty) => {
                array
                    .as_any()
                    .downcast_ref::<$ty>()
                    .expect("data type matched the array")
                    .value(index)
            };
        }

        Ok(match array.data_type() {
            ADataType::Boolean => ScalarValue::Boolean(get!(BooleanArray)),
            ADataType::Int8 => ScalarValue::int(get!(Int8Array) as i128, 8, true),
            ADataType::Int16 => ScalarValue::int(get!(Int16Array) as i128, 16, true),
            ADataType::Int32 => ScalarValue::int(get!(Int32Array) as i128, 32, true),
            ADataType::Int64 => ScalarValue::int(get!(Int64Array) as i128, 64, true),
            ADataType::UInt8 => ScalarValue::int(get!(UInt8Array) as i128, 8, false),
            ADataType::UInt16 => ScalarValue::int(get!(UInt16Array) as i128, 16, false),
            ADataType::UInt32 => ScalarValue::int(get!(UInt32Array) as i128, 32, false),
            ADataType::UInt64 => ScalarValue::int(get!(UInt64Array) as i128, 64, false),
            ADataType::Float16 => ScalarValue::Float {
                value: F64(get!(Float16Array).to_f64()),
                bits: 16,
            },
            ADataType::Float32 => ScalarValue::float(get!(Float32Array) as f64, 32),
            ADataType::Float64 => ScalarValue::float(get!(Float64Array), 64),
            ADataType::Decimal32(p, s) => {
                ScalarValue::decimal(i256::from_i128(get!(Decimal32Array) as i128), *p, *s, 32)
            }
            ADataType::Decimal64(p, s) => {
                ScalarValue::decimal(i256::from_i128(get!(Decimal64Array) as i128), *p, *s, 64)
            }
            ADataType::Decimal128(p, s) => {
                ScalarValue::decimal(i256::from_i128(get!(Decimal128Array)), *p, *s, 128)
            }
            ADataType::Decimal256(p, s) => ScalarValue::decimal(get!(Decimal256Array), *p, *s, 256),
            ADataType::Utf8 => ScalarValue::utf8(get!(StringArray).to_string()),
            ADataType::LargeUtf8 => ScalarValue::Utf8 {
                value: get!(LargeStringArray).to_string(),
                charset: yggdryl_core::Charset::Utf8,
                large: true,
                view: false,
                size: None,
            },
            ADataType::Utf8View => ScalarValue::Utf8 {
                value: get!(StringViewArray).to_string(),
                charset: yggdryl_core::Charset::Utf8,
                large: false,
                view: true,
                size: None,
            },
            ADataType::Binary => ScalarValue::binary(get!(BinaryArray).to_vec()),
            ADataType::LargeBinary => ScalarValue::Binary {
                value: get!(LargeBinaryArray).to_vec(),
                large: true,
                view: false,
                size: None,
            },
            ADataType::BinaryView => ScalarValue::Binary {
                value: get!(BinaryViewArray).to_vec(),
                large: false,
                view: true,
                size: None,
            },
            ADataType::FixedSizeBinary(n) => ScalarValue::Binary {
                value: get!(FixedSizeBinaryArray).to_vec(),
                large: false,
                view: false,
                size: Some(*n),
            },
            ADataType::Date32 => ScalarValue::Date {
                value: get!(Date32Array) as i64,
                large: false,
            },
            ADataType::Date64 => ScalarValue::Date {
                value: get!(Date64Array),
                large: true,
            },
            ADataType::Time32(ATimeUnit::Second) => ScalarValue::Time {
                value: get!(Time32SecondArray) as i64,
                unit: TimeUnit::Second,
            },
            ADataType::Time32(ATimeUnit::Millisecond) => ScalarValue::Time {
                value: get!(Time32MillisecondArray) as i64,
                unit: TimeUnit::Millisecond,
            },
            ADataType::Time64(ATimeUnit::Microsecond) => ScalarValue::Time {
                value: get!(Time64MicrosecondArray),
                unit: TimeUnit::Microsecond,
            },
            ADataType::Time64(ATimeUnit::Nanosecond) => ScalarValue::Time {
                value: get!(Time64NanosecondArray),
                unit: TimeUnit::Nanosecond,
            },
            ADataType::Time32(_) | ADataType::Time64(_) => {
                return Err(ScalarError::Unsupported(
                    "unsupported time resolution in array".into(),
                ))
            }
            ADataType::Timestamp(unit, tz) => ScalarValue::Timestamp {
                value: timestamp_value(array, index, unit),
                unit: time_unit_from_arrow(unit),
                timezone: tz.as_ref().map(|name| parse_timezone(name)),
            },
            ADataType::Duration(unit) => ScalarValue::Duration {
                value: duration_value(array, index, unit),
                unit: time_unit_from_arrow(unit),
            },
            ADataType::Interval(AIntervalUnit::YearMonth) => {
                ScalarValue::Interval(Interval::YearMonth(get!(IntervalYearMonthArray)))
            }
            ADataType::Interval(AIntervalUnit::DayTime) => {
                let v = get!(IntervalDayTimeArray);
                ScalarValue::Interval(Interval::DayTime {
                    days: v.days,
                    millis: v.milliseconds,
                })
            }
            ADataType::Interval(AIntervalUnit::MonthDayNano) => {
                let v = get!(IntervalMonthDayNanoArray);
                ScalarValue::Interval(Interval::MonthDayNano {
                    months: v.months,
                    days: v.days,
                    nanos: v.nanoseconds,
                })
            }
            ADataType::List(_) => read_list::<i32>(array, index, false)?,
            ADataType::LargeList(_) => read_list::<i64>(array, index, true)?,
            ADataType::FixedSizeList(field, _) => read_fixed_list(array, index, field)?,
            ADataType::Struct(fields) => read_struct(array, index, fields)?,
            ADataType::Map(_, sorted) => read_map(array, index, *sorted)?,
            other => {
                return Err(ScalarError::Unsupported(format!(
                    "reading a '{other}' value into a scalar is not supported; decode \
                     dictionary / run-end / union columns to a flat type first"
                )))
            }
        })
    }

    /// Reads index 0 of an [`arrow_array::Scalar`] back into a [`ScalarValue`].
    pub fn from_arrow_scalar<T: Array>(
        scalar: &arrow_array::Scalar<T>,
    ) -> ScalarResult<ScalarValue> {
        let (array, _is_scalar) = scalar.get();
        ScalarValue::from_array(array, 0)
    }
}

// ---- to_array helpers (one builder per family) ----

/// A length-1 integer array of the given width / signedness.
fn int_array(value: i128, bits: u16, signed: bool) -> ScalarResult<ArrayRef> {
    Ok(match (bits, signed) {
        (8, true) => Arc::new(Int8Array::from(vec![value as i8])),
        (16, true) => Arc::new(Int16Array::from(vec![value as i16])),
        (32, true) => Arc::new(Int32Array::from(vec![value as i32])),
        (64, true) => Arc::new(Int64Array::from(vec![value as i64])),
        (8, false) => Arc::new(UInt8Array::from(vec![value as u8])),
        (16, false) => Arc::new(UInt16Array::from(vec![value as u16])),
        (32, false) => Arc::new(UInt32Array::from(vec![value as u32])),
        (64, false) => Arc::new(UInt64Array::from(vec![value as u64])),
        _ => {
            return Err(ScalarError::Unsupported(format!(
                "Arrow has no {}int{bits}; use a standard width (8/16/32/64)",
                if signed { "" } else { "u" }
            )))
        }
    })
}

/// A length-1 float array of the given width.
fn float_array(value: f64, bits: u16) -> ScalarResult<ArrayRef> {
    Ok(match bits {
        16 => Arc::new(Float16Array::from(vec![half::f16::from_f64(value)])),
        32 => Arc::new(Float32Array::from(vec![value as f32])),
        64 => Arc::new(Float64Array::from(vec![value])),
        _ => {
            return Err(ScalarError::Unsupported(format!(
                "Arrow has no float{bits}; use a standard width (16/32/64)"
            )))
        }
    })
}

/// A length-1 decimal array of the given storage width, precision and scale.
fn decimal_array(value: i256, precision: u8, scale: i8, bits: u16) -> ScalarResult<ArrayRef> {
    let overflow = || {
        ScalarError::Invalid(format!(
            "decimal value {value} does not fit a decimal{bits}"
        ))
    };
    Ok(match bits {
        32 => Arc::new(
            Decimal32Array::from(vec![
                i32::try_from(value.to_i128().ok_or_else(overflow)?).map_err(|_| overflow())?
            ])
            .with_precision_and_scale(precision, scale)?,
        ),
        64 => Arc::new(
            Decimal64Array::from(vec![
                i64::try_from(value.to_i128().ok_or_else(overflow)?).map_err(|_| overflow())?
            ])
            .with_precision_and_scale(precision, scale)?,
        ),
        256 => {
            Arc::new(Decimal256Array::from(vec![value]).with_precision_and_scale(precision, scale)?)
        }
        _ => Arc::new(
            Decimal128Array::from(vec![value.to_i128().ok_or_else(overflow)?])
                .with_precision_and_scale(precision, scale)?,
        ),
    })
}

/// A length-1 string array of the right offset / view flavour (a fixed `size` is
/// dropped, as Arrow has no fixed-size UTF-8 — matching the schema conversion).
fn string_array(value: &str, large: bool, view: bool) -> ArrayRef {
    if large {
        Arc::new(LargeStringArray::from_iter_values([value]))
    } else if view {
        Arc::new(StringViewArray::from_iter_values([value]))
    } else {
        Arc::new(StringArray::from_iter_values([value]))
    }
}

/// A length-1 binary array of the right flavour.
fn binary_array(
    value: &[u8],
    large: bool,
    view: bool,
    size: Option<i32>,
) -> ScalarResult<ArrayRef> {
    Ok(match (large, view, size) {
        (_, _, Some(_)) => Arc::new(FixedSizeBinaryArray::try_from_sparse_iter_with_size(
            std::iter::once(Some(value)),
            size.unwrap(),
        )?),
        (true, _, None) => Arc::new(LargeBinaryArray::from_iter_values([value])),
        (_, true, None) => Arc::new(BinaryViewArray::from_iter_values([value])),
        _ => Arc::new(BinaryArray::from_iter_values([value])),
    })
}

/// A length-1 time-of-day array of the right resolution.
fn time_array(value: i64, unit: TimeUnit) -> ArrayRef {
    match unit {
        TimeUnit::Second => Arc::new(Time32SecondArray::from(vec![value as i32])),
        TimeUnit::Millisecond => Arc::new(Time32MillisecondArray::from(vec![value as i32])),
        TimeUnit::Microsecond => Arc::new(Time64MicrosecondArray::from(vec![value])),
        TimeUnit::Nanosecond => Arc::new(Time64NanosecondArray::from(vec![value])),
    }
}

/// A length-1 timestamp array of the right resolution, with the optional timezone.
fn timestamp_array(value: i64, unit: TimeUnit, timezone: Option<&Timezone>) -> ArrayRef {
    let tz = timezone.map(|t| Arc::<str>::from(t.name().as_str()));
    match unit {
        TimeUnit::Second => Arc::new(TimestampSecondArray::from(vec![value]).with_timezone_opt(tz)),
        TimeUnit::Millisecond => {
            Arc::new(TimestampMillisecondArray::from(vec![value]).with_timezone_opt(tz))
        }
        TimeUnit::Microsecond => {
            Arc::new(TimestampMicrosecondArray::from(vec![value]).with_timezone_opt(tz))
        }
        TimeUnit::Nanosecond => {
            Arc::new(TimestampNanosecondArray::from(vec![value]).with_timezone_opt(tz))
        }
    }
}

/// A length-1 duration array of the right resolution.
fn duration_array(value: i64, unit: TimeUnit) -> ArrayRef {
    match unit {
        TimeUnit::Second => Arc::new(DurationSecondArray::from(vec![value])),
        TimeUnit::Millisecond => Arc::new(DurationMillisecondArray::from(vec![value])),
        TimeUnit::Microsecond => Arc::new(DurationMicrosecondArray::from(vec![value])),
        TimeUnit::Nanosecond => Arc::new(DurationNanosecondArray::from(vec![value])),
    }
}

/// A length-1 interval array of the right resolution.
fn interval_array(interval: Interval) -> ArrayRef {
    match interval {
        Interval::YearMonth(months) => Arc::new(IntervalYearMonthArray::from(vec![months])),
        Interval::DayTime { days, millis } => {
            Arc::new(IntervalDayTimeArray::from(vec![IntervalDayTime::new(
                days, millis,
            )]))
        }
        Interval::MonthDayNano {
            months,
            days,
            nanos,
        } => Arc::new(IntervalMonthDayNanoArray::from(vec![
            IntervalMonthDayNano::new(months, days, nanos),
        ])),
    }
}

/// Concatenates each scalar's length-1 array into the flattened child array of a nested
/// value (or an empty array of `item_type` when there are no elements).
fn child_array(values: &[ScalarValue], item_type: &ADataType) -> ScalarResult<ArrayRef> {
    if values.is_empty() {
        return Ok(new_empty_array(item_type));
    }
    let arrays = values
        .iter()
        .map(ScalarValue::to_array)
        .collect::<ScalarResult<Vec<_>>>()?;
    let refs: Vec<&dyn Array> = arrays.iter().map(|a| a.as_ref()).collect();
    Ok(arrow_select::concat::concat(&refs)?)
}

/// A length-1 list array whose single element is the list of `values`.
fn list_array(
    values: &[ScalarValue],
    field: &Field,
    large: bool,
    view: bool,
    size: Option<i32>,
) -> ScalarResult<ArrayRef> {
    if view {
        return Err(ScalarError::Unsupported(
            "list-view scalars are not supported; use a plain list".into(),
        ));
    }
    let item_arrow = field.to_arrow()?;
    let child = child_array(values, item_arrow.data_type())?;
    let fref = Arc::new(item_arrow);
    Ok(match size {
        Some(n) => Arc::new(FixedSizeListArray::new(fref, n, child, None)),
        None if large => {
            let offsets = OffsetBuffer::<i64>::from_lengths([child.len()]);
            Arc::new(LargeListArray::new(fref, offsets, child, None))
        }
        None => {
            let offsets = OffsetBuffer::<i32>::from_lengths([child.len()]);
            Arc::new(ListArray::new(fref, offsets, child, None))
        }
    })
}

/// A length-1 struct array holding one record.
fn struct_array(fields: &[Field], values: &[ScalarValue]) -> ScalarResult<ArrayRef> {
    if fields.is_empty() {
        return Ok(Arc::new(StructArray::new_empty_fields(1, None)));
    }
    let afields = fields
        .iter()
        .map(|f| f.to_arrow().map(Arc::new))
        .collect::<Result<Vec<_>, _>>()?;
    let arrays = values
        .iter()
        .map(ScalarValue::to_array)
        .collect::<ScalarResult<Vec<_>>>()?;
    let fields: Fields = afields.into();
    Ok(Arc::new(StructArray::try_new(fields, arrays, None)?))
}

/// A length-1 map array holding one map's entries.
fn map_array(
    key: &DataType,
    value: &DataType,
    sorted: bool,
    entries: &[(ScalarValue, ScalarValue)],
) -> ScalarResult<ArrayRef> {
    let key_arrow = key.to_arrow()?;
    let value_arrow = value.to_arrow()?;
    let keys: Vec<ScalarValue> = entries.iter().map(|(k, _)| k.clone()).collect();
    let vals: Vec<ScalarValue> = entries.iter().map(|(_, v)| v.clone()).collect();
    let keys_child = child_array(&keys, &key_arrow)?;
    let vals_child = child_array(&vals, &value_arrow)?;
    let entry_fields: Fields = vec![
        Arc::new(AField::new("key", key_arrow, false)),
        Arc::new(AField::new("value", value_arrow, true)),
    ]
    .into();
    let entries_struct = StructArray::try_new(entry_fields, vec![keys_child, vals_child], None)?;
    let entries_field = Arc::new(AField::new(
        "entries",
        entries_struct.data_type().clone(),
        false,
    ));
    let offsets = OffsetBuffer::<i32>::from_lengths([entries_struct.len()]);
    Ok(Arc::new(MapArray::try_new(
        entries_field,
        offsets,
        entries_struct,
        None,
        sorted,
    )?))
}

// ---- from_array helpers ----

/// Reads the physical `i64` of a timestamp cell (any resolution).
fn timestamp_value(array: &dyn Array, index: usize, unit: &ATimeUnit) -> i64 {
    macro_rules! v {
        ($ty:ty) => {
            array.as_any().downcast_ref::<$ty>().unwrap().value(index)
        };
    }
    match unit {
        ATimeUnit::Second => v!(TimestampSecondArray),
        ATimeUnit::Millisecond => v!(TimestampMillisecondArray),
        ATimeUnit::Microsecond => v!(TimestampMicrosecondArray),
        ATimeUnit::Nanosecond => v!(TimestampNanosecondArray),
    }
}

/// Reads the physical `i64` of a duration cell (any resolution).
fn duration_value(array: &dyn Array, index: usize, unit: &ATimeUnit) -> i64 {
    macro_rules! v {
        ($ty:ty) => {
            array.as_any().downcast_ref::<$ty>().unwrap().value(index)
        };
    }
    match unit {
        ATimeUnit::Second => v!(DurationSecondArray),
        ATimeUnit::Millisecond => v!(DurationMillisecondArray),
        ATimeUnit::Microsecond => v!(DurationMicrosecondArray),
        ATimeUnit::Nanosecond => v!(DurationNanosecondArray),
    }
}

/// Maps an Arrow time unit to the core [`TimeUnit`].
fn time_unit_from_arrow(unit: &ATimeUnit) -> TimeUnit {
    match unit {
        ATimeUnit::Second => TimeUnit::Second,
        ATimeUnit::Millisecond => TimeUnit::Millisecond,
        ATimeUnit::Microsecond => TimeUnit::Microsecond,
        ATimeUnit::Nanosecond => TimeUnit::Nanosecond,
    }
}

/// Parses an Arrow timezone name, defaulting to UTC with a warn (like the schema layer).
fn parse_timezone(name: &str) -> Timezone {
    Timezone::from_str(name).unwrap_or_else(|_| {
        log_event!(
            warn,
            "ScalarValue::from_array: timezone {name:?} is not a recognised zone, defaulting to UTC"
        );
        Timezone::Utc
    })
}

/// Reads the sub-list at `index` of a (large) list array into a [`ScalarValue::List`].
fn read_list<O: OffsetSizeTrait>(
    array: &dyn Array,
    index: usize,
    large: bool,
) -> ScalarResult<ScalarValue> {
    let list = array
        .as_any()
        .downcast_ref::<GenericListArray<O>>()
        .expect("data type matched the list array");
    let sub = list.value(index);
    let values = (0..sub.len())
        .map(|k| ScalarValue::from_array(sub.as_ref(), k))
        .collect::<ScalarResult<Vec<_>>>()?;
    let field = match list.data_type() {
        ADataType::List(f) | ADataType::LargeList(f) => Field::from_arrow(f),
        _ => unreachable!("list array has a list data type"),
    };
    Ok(ScalarValue::List {
        values,
        field: Box::new(field),
        large,
        view: false,
        size: None,
    })
}

/// Reads the sub-list at `index` of a fixed-size list array into a [`ScalarValue::List`].
fn read_fixed_list(
    array: &dyn Array,
    index: usize,
    field: &Arc<AField>,
) -> ScalarResult<ScalarValue> {
    let list = array
        .as_any()
        .downcast_ref::<FixedSizeListArray>()
        .expect("data type matched the fixed-size list array");
    let sub = list.value(index);
    let n = sub.len();
    let values = (0..n)
        .map(|k| ScalarValue::from_array(sub.as_ref(), k))
        .collect::<ScalarResult<Vec<_>>>()?;
    Ok(ScalarValue::List {
        values,
        field: Box::new(Field::from_arrow(field)),
        large: false,
        view: false,
        size: Some(n as i32),
    })
}

/// Reads the record at `index` of a struct array into a [`ScalarValue::Struct`].
fn read_struct(array: &dyn Array, index: usize, fields: &Fields) -> ScalarResult<ScalarValue> {
    let s = array
        .as_any()
        .downcast_ref::<StructArray>()
        .expect("data type matched the struct array");
    let values = s
        .columns()
        .iter()
        .map(|col| ScalarValue::from_array(col.as_ref(), index))
        .collect::<ScalarResult<Vec<_>>>()?;
    let fields = fields.iter().map(|f| Field::from_arrow(f)).collect();
    Ok(ScalarValue::Struct { fields, values })
}

/// Reads the map at `index` of a map array into a [`ScalarValue::Map`].
fn read_map(array: &dyn Array, index: usize, sorted: bool) -> ScalarResult<ScalarValue> {
    let map = array
        .as_any()
        .downcast_ref::<MapArray>()
        .expect("data type matched the map array");
    let offsets = map.value_offsets();
    let start = offsets[index] as usize;
    let end = offsets[index + 1] as usize;
    let keys = map.keys();
    let vals = map.values();
    let entries = (start..end)
        .map(|k| {
            Ok((
                ScalarValue::from_array(keys.as_ref(), k)?,
                ScalarValue::from_array(vals.as_ref(), k)?,
            ))
        })
        .collect::<ScalarResult<Vec<_>>>()?;
    Ok(ScalarValue::Map {
        key: Box::new(DataType::from_arrow(keys.data_type())),
        value: Box::new(DataType::from_arrow(vals.data_type())),
        sorted,
        entries,
    })
}
