//! [`Scalar`] — a single, type-erased value read out of a column by index, and the
//! [`scalar_at`] extractor that maps any backing Arrow array cell to one.

use std::fmt;

use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, Date64Array, Decimal128Array,
    Decimal256Array, DurationMicrosecondArray, DurationMillisecondArray, DurationNanosecondArray,
    DurationSecondArray, Float16Array, Float32Array, Float64Array, Int16Array, Int32Array,
    Int64Array, Int8Array, IntervalDayTimeArray, IntervalMonthDayNanoArray, IntervalYearMonthArray,
    LargeBinaryArray, LargeStringArray, StringArray, Time32MillisecondArray, Time32SecondArray,
    Time64MicrosecondArray, Time64NanosecondArray, TimestampMicrosecondArray,
    TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt16Array,
    UInt32Array, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType as ADataType, IntervalUnit as AIntervalUnit, TimeUnit as ATimeUnit};

/// A single value read from a column. Integers (of any width), decimals up to 128
/// bits and the temporal **physical** values widen losslessly into [`Int`](Scalar::Int);
/// floats into [`Float`](Scalar::Float). The few exotic physical types (256-bit
/// decimals, the day-time / month-day-nano interval structs) are rendered textually
/// into [`Other`](Scalar::Other) so no value is ever dropped or mislabelled.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    /// A null (or out-of-bounds) cell.
    Null,
    /// A boolean.
    Boolean(bool),
    /// Any integer, decimal128 or temporal physical value, widened to `i128`.
    Int(i128),
    /// Any float (`f16` / `f32` / `f64`), widened to `f64`.
    Float(f64),
    /// A string value.
    Utf8(String),
    /// A binary value.
    Binary(Vec<u8>),
    /// An exotic value rendered textually (256-bit decimal, interval structs).
    Other(String),
}

impl Scalar {
    /// The **default** value of a [`DataType`](yggdryl_schema::DataType): `false` for
    /// boolean, `0` for every integer / decimal / temporal physical, `0.0` for floats,
    /// the empty string for text (and `Json`), empty bytes for binary (and `Bson`), and
    /// [`Null`](Scalar::Null) for the null / nested / wildcard types (which have no
    /// scalar default).
    pub fn default_for(dtype: &yggdryl_schema::DataType) -> Scalar {
        if dtype.is_boolean() {
            Scalar::Boolean(false)
        } else if dtype.is_floating() {
            Scalar::Float(0.0)
        } else if dtype.is_integer() || dtype.is_decimal() || dtype.is_temporal() {
            Scalar::Int(0)
        } else if dtype.is_string() || dtype.is_json() {
            Scalar::Utf8(String::new())
        } else if dtype.is_binary() || dtype.is_bson() {
            Scalar::Binary(Vec::new())
        } else {
            Scalar::Null
        }
    }

    /// Whether this is the [`Null`](Scalar::Null) value.
    pub fn is_null(&self) -> bool {
        matches!(self, Scalar::Null)
    }

    /// The value as an `i128`, when it is an [`Int`](Scalar::Int).
    pub fn as_int(&self) -> Option<i128> {
        match self {
            Scalar::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// The value as an `f64`, when it is a [`Float`](Scalar::Float).
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Scalar::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// The value as a `&str`, when it is a [`Utf8`](Scalar::Utf8) string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Scalar::Utf8(v) => Some(v),
            _ => None,
        }
    }
}

impl fmt::Display for Scalar {
    /// A compact value rendering used by [`Serie::display`](crate::Serie::display): a
    /// null is `"null"`, a string its text, binary lowercase hex, and the rest their
    /// natural form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scalar::Null => f.write_str("null"),
            Scalar::Boolean(v) => write!(f, "{v}"),
            Scalar::Int(v) => write!(f, "{v}"),
            Scalar::Float(v) => write!(f, "{v}"),
            Scalar::Utf8(v) => f.write_str(v),
            Scalar::Binary(v) => {
                f.write_str("0x")?;
                for byte in v {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
            Scalar::Other(v) => f.write_str(v),
        }
    }
}

