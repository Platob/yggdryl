//! The Apache Iceberg type vocabulary and its mapping to core datatypes.
//!
//! Iceberg names a small, closed set of primitive types in its table metadata
//! JSON. Each one has exactly one physical [`DataType`] here, and the mapping
//! is total in that direction: every Iceberg type reads back as a datatype
//! without loss. The other direction is not total - datatypes such as
//! `int8`, `interval`, or `union` have no Iceberg spelling - so writing
//! reports what cannot be represented rather than silently widening it.

use std::fmt;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, TimeUnit};

/// A primitive type from the Iceberg specification.
///
/// ```
/// use yggdryl::iceberg::PrimitiveType;
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let iceberg = PrimitiveType::from_str("decimal(18, 4)")?;
/// assert_eq!(iceberg.into_dtype()?, DataType::decimal(18, 4)?);
/// assert_eq!(iceberg.to_string(), "decimal(18, 4)");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum PrimitiveType {
    /// True or false.
    Boolean,
    /// 32-bit signed integer.
    Int,
    /// 64-bit signed integer.
    Long,
    /// 32-bit IEEE 754 float.
    Float,
    /// 64-bit IEEE 754 float.
    Double,
    /// Fixed-point decimal with a precision and scale.
    Decimal {
        /// Total digits, 1 through 38.
        precision: u8,
        /// Digits after the point.
        scale: i8,
    },
    /// Days since the Unix epoch.
    Date,
    /// Microseconds since midnight.
    Time,
    /// Microseconds since the Unix epoch, without a zone.
    Timestamp,
    /// Microseconds since the Unix epoch, in UTC.
    Timestamptz,
    /// Nanoseconds since the Unix epoch, without a zone. Added in v3.
    TimestampNs,
    /// Nanoseconds since the Unix epoch, in UTC. Added in v3.
    TimestamptzNs,
    /// A column whose type is not yet known, always null. Added in v3.
    Unknown,
    /// UTF-8 text.
    String,
    /// A 16-byte universally unique identifier.
    Uuid,
    /// A fixed-length byte array.
    Fixed(i32),
    /// A variable-length byte array.
    Binary,
}

impl PrimitiveType {
    /// Parse an Iceberg primitive type name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the vocabulary and the input.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the physical datatype this Iceberg type materializes into.
    ///
    /// # Errors
    ///
    /// Returns an error only when a decimal precision or scale is outside the
    /// range the core accepts.
    pub fn into_dtype(self) -> Result<DataType> {
        Ok(match self {
            Self::Boolean => DataType::Boolean,
            Self::Int => DataType::Int32,
            Self::Long => DataType::Int64,
            Self::Float => DataType::Float32,
            Self::Double => DataType::Float64,
            Self::Decimal { precision, scale } => DataType::decimal(precision, scale)?,
            Self::Date => DataType::Date32,
            // Iceberg fixes every temporal resolution at microseconds.
            Self::Time => DataType::time(TimeUnit::Microsecond)?,
            Self::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, None),
            Self::Timestamptz => {
                DataType::Timestamp(TimeUnit::Microsecond, Some(crate::Timezone::UTC))
            }
            Self::TimestampNs => DataType::Timestamp(TimeUnit::Nanosecond, None),
            Self::TimestamptzNs => {
                DataType::Timestamp(TimeUnit::Nanosecond, Some(crate::Timezone::UTC))
            }
            // An unknown column always reads as null, which is exactly Arrow's
            // null datatype rather than a placeholder of some other width.
            Self::Unknown => DataType::Null,
            Self::String => DataType::Utf8,
            // A UUID is a 16-byte fixed value on the wire.
            Self::Uuid => DataType::fixed_size_binary(16)?,
            Self::Fixed(width) => DataType::fixed_size_binary(width)?,
            Self::Binary => DataType::Binary,
        })
    }

    /// Return the Iceberg type a physical datatype maps onto.
    ///
    /// # Errors
    ///
    /// Returns an error naming the datatype when Iceberg has no spelling for
    /// it, so a caller can widen the schema deliberately instead of losing
    /// information silently.
    pub fn from_dtype(dtype: &DataType) -> Result<Self> {
        Ok(match dtype {
            DataType::Boolean => Self::Boolean,
            DataType::Int32 => Self::Int,
            DataType::Int64 => Self::Long,
            DataType::Float32 => Self::Float,
            DataType::Float64 => Self::Double,
            DataType::Decimal32 { precision, scale }
            | DataType::Decimal64 { precision, scale }
            | DataType::Decimal128 { precision, scale } => {
                // Arrow admits a negative scale; Iceberg's decimal grammar
                // does not, and Parquet rejects one at write time - so it is
                // refused here, before a schema carrying it is committed.
                if *scale < 0 {
                    return Err(Error::InvalidDataType {
                        kind: "iceberg",
                        reason: format_smolstr!(
                            "expected a decimal scale of 0 or more (Iceberg spells no negative \
                             scale), got decimal({precision}, {scale})"
                        ),
                    });
                }
                Self::Decimal {
                    precision: *precision,
                    scale: *scale,
                }
            }
            DataType::Date32 => Self::Date,
            DataType::Time64(TimeUnit::Microsecond) => Self::Time,
            DataType::Timestamp(TimeUnit::Microsecond, zone) => match zone {
                Some(_) => Self::Timestamptz,
                None => Self::Timestamp,
            },
            DataType::Timestamp(TimeUnit::Nanosecond, zone) => match zone {
                Some(_) => Self::TimestamptzNs,
                None => Self::TimestampNs,
            },
            DataType::Null => Self::Unknown,
            // An ASCII width is text; the padding is storage, never a value.
            DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Ascii32
            | DataType::Ascii64
            | DataType::Ascii128 => Self::String,
            DataType::FixedSizeBinary(16) => Self::Fixed(16),
            DataType::FixedSizeBinary(width) => Self::Fixed(*width),
            DataType::Binary | DataType::LargeBinary | DataType::BinaryView => Self::Binary,
            other => {
                return Err(Error::InvalidDataType {
                    kind: "iceberg",
                    reason: format_smolstr!(
                        "expected a datatype Iceberg can express (boolean, int, long, float, \
                         double, decimal, date, time, timestamp, timestamptz, timestamp_ns, \
                         timestamptz_ns, string, uuid, fixed, binary, unknown), got {other}; \
                         into_scheme_compat(&Scheme::ICEBERG) widens the ones that widen losslessly"
                    ),
                });
            }
        })
    }
}

