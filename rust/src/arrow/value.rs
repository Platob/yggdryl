//! Schema-directed scalar/array conversion.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::{DataType, Field, I256, Scalar, TimeUnit, Timezone, UnionMode};
use arrow_array::types::{
    Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type, UInt32Type, UInt64Type,
};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, Date32Array, Date64Array,
    Decimal32Array, Decimal64Array, Decimal128Array, Decimal256Array, DictionaryArray,
    DurationMicrosecondArray, DurationMillisecondArray, DurationNanosecondArray,
    DurationSecondArray, FixedSizeBinaryArray, FixedSizeListArray, Float16Array, Float32Array,
    Float64Array, Int8Array, Int16Array, Int16RunArray, Int32Array, Int32RunArray, Int64Array,
    Int64RunArray, IntervalDayTimeArray, IntervalMonthDayNanoArray, IntervalYearMonthArray,
    LargeBinaryArray, LargeListArray, LargeListViewArray, LargeStringArray, ListArray,
    ListViewArray, MapArray, NullArray, PrimitiveArray, StringArray, StringViewArray, StructArray,
    Time32MillisecondArray, Time32SecondArray, Time64MicrosecondArray, Time64NanosecondArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray, UInt8Array, UInt16Array, UInt32Array, UInt64Array, UnionArray,
    make_array, new_empty_array,
};
use arrow_buffer::{
    IntervalDayTime, IntervalMonthDayNano, NullBuffer, OffsetBuffer, ScalarBuffer, i256,
};
use arrow_schema::DataType as ArrowDataType;
use half::f16;

use super::{Error, Result};