/// Reads the value of `array` at `index` into a [`Scalar`] (mapping `Null` for a null
/// cell or an out-of-bounds index).
pub(crate) fn scalar_at(array: &ArrayRef, index: usize) -> Scalar {
    if index >= array.len() || array.is_null(index) {
        return Scalar::Null;
    }

    /// Downcasts `array` to `$ty` and reads its value at `index`.
    macro_rules! val {
        ($ty:ty) => {
            array.as_any().downcast_ref::<$ty>().unwrap().value(index)
        };
    }

    match array.data_type() {
        ADataType::Boolean => Scalar::Boolean(val!(BooleanArray)),
        ADataType::Int8 => Scalar::Int(val!(Int8Array) as i128),
        ADataType::Int16 => Scalar::Int(val!(Int16Array) as i128),
        ADataType::Int32 => Scalar::Int(val!(Int32Array) as i128),
        ADataType::Int64 => Scalar::Int(val!(Int64Array) as i128),
        ADataType::UInt8 => Scalar::Int(val!(UInt8Array) as i128),
        ADataType::UInt16 => Scalar::Int(val!(UInt16Array) as i128),
        ADataType::UInt32 => Scalar::Int(val!(UInt32Array) as i128),
        ADataType::UInt64 => Scalar::Int(val!(UInt64Array) as i128),
        ADataType::Float16 => Scalar::Float(val!(Float16Array).to_f64()),
        ADataType::Float32 => Scalar::Float(val!(Float32Array) as f64),
        ADataType::Float64 => Scalar::Float(val!(Float64Array)),
        ADataType::Decimal128(_, _) => Scalar::Int(val!(Decimal128Array)),
        ADataType::Date32 => Scalar::Int(val!(Date32Array) as i128),
        ADataType::Date64 => Scalar::Int(val!(Date64Array) as i128),
        ADataType::Time32(ATimeUnit::Second) => Scalar::Int(val!(Time32SecondArray) as i128),
        ADataType::Time32(ATimeUnit::Millisecond) => {
            Scalar::Int(val!(Time32MillisecondArray) as i128)
        }
        ADataType::Time64(ATimeUnit::Microsecond) => {
            Scalar::Int(val!(Time64MicrosecondArray) as i128)
        }
        ADataType::Time64(ATimeUnit::Nanosecond) => {
            Scalar::Int(val!(Time64NanosecondArray) as i128)
        }
        ADataType::Timestamp(ATimeUnit::Second, _) => {
            Scalar::Int(val!(TimestampSecondArray) as i128)
        }
        ADataType::Timestamp(ATimeUnit::Millisecond, _) => {
            Scalar::Int(val!(TimestampMillisecondArray) as i128)
        }
        ADataType::Timestamp(ATimeUnit::Microsecond, _) => {
            Scalar::Int(val!(TimestampMicrosecondArray) as i128)
        }
        ADataType::Timestamp(ATimeUnit::Nanosecond, _) => {
            Scalar::Int(val!(TimestampNanosecondArray) as i128)
        }
        ADataType::Duration(ATimeUnit::Second) => Scalar::Int(val!(DurationSecondArray) as i128),
        ADataType::Duration(ATimeUnit::Millisecond) => {
            Scalar::Int(val!(DurationMillisecondArray) as i128)
        }
        ADataType::Duration(ATimeUnit::Microsecond) => {
            Scalar::Int(val!(DurationMicrosecondArray) as i128)
        }
        ADataType::Duration(ATimeUnit::Nanosecond) => {
            Scalar::Int(val!(DurationNanosecondArray) as i128)
        }
        ADataType::Interval(AIntervalUnit::YearMonth) => {
            Scalar::Int(val!(IntervalYearMonthArray) as i128)
        }
        ADataType::Utf8 => Scalar::Utf8(val!(StringArray).to_string()),
        ADataType::LargeUtf8 => Scalar::Utf8(val!(LargeStringArray).to_string()),
        ADataType::Binary => Scalar::Binary(val!(BinaryArray).to_vec()),
        ADataType::LargeBinary => Scalar::Binary(val!(LargeBinaryArray).to_vec()),
        ADataType::Decimal256(_, _) => Scalar::Other(val!(Decimal256Array).to_string()),
        ADataType::Interval(AIntervalUnit::DayTime) => {
            Scalar::Other(format!("{:?}", val!(IntervalDayTimeArray)))
        }
        ADataType::Interval(AIntervalUnit::MonthDayNano) => {
            Scalar::Other(format!("{:?}", val!(IntervalMonthDayNanoArray)))
        }
        ADataType::Null => Scalar::Null,
        other => Scalar::Other(format!("<{other}>")),
    }
}