impl FromStr for PrimitiveType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let trimmed = value.trim();
        match trimmed {
            "boolean" => return Ok(Self::Boolean),
            "int" => return Ok(Self::Int),
            "long" => return Ok(Self::Long),
            "float" => return Ok(Self::Float),
            "double" => return Ok(Self::Double),
            "date" => return Ok(Self::Date),
            "time" => return Ok(Self::Time),
            "timestamp" => return Ok(Self::Timestamp),
            "timestamptz" => return Ok(Self::Timestamptz),
            "timestamp_ns" => return Ok(Self::TimestampNs),
            "timestamptz_ns" => return Ok(Self::TimestamptzNs),
            "unknown" => return Ok(Self::Unknown),
            "string" => return Ok(Self::String),
            "uuid" => return Ok(Self::Uuid),
            "binary" => return Ok(Self::Binary),
            _ => {}
        }

        if let Some(rest) = trimmed.strip_prefix("decimal") {
            let (precision, scale) = parenthesized_pair(rest, "decimal")?;
            let precision = u8::try_from(precision).map_err(|_| {
                parse_error(format_smolstr!(
                    "expected a decimal precision of 1 through 38, got {precision}"
                ))
            })?;
            let scale = i8::try_from(scale).map_err(|_| {
                parse_error(format_smolstr!(
                    "expected a decimal scale that fits 8 bits, got {scale}"
                ))
            })?;
            return Ok(Self::Decimal { precision, scale });
        }

        if let Some(rest) = trimmed.strip_prefix("fixed") {
            let width = parenthesized_one(rest, "fixed")?;
            return Ok(Self::Fixed(width));
        }

        Err(parse_error(format_smolstr!(
            "expected an Iceberg primitive type name, got {trimmed:?}"
        )))
    }
}

impl fmt::Display for PrimitiveType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean => formatter.write_str("boolean"),
            Self::Int => formatter.write_str("int"),
            Self::Long => formatter.write_str("long"),
            Self::Float => formatter.write_str("float"),
            Self::Double => formatter.write_str("double"),
            Self::Decimal { precision, scale } => {
                write!(formatter, "decimal({precision}, {scale})")
            }
            Self::Date => formatter.write_str("date"),
            Self::Time => formatter.write_str("time"),
            Self::Timestamp => formatter.write_str("timestamp"),
            Self::Timestamptz => formatter.write_str("timestamptz"),
            Self::TimestampNs => formatter.write_str("timestamp_ns"),
            Self::TimestamptzNs => formatter.write_str("timestamptz_ns"),
            Self::Unknown => formatter.write_str("unknown"),
            Self::String => formatter.write_str("string"),
            Self::Uuid => formatter.write_str("uuid"),
            Self::Fixed(width) => write!(formatter, "fixed[{width}]"),
            Self::Binary => formatter.write_str("binary"),
        }
    }
}

/// Report a malformed Iceberg type name.
fn parse_error(reason: SmolStr) -> Error {
    Error::Parse {
        target: "iceberg type",
        position: 0,
        reason,
    }
}

/// Read `(a, b)` or `(a,b)` after a type keyword.
fn parenthesized_pair(rest: &str, keyword: &str) -> Result<(i64, i64)> {
    let inner = rest
        .trim()
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| {
            parse_error(format_smolstr!(
                "expected {keyword}(precision, scale), got {keyword}{rest}"
            ))
        })?;
    let (left, right) = inner.split_once(',').ok_or_else(|| {
        parse_error(format_smolstr!(
            "expected {keyword}(precision, scale), got {keyword}{rest}"
        ))
    })?;
    Ok((parse_number(left, keyword)?, parse_number(right, keyword)?))
}

/// Read `[n]` or `(n)` after a type keyword.
fn parenthesized_one(rest: &str, keyword: &str) -> Result<i32> {
    let trimmed = rest.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .or_else(|| {
            trimmed
                .strip_prefix('(')
                .and_then(|value| value.strip_suffix(')'))
        })
        .ok_or_else(|| {
            parse_error(format_smolstr!(
                "expected {keyword}[length], got {keyword}{rest}"
            ))
        })?;
    let value = parse_number(inner, keyword)?;
    i32::try_from(value).map_err(|_| {
        parse_error(format_smolstr!(
            "expected a {keyword} length that fits 32 bits, got {value}"
        ))
    })
}

/// Parse one decimal number from a type parameter.
fn parse_number(value: &str, keyword: &str) -> Result<i64> {
    value.trim().parse::<i64>().map_err(|_| {
        parse_error(format_smolstr!(
            "expected an integer {keyword} parameter, got {:?}",
            value.trim()
        ))
    })
}