#[allow(clippy::too_many_lines)]
pub(crate) fn array_from_values(field: &Field, values: &[&Scalar]) -> Result<ArrayRef> {
    let data_type = field.data_type();
    let arrow_type = field.clone().into_arrow_ref()?.data_type().clone();
    // Arrow owns the canonical empty representation for every validated
    // datatype. Taking this path before schema-directed value materialization
    // avoids inventing defaults for children that have no physical slots.
    if values.is_empty() {
        return Ok(new_empty_array(&arrow_type));
    }
    macro_rules! primitive {
        ($array:ty, $conversion:expr) => {{
            let values = values
                .iter()
                .map(|value| {
                    if matches!(value, Scalar::Null) {
                        Ok(None)
                    } else {
                        ($conversion)(value).map(Some)
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            Arc::new(<$array>::from(values)) as ArrayRef
        }};
    }
    macro_rules! physical_primitive {
        ($array:ty, $conversion:expr) => {{
            let array = primitive!($array, $conversion);
            let array = downcast::<$array>(&array)?
                .clone()
                .with_data_type(arrow_type.clone());
            Arc::new(array) as ArrayRef
        }};
    }
    let array = match data_type {
        DataType::Null => Arc::new(NullArray::new(values.len())) as ArrayRef,
        DataType::Boolean => Arc::new(BooleanArray::from(
            values
                .iter()
                .map(|value| optional_bool(value))
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::Int8 => primitive!(Int8Array, |value: &&Scalar| exact_i128(value)
            .and_then(|value| i8::try_from(value).map_err(|_| invalid_value("int8", value)))),
        DataType::Int16 => primitive!(Int16Array, |value: &&Scalar| exact_i128(value)
            .and_then(|value| i16::try_from(value).map_err(|_| invalid_value("int16", value)))),
        DataType::Int32 => primitive!(Int32Array, |value: &&Scalar| exact_i128(value)
            .and_then(|value| i32::try_from(value).map_err(|_| invalid_value("int32", value)))),
        DataType::Int64 => primitive!(Int64Array, |value: &&Scalar| exact_i128(value)
            .and_then(|value| i64::try_from(value).map_err(|_| invalid_value("int64", value)))),
        DataType::UInt8 => primitive!(UInt8Array, |value: &&Scalar| exact_u128(value)
            .and_then(|value| u8::try_from(value).map_err(|_| invalid_value("uint8", value)))),
        DataType::UInt16 => primitive!(UInt16Array, |value: &&Scalar| exact_u128(value)
            .and_then(|value| u16::try_from(value).map_err(|_| invalid_value("uint16", value)))),
        DataType::UInt32 => primitive!(UInt32Array, |value: &&Scalar| exact_u128(value)
            .and_then(|value| u32::try_from(value).map_err(|_| invalid_value("uint32", value)))),
        DataType::UInt64 => primitive!(UInt64Array, |value: &&Scalar| exact_u128(value)
            .and_then(|value| u64::try_from(value).map_err(|_| invalid_value("uint64", value)))),
        DataType::Float16 => primitive!(Float16Array, |value: &&Scalar| exact_f64(value)
            .map(f16::from_f64)),
        DataType::Float32 => primitive!(Float32Array, narrow_f32),
        DataType::Float64 => primitive!(Float64Array, |value: &&Scalar| exact_f64(value)),
        DataType::Timestamp(unit, _) => match unit {
            TimeUnit::Second => physical_primitive!(TimestampSecondArray, temporal_i64(*unit)),
            TimeUnit::Millisecond => {
                physical_primitive!(TimestampMillisecondArray, temporal_i64(*unit))
            }
            TimeUnit::Microsecond => {
                physical_primitive!(TimestampMicrosecondArray, temporal_i64(*unit))
            }
            TimeUnit::Nanosecond => {
                physical_primitive!(TimestampNanosecondArray, temporal_i64(*unit))
            }
            _ => return Err(unsupported(data_type, "invalid timestamp unit")),
        },
        DataType::Date32 => primitive!(Date32Array, date_i32),
        DataType::Date64 => primitive!(Date64Array, date_i64),
        DataType::Time32(unit) => match unit {
            TimeUnit::Second => primitive!(Time32SecondArray, temporal_i32(*unit)),
            TimeUnit::Millisecond => primitive!(Time32MillisecondArray, temporal_i32(*unit)),
            _ => return Err(unsupported(data_type, "invalid time32 unit")),
        },
        DataType::Time64(unit) => match unit {
            TimeUnit::Microsecond => primitive!(Time64MicrosecondArray, temporal_i64(*unit)),
            TimeUnit::Nanosecond => primitive!(Time64NanosecondArray, temporal_i64(*unit)),
            _ => return Err(unsupported(data_type, "invalid time64 unit")),
        },
        DataType::Duration32(unit) => match unit {
            TimeUnit::Second => primitive!(DurationSecondArray, |value: &&Scalar| {
                temporal_i32(*unit)(value).map(i64::from)
            }),
            TimeUnit::Millisecond => primitive!(DurationMillisecondArray, |value: &&Scalar| {
                temporal_i32(*unit)(value).map(i64::from)
            }),
            TimeUnit::Microsecond => primitive!(DurationMicrosecondArray, |value: &&Scalar| {
                temporal_i32(*unit)(value).map(i64::from)
            }),
            TimeUnit::Nanosecond => primitive!(DurationNanosecondArray, |value: &&Scalar| {
                temporal_i32(*unit)(value).map(i64::from)
            }),
            _ => return Err(unsupported(data_type, "invalid duration32 unit")),
        },
        DataType::Duration64(unit) => match unit {
            TimeUnit::Second => primitive!(DurationSecondArray, temporal_i64(*unit)),
            TimeUnit::Millisecond => primitive!(DurationMillisecondArray, temporal_i64(*unit)),
            TimeUnit::Microsecond => primitive!(DurationMicrosecondArray, temporal_i64(*unit)),
            TimeUnit::Nanosecond => primitive!(DurationNanosecondArray, temporal_i64(*unit)),
            _ => return Err(unsupported(data_type, "invalid duration64 unit")),
        },
        DataType::Interval(TimeUnit::YearMonth) => {
            primitive!(IntervalYearMonthArray, signed_i32)
        }
        DataType::Interval(TimeUnit::DayTime) => {
            primitive!(IntervalDayTimeArray, interval_day_time)
        }
        DataType::Interval(TimeUnit::MonthDayNano) => {
            primitive!(IntervalMonthDayNanoArray, interval_month_day_nano)
        }
        DataType::Interval(_) => return Err(unsupported(data_type, "invalid interval layout")),
        DataType::Binary => Arc::new(BinaryArray::from(
            values
                .iter()
                .map(|value| optional_bytes(value))
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::FixedSizeBinary(width) => {
            let bytes = values
                .iter()
                .map(|value| optional_bytes(value))
                .collect::<Result<Vec<_>>>()?;
            Arc::new(FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                bytes.into_iter(),
                *width,
            )?)
        }
        DataType::LargeBinary => Arc::new(LargeBinaryArray::from(
            values
                .iter()
                .map(|value| optional_bytes(value))
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::BinaryView => Arc::new(
            values
                .iter()
                .map(|value| optional_bytes(value))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .collect::<BinaryViewArray>(),
        ),
        DataType::Utf8 => Arc::new(StringArray::from(
            values
                .iter()
                .map(|value| optional_str(value))
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::LargeUtf8 => Arc::new(LargeStringArray::from(
            values
                .iter()
                .map(|value| optional_str(value))
                .collect::<Result<Vec<_>>>()?,
        )),
        DataType::Utf8View => Arc::new(
            values
                .iter()
                .map(|value| optional_str(value))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .collect::<StringViewArray>(),
        ),
        DataType::List(child) => list_array::<i32>(child, values, ListKind::List)?,
        DataType::ListView(child) => list_view_array::<i32>(child, values, ListKind::ListView)?,
        DataType::FixedSizeList(child, size) => fixed_size_list_array(child, *size, values)?,
        DataType::LargeList(child) => list_array::<i64>(child, values, ListKind::LargeList)?,
        DataType::LargeListView(child) => {
            list_view_array::<i64>(child, values, ListKind::LargeListView)?
        }
        DataType::Struct(fields) => struct_array(fields, values)?,
        DataType::Union(fields, mode) => union_array(fields, *mode, values)?,
        DataType::Dictionary(dictionary) => dictionary_array(dictionary, values)?,
        DataType::Decimal32 { scale, .. } => {
            physical_primitive!(Decimal32Array, |value: &&Scalar| i32::try_from(
                unscaled_i128(value, *scale)?
            )
            .map_err(|_| invalid_value("decimal32", value.kind())))
        }
        DataType::Decimal64 { scale, .. } => {
            physical_primitive!(Decimal64Array, |value: &&Scalar| i64::try_from(
                unscaled_i128(value, *scale)?
            )
            .map_err(|_| invalid_value("decimal64", value.kind())))
        }
        DataType::Decimal128 { scale, .. } => {
            physical_primitive!(Decimal128Array, |value: &&Scalar| unscaled_i128(
                value, *scale
            ))
        }
        DataType::Decimal256 { scale, .. } => {
            physical_primitive!(Decimal256Array, |value: &&Scalar| decimal256(value, *scale))
        }
        DataType::Map(map) => map_array(map, values)?,
        DataType::RunEndEncoded(encoded) => run_array(encoded, values)?,
        // A geospatial value *is* its WKB payload, so the array is the bytes;
        // both the canonical `Geospatial` spelling and plain bytes build it.
        DataType::Geometry(_) | DataType::Geography(_) => Arc::new(BinaryArray::from(
            values
                .iter()
                .map(|value| optional_wkb(value))
                .collect::<Result<Vec<_>>>()?,
        )),
        // A variant value crosses this boundary as the Parquet Variant binary
        // encoding, which the Iceberg v3 layer owns; until that codec lands a
        // variant column refuses by name rather than inventing a second
        // encoding here.
        DataType::Variant => {
            return Err(unsupported(
                data_type,
                "the variant binary encoding lands with the Iceberg v3 layer",
            ));
        }
    };
    Ok(array)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn value_from_array(
    data_type: &DataType,
    array: &dyn Array,
    index: usize,
) -> Result<Scalar> {
    if index >= array.len() {
        return Err(Error::IncompatibleSchema(format!(
            "array index {index} exceeds length {}",
            array.len()
        )));
    }
    if array.is_null(index)
        && !matches!(data_type, DataType::Union(..) | DataType::RunEndEncoded(_))
    {
        return Ok(Scalar::Null);
    }
    macro_rules! primitive {
        ($array:ty, $conversion:expr) => {{
            let value = downcast::<$array>(array)?.value(index);
            ($conversion)(value)
        }};
    }
    let value = match data_type {
        DataType::Null => Scalar::Null,
        DataType::Boolean => Scalar::from(downcast::<BooleanArray>(array)?.value(index)),
        DataType::Int8 => primitive!(Int8Array, Scalar::from),
        DataType::Int16 => primitive!(Int16Array, Scalar::from),
        DataType::Int32 => primitive!(Int32Array, Scalar::from),
        DataType::Int64 => primitive!(Int64Array, Scalar::from),
        DataType::UInt8 => primitive!(UInt8Array, Scalar::from),
        DataType::UInt16 => primitive!(UInt16Array, Scalar::from),
        DataType::UInt32 => primitive!(UInt32Array, Scalar::from),
        DataType::UInt64 => primitive!(UInt64Array, Scalar::from),
        DataType::Float16 => primitive!(Float16Array, Scalar::from),
        DataType::Float32 => primitive!(Float32Array, Scalar::from),
        DataType::Float64 => primitive!(Float64Array, Scalar::from),
        // Every temporal reads as its typed value: the count alone is not
        // the datum, the unit and zone are, and the typed spelling is what
        // serializes losslessly and compares across resolutions.
        DataType::Timestamp(unit, zone) => match unit {
            TimeUnit::Second => primitive!(TimestampSecondArray, |value| {
                Scalar::DateTime64(value, *unit, zone.clone().unwrap_or(Timezone::NAIVE))
            }),
            TimeUnit::Millisecond => primitive!(TimestampMillisecondArray, |value| {
                Scalar::DateTime64(value, *unit, zone.clone().unwrap_or(Timezone::NAIVE))
            }),
            TimeUnit::Microsecond => primitive!(TimestampMicrosecondArray, |value| {
                Scalar::DateTime64(value, *unit, zone.clone().unwrap_or(Timezone::NAIVE))
            }),
            TimeUnit::Nanosecond => primitive!(TimestampNanosecondArray, |value| {
                Scalar::DateTime64(value, *unit, zone.clone().unwrap_or(Timezone::NAIVE))
            }),
            _ => return Err(unsupported(data_type, "invalid timestamp unit")),
        },
        DataType::Date32 => primitive!(Date32Array, |value| {
            Scalar::Date32(value, TimeUnit::Day, Timezone::NAIVE)
        }),
        DataType::Date64 => primitive!(Date64Array, |value| {
            Scalar::Date64(value, TimeUnit::Millisecond, Timezone::NAIVE)
        }),
        DataType::Time32(unit) => match unit {
            TimeUnit::Second => {
                primitive!(Time32SecondArray, |value| Scalar::Time32(
                    value,
                    *unit,
                    Timezone::NAIVE
                ))
            }
            TimeUnit::Millisecond => primitive!(Time32MillisecondArray, |value| Scalar::Time32(
                value,
                *unit,
                Timezone::NAIVE
            )),
            _ => return Err(unsupported(data_type, "invalid time32 unit")),
        },
        DataType::Time64(unit) => match unit {
            TimeUnit::Microsecond => {
                primitive!(Time64MicrosecondArray, |value| Scalar::Time64(
                    value,
                    *unit,
                    Timezone::NAIVE
                ))
            }
            TimeUnit::Nanosecond => {
                primitive!(Time64NanosecondArray, |value| Scalar::Time64(
                    value,
                    *unit,
                    Timezone::NAIVE
                ))
            }
            _ => return Err(unsupported(data_type, "invalid time64 unit")),
        },
        DataType::Duration32(unit) => duration32_from_array(array, index, *unit)?,
        DataType::Duration64(unit) => match unit {
            TimeUnit::Second => primitive!(DurationSecondArray, |value| {
                Scalar::Duration64(value, *unit, Timezone::NAIVE)
            }),
            TimeUnit::Millisecond => primitive!(DurationMillisecondArray, |value| {
                Scalar::Duration64(value, *unit, Timezone::NAIVE)
            }),
            TimeUnit::Microsecond => primitive!(DurationMicrosecondArray, |value| {
                Scalar::Duration64(value, *unit, Timezone::NAIVE)
            }),
            TimeUnit::Nanosecond => primitive!(DurationNanosecondArray, |value| {
                Scalar::Duration64(value, *unit, Timezone::NAIVE)
            }),
            _ => return Err(unsupported(data_type, "invalid duration64 unit")),
        },
        DataType::Interval(TimeUnit::YearMonth) => {
            primitive!(IntervalYearMonthArray, Scalar::from)
        }
        DataType::Interval(TimeUnit::DayTime) => {
            let value = downcast::<IntervalDayTimeArray>(array)?.value(index);
            Scalar::from_sequence([Scalar::from(value.days), Scalar::from(value.milliseconds)])
        }
        DataType::Interval(TimeUnit::MonthDayNano) => {
            let value = downcast::<IntervalMonthDayNanoArray>(array)?.value(index);
            Scalar::from_sequence([
                Scalar::from(value.months),
                Scalar::from(value.days),
                Scalar::from(value.nanoseconds),
            ])
        }
        DataType::Interval(_) => return Err(unsupported(data_type, "invalid interval layout")),
        DataType::Binary => Scalar::from(downcast::<BinaryArray>(array)?.value(index).to_vec()),
        DataType::FixedSizeBinary(_) => Scalar::from(
            downcast::<FixedSizeBinaryArray>(array)?
                .value(index)
                .to_vec(),
        ),
        DataType::LargeBinary => {
            Scalar::from(downcast::<LargeBinaryArray>(array)?.value(index).to_vec())
        }
        DataType::BinaryView => {
            Scalar::from(downcast::<BinaryViewArray>(array)?.value(index).to_vec())
        }
        DataType::Utf8 => Scalar::from(downcast::<StringArray>(array)?.value(index)),
        DataType::LargeUtf8 => Scalar::from(downcast::<LargeStringArray>(array)?.value(index)),
        DataType::Utf8View => Scalar::from(downcast::<StringViewArray>(array)?.value(index)),
        DataType::List(child) => {
            list_value(child, downcast::<ListArray>(array)?.value(index).as_ref())?
        }
        DataType::ListView(child) => list_value(
            child,
            downcast::<ListViewArray>(array)?.value(index).as_ref(),
        )?,
        DataType::FixedSizeList(child, _) => list_value(
            child,
            downcast::<FixedSizeListArray>(array)?.value(index).as_ref(),
        )?,
        DataType::LargeList(child) => list_value(
            child,
            downcast::<LargeListArray>(array)?.value(index).as_ref(),
        )?,
        DataType::LargeListView(child) => list_value(
            child,
            downcast::<LargeListViewArray>(array)?.value(index).as_ref(),
        )?,
        DataType::Struct(fields) => {
            let array = downcast::<StructArray>(array)?;
            let values = fields
                .iter()
                .zip(array.columns())
                .map(|(field, child)| value_from_array(field.data_type(), child.as_ref(), index))
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
                field.data_type(),
                array.child(type_id).as_ref(),
                array.value_offset(index),
            )?;
            Scalar::from_sequence([Scalar::from(i64::from(type_id)), payload])
        }
        DataType::Dictionary(dictionary) => dictionary_value(dictionary, array, index)?,
        DataType::Decimal32 { scale, .. } => {
            let value = downcast::<Decimal32Array>(array)?.value(index);
            Scalar::d128(i128::from(value), *scale)
        }
        DataType::Decimal64 { scale, .. } => {
            let value = downcast::<Decimal64Array>(array)?.value(index);
            Scalar::d128(i128::from(value), *scale)
        }
        DataType::Decimal128 { scale, .. } => {
            Scalar::d128(downcast::<Decimal128Array>(array)?.value(index), *scale)
        }
        DataType::Decimal256 { scale, .. } => {
            let value = downcast::<Decimal256Array>(array)?.value(index);
            Scalar::d256(I256::from_le_bytes(value.to_le_bytes()), *scale)
        }
        DataType::Map(map) => {
            let entries = downcast::<MapArray>(array)?.value(index);
            let fields = map
                .entries()
                .data_type()
                .as_fields()
                .ok_or_else(|| unsupported(data_type, "map entries are not a struct"))?;
            let pairs = (0..entries.len())
                .map(|entry| {
                    Ok((
                        value_from_array(fields[0].data_type(), entries.column(0).as_ref(), entry)?,
                        value_from_array(fields[1].data_type(), entries.column(1).as_ref(), entry)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            Scalar::from_mapping(pairs)?
        }
        DataType::RunEndEncoded(encoded) => run_value(encoded, array, index)?,
        // A geospatial column reads back in its canonical value spelling.
        DataType::Geometry(_) | DataType::Geography(_) => {
            Scalar::Geospatial(Arc::from(downcast::<BinaryArray>(array)?.value(index)))
        }
        DataType::Variant => {
            return Err(unsupported(
                data_type,
                "the variant binary encoding lands with the Iceberg v3 layer",
            ));
        }
    };
    Ok(value)
}

fn nulls(validity: Vec<bool>) -> Option<NullBuffer> {
    validity.iter().any(|valid| !valid).then(|| validity.into())
}

// Composite Arrow layouts can turn one logical null or inactive union member
// into a large number of mandatory physical child slots. Keep the same
// conservative limits as the core default planner, but account for the Arrow
// buffers that the logical Scalar tree does not own.
const MAX_PHYSICAL_SLOTS: usize = 1_000_000;
const MAX_PHYSICAL_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct MaterializationMark {
    slots: usize,
    fixed_bytes: usize,
}

#[derive(Default)]
pub(crate) struct MaterializationBudget {
    slots: usize,
    fixed_bytes: usize,
}

impl MaterializationBudget {
    /// Captures the retained allocation total before a temporary phase.
    pub(crate) fn mark(&self) -> MaterializationMark {
        MaterializationMark {
            slots: self.slots,
            fixed_bytes: self.fixed_bytes,
        }
    }

    /// Releases reservations whose allocations cannot outlive a completed phase.
    pub(crate) fn restore(&mut self, mark: MaterializationMark) {
        self.slots = mark.slots;
        self.fixed_bytes = mark.fixed_bytes;
    }

    pub(crate) fn add_bitmap(&mut self, rows: usize) -> Result<()> {
        self.add_bytes(bitmap_bytes(rows)?)
    }

    pub(crate) fn add_repeated_default(&mut self, data_type: &DataType, rows: usize) -> Result<()> {
        self.add_repeated_default_impl(data_type, rows, true)
    }

    pub(crate) fn add_repeated_default_without_dictionary_values(
        &mut self,
        data_type: &DataType,
        rows: usize,
    ) -> Result<()> {
        self.add_repeated_default_impl(data_type, rows, false)
    }

    fn add_repeated_default_impl(
        &mut self,
        data_type: &DataType,
        rows: usize,
        include_dictionary_values: bool,
    ) -> Result<()> {
        if rows == 0 {
            return Ok(());
        }
        match data_type {
            DataType::FixedSizeList(child, size) => {
                self.add_array_layout(data_type, rows)?;
                let size = usize::try_from(*size)
                    .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                let child_rows =
                    checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                self.add_repeated_field_default(child, child_rows, include_dictionary_values)
            }
            DataType::Struct(fields) => {
                self.add_array_layout(data_type, rows)?;
                for field in fields {
                    self.add_repeated_field_default(field, rows, include_dictionary_values)?;
                }
                Ok(())
            }
            DataType::Union(fields, mode) => {
                self.add_array_layout(data_type, rows)?;
                let (selected_id, _) = physical_union_branch(data_type, fields)?;
                for (type_id, field) in fields {
                    if matches!(mode, UnionMode::Dense) && type_id != selected_id {
                        continue;
                    }
                    if type_id == selected_id {
                        self.add_repeated_field_default(field, rows, include_dictionary_values)?;
                    } else {
                        self.add_null_array(field.data_type(), rows)?;
                    }
                }
                Ok(())
            }
            DataType::Dictionary(dictionary) => {
                self.add_array_layout(data_type, rows)?;
                if !include_dictionary_values
                    || dictionary.value().is_default_value(&Scalar::Null)?
                {
                    Ok(())
                } else {
                    self.add_repeated_default_impl(dictionary.value(), 1, true)
                }
            }
            DataType::RunEndEncoded(encoded) => {
                self.add_slots(1)?;
                self.add_array(encoded.run_ends().data_type(), 1)?;
                self.add_repeated_field_default(encoded.values(), 1, include_dictionary_values)
            }
            _ => self.add_array(data_type, rows),
        }
    }

    fn add_repeated_field_default(
        &mut self,
        field: &Field,
        rows: usize,
        include_dictionary_values: bool,
    ) -> Result<()> {
        if field.is_nullable() {
            self.add_null_array(field.data_type(), rows)
        } else {
            self.add_repeated_default_impl(field.data_type(), rows, include_dictionary_values)
        }
    }

    /// Reserves a live one-row default without charging its reusable logical
    /// root slot twice. Physical descendants and every owned root buffer are
    /// still charged because a deeply nested scalar can itself reach a cap.
    pub(crate) fn add_default_scalar_scratch(&mut self, data_type: &DataType) -> Result<()> {
        match data_type {
            DataType::FixedSizeList(child, size) => {
                self.add_array_layout_without_slots(data_type, 1)?;
                let size = usize::try_from(*size)
                    .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                self.add_repeated_field_default(child, size, true)
            }
            DataType::Struct(fields) => {
                self.add_array_layout_without_slots(data_type, 1)?;
                for field in fields {
                    self.add_repeated_field_default(field, 1, true)?;
                }
                Ok(())
            }
            DataType::Union(fields, mode) => {
                self.add_array_layout_without_slots(data_type, 1)?;
                let (selected_id, _) = physical_union_branch(data_type, fields)?;
                for (type_id, field) in fields {
                    if matches!(mode, UnionMode::Dense) && type_id != selected_id {
                        continue;
                    }
                    if type_id == selected_id {
                        self.add_repeated_field_default(field, 1, true)?;
                    } else {
                        self.add_null_array(field.data_type(), 1)?;
                    }
                }
                Ok(())
            }
            DataType::Dictionary(dictionary) => {
                self.add_array_layout_without_slots(data_type, 1)?;
                if dictionary.value().is_default_value(&Scalar::Null)? {
                    Ok(())
                } else {
                    self.add_repeated_default(dictionary.value(), 1)
                }
            }
            DataType::RunEndEncoded(encoded) => {
                self.add_array(encoded.run_ends().data_type(), 1)?;
                self.add_repeated_field_default(encoded.values(), 1, true)
            }
            _ => self.add_array_without_root_slots(data_type, 1),
        }
    }

    pub(crate) fn add_array(&mut self, data_type: &DataType, rows: usize) -> Result<()> {
        self.add_array_impl(data_type, rows, true)
    }

    fn add_array_without_root_slots(&mut self, data_type: &DataType, rows: usize) -> Result<()> {
        self.add_array_impl(data_type, rows, false)
    }

    fn add_array_impl(
        &mut self,
        data_type: &DataType,
        rows: usize,
        count_root_slots: bool,
    ) -> Result<()> {
        if rows == 0 {
            return Ok(());
        }
        self.add_array_layout_impl(data_type, rows, count_root_slots)?;
        match data_type {
            DataType::FixedSizeList(child, size) => {
                let size = usize::try_from(*size)
                    .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                let child_rows =
                    checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                self.add_array(child.data_type(), child_rows)?;
            }
            DataType::Struct(fields) => {
                for field in fields {
                    self.add_array(field.data_type(), rows)?;
                }
            }
            DataType::Union(fields, mode) => {
                if matches!(mode, UnionMode::Sparse) {
                    // Sparse unions require every child at the parent length.
                    for (_, field) in fields {
                        self.add_array(field.data_type(), rows)?;
                    }
                } else {
                    // Hidden dense-union fillers all use the core canonical
                    // default branch when one exists. A logically uninhabited
                    // union can still occupy a slot masked by an ancestor;
                    // select its first physically bounded branch without
                    // visiting inactive payloads. Charge exactly that child
                    // into the shared aggregate budget.
                    let (_, field) = physical_union_branch(data_type, fields)?;
                    self.add_array(field.data_type(), rows)?;
                }
            }
            DataType::Dictionary(dictionary) => {
                // There can be at most one distinct dictionary value per row.
                self.add_array(dictionary.value(), rows)?;
            }
            DataType::RunEndEncoded(encoded) => {
                // Both physical children contain at most one slot per logical
                // row. Recurse so wide value storage and nested wrappers join
                // the same aggregate budget before either child is built.
                self.add_array(encoded.run_ends().data_type(), rows)?;
                self.add_array(encoded.values().data_type(), rows)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Reserves only buffers owned by an array's outer layout.
    ///
    /// Selection kernels can share some children and compact others. Keeping
    /// the shallow reservation separate lets those callers charge the actual
    /// selected child rows without pessimistically charging hidden payloads.
    pub(crate) fn add_array_layout(&mut self, data_type: &DataType, rows: usize) -> Result<()> {
        self.add_array_layout_impl(data_type, rows, true)
    }

    fn add_array_layout_without_slots(&mut self, data_type: &DataType, rows: usize) -> Result<()> {
        self.add_array_layout_impl(data_type, rows, false)
    }

    fn add_array_layout_impl(
        &mut self,
        data_type: &DataType,
        rows: usize,
        count_slots: bool,
    ) -> Result<()> {
        if rows == 0 {
            return Ok(());
        }
        if count_slots {
            self.add_slots(rows)?;
        }
        // The constructors in this module use nullable builders even when a
        // particular batch contains no null, so reserve the worst-case bitmap
        // as fixed physical overhead.
        self.add_bytes(bitmap_bytes(rows)?)?;

        match data_type {
            DataType::Boolean => self.add_bytes(bitmap_bytes(rows)?)?,
            DataType::Int8 | DataType::UInt8 => self.add_fixed_rows(rows, 1)?,
            DataType::Int16 | DataType::UInt16 | DataType::Float16 => {
                self.add_fixed_rows(rows, 2)?;
            }
            DataType::Int32
            | DataType::UInt32
            | DataType::Float32
            | DataType::Date32
            | DataType::Time32(_)
            | DataType::Interval(TimeUnit::YearMonth)
            | DataType::Decimal32 { .. } => self.add_fixed_rows(rows, 4)?,
            DataType::Int64
            | DataType::UInt64
            | DataType::Float64
            | DataType::Timestamp(..)
            | DataType::Date64
            | DataType::Time64(_)
            | DataType::Duration32(_)
            | DataType::Duration64(_)
            | DataType::Interval(TimeUnit::DayTime)
            | DataType::Decimal64 { .. }
            | DataType::ListView(_) => self.add_fixed_rows(rows, 8)?,
            DataType::Interval(TimeUnit::MonthDayNano)
            | DataType::Decimal128 { .. }
            | DataType::BinaryView
            | DataType::Utf8View
            | DataType::LargeListView(_) => {
                self.add_fixed_rows(rows, 16)?;
            }
            DataType::Decimal256 { .. } => self.add_fixed_rows(rows, 32)?,
            DataType::Interval(_) => {
                return Err(unsupported(data_type, "invalid interval layout"));
            }
            DataType::Binary
            | DataType::Utf8
            | DataType::List(_)
            | DataType::Map(_)
            // A geospatial column is one binary column of WKB payloads.
            | DataType::Geometry(_)
            | DataType::Geography(_) => {
                self.add_offsets(rows, 4)?;
            }
            // The variant's storage is two required binary children, so the
            // worst-case buffer charge is two offset runs.
            DataType::Variant => {
                self.add_offsets(rows, 4)?;
                self.add_offsets(rows, 4)?;
            }
            DataType::LargeBinary | DataType::LargeUtf8 | DataType::LargeList(_) => {
                self.add_offsets(rows, 8)?;
            }
            DataType::FixedSizeBinary(width) => {
                let width = usize::try_from(*width)
                    .map_err(|_| invalid_value("a fixed binary width within usize", width))?;
                self.add_fixed_rows(rows, width)?;
            }
            DataType::Null
            | DataType::FixedSizeList(..)
            | DataType::Struct(_)
            | DataType::RunEndEncoded(_) => {}
            DataType::Union(_, mode) => self.add_union_buffers(rows, *mode)?,
            DataType::Dictionary(dictionary) => {
                self.add_fixed_rows(rows, integer_width(dictionary.key())?)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // Mirrors every Arrow null-array physical layout.
    pub(crate) fn add_null_array(&mut self, data_type: &DataType, rows: usize) -> Result<()> {
        self.add_null_array_impl(data_type, rows, true)
    }

    /// Reserves a one-row physical null placeholder while excluding its root
    /// slot, which is already represented by the eventual output row.
    pub(crate) fn add_null_scalar_scratch(&mut self, data_type: &DataType) -> Result<()> {
        self.add_null_array_impl(data_type, 1, false)
    }

    #[allow(clippy::too_many_lines)] // Mirrors every Arrow null-array physical layout.
    fn add_null_array_impl(
        &mut self,
        data_type: &DataType,
        rows: usize,
        count_root_slots: bool,
    ) -> Result<()> {
        if rows == 0 {
            return Ok(());
        }
        if count_root_slots {
            self.add_slots(rows)?;
        }
        self.add_bytes(bitmap_bytes(rows)?)?;
        match data_type {
            DataType::Null => {}
            DataType::Boolean => self.add_bytes(bitmap_bytes(rows)?)?,
            DataType::Int8 | DataType::UInt8 => self.add_fixed_rows(rows, 1)?,
            DataType::Int16 | DataType::UInt16 | DataType::Float16 => {
                self.add_fixed_rows(rows, 2)?;
            }
            DataType::Int32
            | DataType::UInt32
            | DataType::Float32
            | DataType::Date32
            | DataType::Time32(_)
            | DataType::Interval(TimeUnit::YearMonth)
            | DataType::Decimal32 { .. } => self.add_fixed_rows(rows, 4)?,
            DataType::Int64
            | DataType::UInt64
            | DataType::Float64
            | DataType::Timestamp(..)
            | DataType::Date64
            | DataType::Time64(_)
            | DataType::Duration32(_)
            | DataType::Duration64(_)
            | DataType::Interval(TimeUnit::DayTime)
            | DataType::Decimal64 { .. }
            | DataType::ListView(_) => self.add_fixed_rows(rows, 8)?,
            DataType::Interval(TimeUnit::MonthDayNano)
            | DataType::Decimal128 { .. }
            | DataType::BinaryView
            | DataType::Utf8View
            | DataType::LargeListView(_) => self.add_fixed_rows(rows, 16)?,
            DataType::Decimal256 { .. } => self.add_fixed_rows(rows, 32)?,
            DataType::Interval(_) => {
                return Err(unsupported(data_type, "invalid interval layout"));
            }
            DataType::Binary
            | DataType::Utf8
            | DataType::List(_)
            | DataType::Map(_)
            // A geospatial column is one binary column of WKB payloads.
            | DataType::Geometry(_)
            | DataType::Geography(_) => {
                self.add_offsets(rows, 4)?;
            }
            // The variant's storage is two required binary children, so the
            // worst-case buffer charge is two offset runs.
            DataType::Variant => {
                self.add_offsets(rows, 4)?;
                self.add_offsets(rows, 4)?;
            }
            DataType::LargeBinary | DataType::LargeUtf8 | DataType::LargeList(_) => {
                self.add_offsets(rows, 8)?;
            }
            DataType::FixedSizeBinary(width) => {
                let width = usize::try_from(*width)
                    .map_err(|_| invalid_value("a fixed binary width within usize", width))?;
                self.add_fixed_rows(rows, width)?;
            }
            DataType::FixedSizeList(child, size) => {
                let size = usize::try_from(*size)
                    .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                let child_rows =
                    checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                self.add_null_array(child.data_type(), child_rows)?;
            }
            DataType::Struct(fields) => {
                for field in fields {
                    self.add_null_array(field.data_type(), rows)?;
                }
            }
            DataType::Union(fields, mode) => {
                self.add_union_buffers(rows, *mode)?;
                match mode {
                    UnionMode::Sparse => {
                        for (_, field) in fields {
                            self.add_null_array(field.data_type(), rows)?;
                        }
                    }
                    UnionMode::Dense => {
                        let (_, field) = physical_union_branch(data_type, fields)?;
                        self.add_null_array(field.data_type(), rows)?;
                    }
                }
            }
            DataType::Dictionary(dictionary) => {
                self.add_fixed_rows(rows, integer_width(dictionary.key())?)?;
            }
            DataType::RunEndEncoded(encoded) => {
                let maximum = match encoded.run_ends().data_type() {
                    DataType::Int16 => i16::MAX as usize,
                    DataType::Int32 => i32::MAX as usize,
                    DataType::Int64 => usize::MAX,
                    data_type => return Err(unsupported(data_type, "invalid run-end type")),
                };
                if rows > maximum {
                    return Err(physical_limit_error("run-end value", rows, maximum));
                }
                self.add_array(encoded.run_ends().data_type(), 1)?;
                self.add_null_array(encoded.values().data_type(), 1)?;
            }
        }
        Ok(())
    }

    fn add_union_buffers(&mut self, rows: usize, mode: UnionMode) -> Result<()> {
        self.add_fixed_rows(rows, 1)?;
        if matches!(mode, UnionMode::Dense) {
            self.add_fixed_rows(rows, 4)?;
        }
        Ok(())
    }

    fn add_offsets(&mut self, rows: usize, width: usize) -> Result<()> {
        let offsets = rows
            .checked_add(1)
            .ok_or_else(|| physical_limit_error("offset count", rows, MAX_PHYSICAL_SLOTS))?;
        self.add_fixed_rows(offsets, width)
    }

    fn add_fixed_rows(&mut self, rows: usize, width: usize) -> Result<()> {
        self.add_bytes(checked_physical_mul(
            rows,
            width,
            "fixed buffer bytes",
            MAX_PHYSICAL_BYTES,
        )?)
    }

    fn add_slots(&mut self, slots: usize) -> Result<()> {
        self.slots = self.slots.checked_add(slots).ok_or_else(|| {
            physical_limit_error(
                "expanded slots",
                self.slots.saturating_add(slots),
                MAX_PHYSICAL_SLOTS,
            )
        })?;
        if self.slots > MAX_PHYSICAL_SLOTS {
            return Err(physical_limit_error(
                "expanded slots",
                self.slots,
                MAX_PHYSICAL_SLOTS,
            ));
        }
        Ok(())
    }

    pub(crate) fn add_physical_slots(&mut self, slots: usize) -> Result<()> {
        self.add_slots(slots)
    }

    pub(crate) fn add_bytes(&mut self, bytes: usize) -> Result<()> {
        self.fixed_bytes = self.fixed_bytes.checked_add(bytes).ok_or_else(|| {
            physical_limit_error(
                "fixed bytes",
                self.fixed_bytes.saturating_add(bytes),
                MAX_PHYSICAL_BYTES,
            )
        })?;
        if self.fixed_bytes > MAX_PHYSICAL_BYTES {
            return Err(physical_limit_error(
                "fixed bytes",
                self.fixed_bytes,
                MAX_PHYSICAL_BYTES,
            ));
        }
        Ok(())
    }
}

fn bitmap_bytes(rows: usize) -> Result<usize> {
    rows.checked_add(7)
        .map(|bits| bits / 8)
        .ok_or_else(|| physical_limit_error("bitmap bytes", rows, MAX_PHYSICAL_BYTES))
}

fn checked_physical_mul(
    left: usize,
    right: usize,
    kind: &'static str,
    limit: usize,
) -> Result<usize> {
    left.checked_mul(right)
        .ok_or_else(|| physical_limit_error(kind, left.saturating_mul(right), limit))
}

fn integer_width(data_type: &DataType) -> Result<usize> {
    match data_type {
        DataType::Int8 | DataType::UInt8 => Ok(1),
        DataType::Int16 | DataType::UInt16 => Ok(2),
        DataType::Int32 | DataType::UInt32 => Ok(4),
        DataType::Int64 | DataType::UInt64 => Ok(8),
        other => Err(unsupported(
            other,
            format!(
                "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {other}"
            ),
        )),
    }
}

fn physical_limit_error(kind: &'static str, actual: usize, limit: usize) -> Error {
    Error::physical_limit(kind, actual, limit)
}

fn physical_union_branch<'a>(
    data_type: &DataType,
    fields: &'a crate::UnionFields,
) -> Result<(i8, &'a Field)> {
    if let Ok(Some(selected)) = data_type.default_union_type_id() {
        if let Some((type_id, field)) = fields.iter().find(|(type_id, _)| *type_id == selected) {
            return Ok((type_id, field));
        }
    }

    let mut first_error = None;
    for (type_id, field) in fields {
        let mut probe = MaterializationBudget::default();
        match probe.add_array(field.data_type(), 1) {
            Ok(()) => return Ok((type_id, field)),
            Err(error) => first_error.get_or_insert(error),
        };
    }
    Err(first_error.unwrap_or_else(|| Error::internal("union_array::no_physical_branch")))
}

#[derive(Clone, Copy)]
enum ListKind {
    List,
    LargeList,
    ListView,
    LargeListView,
}

trait Offset: arrow_array::OffsetSizeTrait + TryFrom<usize> {}
impl Offset for i32 {}
impl Offset for i64 {}

type ListParts<'a, O> = (Vec<O>, Vec<O>, Vec<&'a Scalar>, Option<NullBuffer>);

fn list_parts<'a, O: Offset>(values: &'a [&Scalar]) -> Result<ListParts<'a, O>> {
    let mut offsets = Vec::with_capacity(values.len() + 1);
    let mut sizes = Vec::with_capacity(values.len());
    let mut flattened = Vec::new();
    let mut validity = Vec::with_capacity(values.len());
    offsets.push(
        O::try_from(0).map_err(|_| invalid_value("a list offset within the offset type", 0))?,
    );
    for value in values {
        if matches!(value, Scalar::Null) {
            validity.push(false);
            sizes.push(
                O::try_from(0)
                    .map_err(|_| invalid_value("a list size within the offset type", 0))?,
            );
        } else {
            let items = value
                .as_sequence()
                .ok_or_else(|| invalid_value_kind("a sequence for a list column", value))?;
            validity.push(true);
            sizes.push(
                O::try_from(items.len()).map_err(|_| {
                    invalid_value("a list size within the offset type", items.len())
                })?,
            );
            flattened.extend(items);
        }
        offsets.push(
            O::try_from(flattened.len()).map_err(|_| {
                invalid_value("a list offset within the offset type", flattened.len())
            })?,
        );
    }
    Ok((offsets, sizes, flattened, nulls(validity)))
}

fn list_array<O: Offset>(child: &Field, values: &[&Scalar], kind: ListKind) -> Result<ArrayRef> {
    let (offsets, _, flattened, nulls) = list_parts::<O>(values)?;
    let child_array = array_from_values(child, &flattened)?;
    let child = child.clone().into_arrow_ref()?;
    let offsets = OffsetBuffer::new(ScalarBuffer::from(offsets));
    match kind {
        ListKind::List => Ok(Arc::new(ListArray::try_new(
            child,
            cast_offsets(&offsets)?,
            child_array,
            nulls,
        )?)),
        ListKind::LargeList => Ok(Arc::new(LargeListArray::try_new(
            child,
            cast_offsets(&offsets)?,
            child_array,
            nulls,
        )?)),
        _ => Err(Error::internal("list_array::list_kind")),
    }
}

fn list_view_array<O: Offset>(
    child: &Field,
    values: &[&Scalar],
    kind: ListKind,
) -> Result<ArrayRef> {
    let (offsets, sizes, flattened, nulls) = list_parts::<O>(values)?;
    let offsets = offsets.into_iter().take(values.len()).collect::<Vec<_>>();
    let child_array = array_from_values(child, &flattened)?;
    let child = child.clone().into_arrow_ref()?;
    match kind {
        ListKind::ListView => Ok(Arc::new(ListViewArray::try_new(
            child,
            cast_scalar(offsets)?,
            cast_scalar(sizes)?,
            child_array,
            nulls,
        )?)),
        ListKind::LargeListView => Ok(Arc::new(LargeListViewArray::try_new(
            child,
            cast_scalar(offsets)?,
            cast_scalar(sizes)?,
            child_array,
            nulls,
        )?)),
        _ => Err(Error::internal("list_view_array::list_kind")),
    }
}

fn cast_offsets<O: Offset, T: Offset>(value: &OffsetBuffer<O>) -> Result<OffsetBuffer<T>> {
    let values = value
        .iter()
        .map(|value| {
            T::try_from(value.as_usize()).map_err(|_| {
                invalid_value("a list offset within the offset type", value.as_usize())
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(OffsetBuffer::new(ScalarBuffer::from(values)))
}

fn cast_scalar<O: Offset, T: Offset>(values: Vec<O>) -> Result<ScalarBuffer<T>> {
    values
        .into_iter()
        .map(|value| {
            T::try_from(value.as_usize()).map_err(|_| {
                invalid_value("a list offset within the offset type", value.as_usize())
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(ScalarBuffer::from)
}

fn fixed_size_list_array(child: &Field, size: i32, values: &[&Scalar]) -> Result<ArrayRef> {
    let size_usize =
        usize::try_from(size).map_err(|_| invalid_value("a fixed list size within usize", size))?;
    let physical_len = values.len().checked_mul(size_usize).ok_or_else(|| {
        physical_limit_error("fixed-size-list slots", values.len(), MAX_PHYSICAL_SLOTS)
    })?;
    let null_rows = values
        .iter()
        .filter(|value| matches!(value, Scalar::Null))
        .count();
    let hidden_rows = checked_physical_mul(
        null_rows,
        size_usize,
        "fixed-size-list slots",
        MAX_PHYSICAL_SLOTS,
    )?;
    if hidden_rows != 0 {
        let mut budget = MaterializationBudget::default();
        budget.add_array(child.data_type(), hidden_rows)?;
    }

    let has_parent_null = null_rows != 0;
    let placeholder = has_parent_null
        .then(|| physical_placeholder_for_field(child))
        .transpose()?;
    let mut flattened = Vec::new();
    flattened
        .try_reserve_exact(physical_len)
        .map_err(|error| allocation_error("fixed-size-list child slots", physical_len, &error))?;
    let mut validity = Vec::with_capacity(values.len());
    for value in values {
        if matches!(value, Scalar::Null) {
            validity.push(false);
            let placeholder = placeholder
                .as_ref()
                .ok_or_else(|| Error::internal("fixed_size_list_array::null_placeholder"))?;
            flattened.extend(std::iter::repeat_n(placeholder, size_usize));
        } else {
            let items = value.as_sequence().ok_or_else(|| {
                invalid_value_kind("a sequence for a fixed-size-list column", value)
            })?;
            if items.len() != size_usize {
                return Err(invalid_value(
                    &format!("a fixed list of exactly {size_usize} items"),
                    items.len(),
                ));
            }
            validity.push(true);
            flattened.extend(items);
        }
    }
    let child_array = array_from_values(child, &flattened)?;
    Ok(Arc::new(FixedSizeListArray::try_new_with_length(
        child.clone().into_arrow_ref()?,
        size,
        child_array,
        nulls(validity),
        values.len(),
    )?))
}

fn struct_array(fields: &crate::Fields, values: &[&Scalar]) -> Result<ArrayRef> {
    let null_rows = values
        .iter()
        .filter(|value| matches!(value, Scalar::Null))
        .count();
    if null_rows != 0 {
        let mut budget = MaterializationBudget::default();
        for field in fields {
            budget.add_array(field.data_type(), null_rows)?;
        }
    }
    let has_parent_null = null_rows != 0;
    let validity = values
        .iter()
        .map(|value| !matches!(value, Scalar::Null))
        .collect::<Vec<_>>();
    let arrow_fields = fields
        .iter()
        .cloned()
        .map(Field::into_arrow_ref)
        .collect::<crate::Result<Vec<_>>>()?;
    if fields.is_empty() {
        return Ok(Arc::new(StructArray::new_empty_fields(
            values.len(),
            nulls(validity),
        )));
    }
    let columns = fields
        .iter()
        .enumerate()
        .map(|(column, field)| {
            let placeholder = has_parent_null
                .then(|| physical_placeholder_for_field(field))
                .transpose()?;
            let column_values = values
                .iter()
                .map(|value| {
                    if matches!(value, Scalar::Null) {
                        placeholder
                            .as_ref()
                            .ok_or_else(|| Error::internal("struct_array::null_placeholder"))
                    } else {
                        value
                            .as_sequence()
                            .and_then(|values| values.get(column))
                            .ok_or_else(|| {
                                invalid_value_kind("a sequence for a struct column", value)
                            })
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            array_from_values(field, &column_values)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Arc::new(StructArray::try_new_with_length(
        arrow_fields.into(),
        columns,
        nulls(validity),
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
            budget.add_array(field.data_type(), values.len())?;
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
    for value in values {
        let pair = value
            .as_sequence()
            .ok_or_else(|| invalid_value_kind("a union [type_id, payload] sequence", value))?;
        let [type_id, payload] = pair else {
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
        .map(|((_, field), values)| array_from_values(field, &values))
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
    let data_type = DataType::union(fields.iter().map(|(id, field)| (id, field.clone())), mode)?;
    data_type.into_arrow().map_err(Into::into)
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
    let dictionary_values = array_from_values(&value_field, &value_refs)?;
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

fn map_array(map: &crate::MapType, values: &[&Scalar]) -> Result<ArrayRef> {
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
        .data_type()
        .as_fields()
        .ok_or_else(|| Error::internal("map_array::entries_struct"))?;
    let keys = entries.iter().map(|(key, _)| key).collect::<Vec<_>>();
    let vals = entries.iter().map(|(_, value)| value).collect::<Vec<_>>();
    let entries_array = StructArray::try_new_with_length(
        vec![
            fields[0].clone().into_arrow_ref()?,
            fields[1].clone().into_arrow_ref()?,
        ]
        .into(),
        vec![
            array_from_values(&fields[0], &keys)?,
            array_from_values(&fields[1], &vals)?,
        ],
        None,
        entries.len(),
    )?;
    Ok(Arc::new(MapArray::try_new(
        map.entries().clone().into_arrow_ref()?,
        OffsetBuffer::new(ScalarBuffer::from(offsets)),
        entries_array,
        nulls(validity),
        map.keys_sorted(),
    )?))
}

fn run_array(encoded: &crate::RunEndEncodedType, values: &[&Scalar]) -> Result<ArrayRef> {
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
    let values_array = array_from_values(encoded.values(), &run_values)?;
    let arrow_type = ArrowDataType::RunEndEncoded(
        encoded.run_ends().clone().into_arrow_ref()?,
        encoded.values().clone().into_arrow_ref()?,
    );
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
    match encoded.run_ends().data_type() {
        DataType::Int16 => run!(i16, Int16RunArray),
        DataType::Int32 => run!(i32, Int32RunArray),
        DataType::Int64 => run!(i64, Int64RunArray),
        data_type => Err(unsupported(data_type, "invalid run-end type")),
    }
}

pub(crate) fn physical_placeholder_for_field(field: &Field) -> Result<Scalar> {
    // These values occupy physically required slots hidden by a parent null
    // bitmap or an inactive sparse-union type ID. They need a valid physical
    // representation, not a logically inhabitable value: a required Null
    // grandchild is legal when an ancestor masks the entire slot.
    physical_placeholder(field.data_type())
}

fn physical_placeholder(data_type: &DataType) -> Result<Scalar> {
    match data_type {
        DataType::Union(fields, _) => {
            let (type_id, field) = physical_union_branch(data_type, fields)?;
            Ok(Scalar::from_sequence([
                Scalar::I64(i64::from(type_id)),
                physical_placeholder(field.data_type())?,
            ]))
        }
        DataType::RunEndEncoded(encoded) => physical_placeholder(encoded.values().data_type()),
        // Every other Arrow layout owns a validity bitmap (or is Null
        // itself). Marking the hidden slot null lets that physical container
        // mask its own required descendants before its parent masks it in
        // turn, and avoids allocating a logical filler that is never visible.
        _ => Ok(Scalar::Null),
    }
}

fn list_value(field: &Field, array: &dyn Array) -> Result<Scalar> {
    (0..array.len())
        .map(|index| value_from_array(field.data_type(), array, index))
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
                encoded.values().data_type(),
                array.values().as_ref(),
                array.get_physical_index(index),
            )
        }};
    }
    match encoded.run_ends().data_type() {
        DataType::Int16 => run!(Int16Type, Int16RunArray),
        DataType::Int32 => run!(Int32Type, Int32RunArray),
        DataType::Int64 => run!(Int64Type, Int64RunArray),
        data_type => Err(unsupported(data_type, "invalid run-end type")),
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

fn duration32_from_array(array: &dyn Array, index: usize, unit: TimeUnit) -> Result<Scalar> {
    let count = match unit {
        TimeUnit::Second => downcast::<DurationSecondArray>(array)?.value(index),
        TimeUnit::Millisecond => downcast::<DurationMillisecondArray>(array)?.value(index),
        TimeUnit::Microsecond => downcast::<DurationMicrosecondArray>(array)?.value(index),
        TimeUnit::Nanosecond => downcast::<DurationNanosecondArray>(array)?.value(index),
        _ => {
            return Err(unsupported(
                &DataType::Duration32(unit),
                "invalid duration32 unit",
            ));
        }
    };
    let count = i32::try_from(count).map_err(|_| invalid_value("duration32", count))?;
    Scalar::duration32(count, unit).map_err(Into::into)
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
        Scalar::Bytes(bytes) => Ok(Some(bytes)),
        _ => Err(invalid_value_kind("bytes", value)),
    }
}

/// Read the WKB a geospatial column stores, in either value spelling.
fn optional_wkb(value: &Scalar) -> Result<Option<&[u8]>> {
    match value {
        Scalar::Null => Ok(None),
        Scalar::Geospatial(bytes) | Scalar::Bytes(bytes) => Ok(Some(bytes)),
        _ => Err(invalid_value_kind("well-known binary", value)),
    }
}

/// Read the text a string column stores, refusing anything that is not text.
fn optional_str(value: &Scalar) -> Result<Option<&str>> {
    match value {
        Scalar::Null => Ok(None),
        Scalar::String(text) => Ok(Some(text)),
        _ => Err(invalid_value_kind("string", value)),
    }
}

/// Read the boolean a boolean column stores, refusing anything that is not one.
fn optional_bool(value: &Scalar) -> Result<Option<bool>> {
    match value {
        Scalar::Null => Ok(None),
        Scalar::Bool(value) => Ok(Some(*value)),
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
    let items = value
        .as_sequence()
        .ok_or_else(|| invalid_value_kind("a [days, milliseconds] sequence", value))?;
    let [days, milliseconds] = items else {
        return Err(invalid_value(
            "a [days, milliseconds] sequence of exactly 2 items",
            items.len(),
        ));
    };
    Ok(IntervalDayTime::new(
        signed_i32(days)?,
        signed_i32(milliseconds)?,
    ))
}

fn interval_month_day_nano(value: &Scalar) -> Result<IntervalMonthDayNano> {
    let items = value
        .as_sequence()
        .ok_or_else(|| invalid_value_kind("a [months, days, nanoseconds] sequence", value))?;
    let [months, days, nanoseconds] = items else {
        return Err(invalid_value(
            "a [months, days, nanoseconds] sequence of exactly 3 items",
            items.len(),
        ));
    };
    Ok(IntervalMonthDayNano::new(
        signed_i32(months)?,
        signed_i32(days)?,
        signed_i64(nanoseconds)?,
    ))
}

fn decimal256(value: &Scalar, scale: i8) -> Result<i256> {
    value
        .decimal256_unscaled_at(scale)
        .map(|coefficient| i256::from_le_bytes(coefficient.into_le_bytes()))
        .ok_or_else(|| {
            invalid_value(
                &format!("a decimal256 representable at scale {scale}"),
                value.kind(),
            )
        })
}

fn unsupported(data_type: &DataType, reason: impl Into<String>) -> Error {
    Error::Unsupported {
        kind: data_type.name(),
        reason: reason.into(),
    }
}

fn invalid_value(expected: &str, actual: impl std::fmt::Display) -> Error {
    Error::InvalidValue {
        path: smol_str::SmolStr::new_static("$"),
        expected: smol_str::SmolStr::new(expected),
        actual: smol_str::format_smolstr!("{actual}"),
    }
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

#[cfg(test)]
mod tests;
