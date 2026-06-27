//! Builders for **fill** arrays used by [`Serie::resize`](crate::Serie::resize): a run
//! of nulls (for a nullable column) or of a type's **default** value (for a
//! non-nullable one — every datatype has a default: `false`, `0`, `0.0`, `""`, empty
//! bytes, a struct of defaults).

use std::sync::Arc;

use arrow_array::types::{
    Date32Type, Date64Type, Decimal128Type, Decimal256Type, DurationMicrosecondType,
    DurationMillisecondType, DurationNanosecondType, DurationSecondType, Float16Type, Float32Type,
    Float64Type, Int16Type, Int32Type, Int64Type, Int8Type, IntervalDayTimeType,
    IntervalMonthDayNanoType, IntervalYearMonthType, Time32MillisecondType, Time32SecondType,
    Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt16Type, UInt32Type,
    UInt64Type, UInt8Type,
};
use arrow_array::{
    new_null_array, ArrayRef, ArrowPrimitiveType, BinaryArray, BooleanArray, LargeBinaryArray,
    LargeStringArray, PrimitiveArray, StringArray, StructArray,
};
use arrow_buffer::i256;
use arrow_schema::{DataType as ADataType, IntervalUnit as AIntervalUnit, TimeUnit as ATimeUnit};
use yggdryl_schema::Field;

use crate::error::{SerieError, SerieResult};
use crate::serie::{from_arrow, SerieRef};

/// A run of `len` nulls of the exact Arrow type `dt`.
pub(crate) fn null_array(dt: &ADataType, len: usize) -> ArrayRef {
    new_null_array(dt, len)
}

/// A [`Serie`](crate::Serie) of `len` fill values for `field`: nulls if the field is
/// nullable, otherwise its type's default. Used to fill a missing column on
/// [`cast`](crate::Serie::cast).
pub(crate) fn fill_serie(field: &Field, len: usize) -> SerieResult<SerieRef> {
    let dt = field.data_type().to_arrow()?;
    let array = if field.is_nullable() {
        null_array(&dt, len)
    } else {
        default_array(&dt, len)?
    };
    from_arrow(field.clone(), array)
}

/// A `PrimitiveArray<A>` of `len` default (`0`) values.
fn prim_default<A: ArrowPrimitiveType>(len: usize) -> PrimitiveArray<A>
where
    A::Native: Default,
{
    PrimitiveArray::<A>::from_iter_values(std::iter::repeat_n(A::Native::default(), len))
}

/// A run of `len` **default** (non-null) values of the exact Arrow type `dt`.
pub(crate) fn default_array(dt: &ADataType, len: usize) -> SerieResult<ArrayRef> {
    let arrow_err = |e: arrow_schema::ArrowError| SerieError::Arrow(e.to_string());
    Ok(match dt {
        ADataType::Boolean => Arc::new(BooleanArray::from(vec![false; len])),
        ADataType::Int8 => Arc::new(prim_default::<Int8Type>(len)),
        ADataType::Int16 => Arc::new(prim_default::<Int16Type>(len)),
        ADataType::Int32 => Arc::new(prim_default::<Int32Type>(len)),
        ADataType::Int64 => Arc::new(prim_default::<Int64Type>(len)),
        ADataType::UInt8 => Arc::new(prim_default::<UInt8Type>(len)),
        ADataType::UInt16 => Arc::new(prim_default::<UInt16Type>(len)),
        ADataType::UInt32 => Arc::new(prim_default::<UInt32Type>(len)),
        ADataType::UInt64 => Arc::new(prim_default::<UInt64Type>(len)),
        ADataType::Float16 => Arc::new(prim_default::<Float16Type>(len)),
        ADataType::Float32 => Arc::new(prim_default::<Float32Type>(len)),
        ADataType::Float64 => Arc::new(prim_default::<Float64Type>(len)),
        ADataType::Decimal128(p, s) => Arc::new(
            PrimitiveArray::<Decimal128Type>::from_iter_values(std::iter::repeat_n(0i128, len))
                .with_precision_and_scale(*p, *s)
                .map_err(arrow_err)?,
        ),
        ADataType::Decimal256(p, s) => Arc::new(
            PrimitiveArray::<Decimal256Type>::from_iter_values(std::iter::repeat_n(
                i256::ZERO,
                len,
            ))
            .with_precision_and_scale(*p, *s)
            .map_err(arrow_err)?,
        ),
        ADataType::Date32 => Arc::new(prim_default::<Date32Type>(len)),
        ADataType::Date64 => Arc::new(prim_default::<Date64Type>(len)),
        ADataType::Time32(ATimeUnit::Second) => Arc::new(prim_default::<Time32SecondType>(len)),
        ADataType::Time32(ATimeUnit::Millisecond) => {
            Arc::new(prim_default::<Time32MillisecondType>(len))
        }
        ADataType::Time64(ATimeUnit::Microsecond) => {
            Arc::new(prim_default::<Time64MicrosecondType>(len))
        }
        ADataType::Time64(ATimeUnit::Nanosecond) => {
            Arc::new(prim_default::<Time64NanosecondType>(len))
        }
        ADataType::Timestamp(unit, tz) => {
            let tz = tz.clone();
            match unit {
                ATimeUnit::Second => {
                    Arc::new(prim_default::<TimestampSecondType>(len).with_timezone_opt(tz))
                }
                ATimeUnit::Millisecond => {
                    Arc::new(prim_default::<TimestampMillisecondType>(len).with_timezone_opt(tz))
                }
                ATimeUnit::Microsecond => {
                    Arc::new(prim_default::<TimestampMicrosecondType>(len).with_timezone_opt(tz))
                }
                ATimeUnit::Nanosecond => {
                    Arc::new(prim_default::<TimestampNanosecondType>(len).with_timezone_opt(tz))
                }
            }
        }
        ADataType::Duration(ATimeUnit::Second) => Arc::new(prim_default::<DurationSecondType>(len)),
        ADataType::Duration(ATimeUnit::Millisecond) => {
            Arc::new(prim_default::<DurationMillisecondType>(len))
        }
        ADataType::Duration(ATimeUnit::Microsecond) => {
            Arc::new(prim_default::<DurationMicrosecondType>(len))
        }
        ADataType::Duration(ATimeUnit::Nanosecond) => {
            Arc::new(prim_default::<DurationNanosecondType>(len))
        }
        ADataType::Interval(AIntervalUnit::YearMonth) => {
            Arc::new(prim_default::<IntervalYearMonthType>(len))
        }
        ADataType::Interval(AIntervalUnit::DayTime) => {
            Arc::new(prim_default::<IntervalDayTimeType>(len))
        }
        ADataType::Interval(AIntervalUnit::MonthDayNano) => {
            Arc::new(prim_default::<IntervalMonthDayNanoType>(len))
        }
        ADataType::Utf8 => Arc::new(StringArray::from_iter_values(std::iter::repeat_n("", len))),
        ADataType::LargeUtf8 => Arc::new(LargeStringArray::from_iter_values(std::iter::repeat_n(
            "", len,
        ))),
        ADataType::Binary => Arc::new(BinaryArray::from_iter_values(std::iter::repeat_n(
            Vec::<u8>::new(),
            len,
        ))),
        ADataType::LargeBinary => Arc::new(LargeBinaryArray::from_iter_values(
            std::iter::repeat_n(Vec::<u8>::new(), len),
        )),
        // A struct default is a record of each field's fill — nulls for a nullable child,
        // its type's default for a non-nullable one (built recursively).
        ADataType::Struct(fields) => {
            let arrays = fields
                .iter()
                .map(|f| {
                    if f.is_nullable() {
                        Ok(null_array(f.data_type(), len))
                    } else {
                        default_array(f.data_type(), len)
                    }
                })
                .collect::<SerieResult<Vec<_>>>()?;
            Arc::new(StructArray::try_new(fields.clone(), arrays, None).map_err(arrow_err)?)
        }
        other => {
            return Err(SerieError::Unsupported(format!(
                "no non-null default value for arrow type '{other}'; make the field nullable to \
                 resize it with nulls"
            )))
        }
    })
}
