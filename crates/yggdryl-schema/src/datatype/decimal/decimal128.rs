//! The 128-bit fixed-point decimal data type.

use core::fmt;

use arrow_schema::{DataType as ArrowDataType, DECIMAL128_MAX_PRECISION};

use crate::datatype::decimal::decimal_type::validate;
use crate::{DataType, DataTypeError, DataTypeId, DecimalType, PrimitiveType};

/// A fixed-point decimal backed by a 128-bit integer, mapping to Arrow
/// `Decimal128Type(precision, scale)`.
///
/// `precision` (1..=38) is the total number of significant digits and `scale`
/// the number of digits after the decimal point (negative scales shift the
/// point left); the scale's magnitude never exceeds the precision.
///
/// ```
/// use yggdryl_schema::{DataType, Decimal128Type, DecimalType};
///
/// let decimal = Decimal128Type::from_parts(38, 10).unwrap();
/// assert_eq!(decimal.to_arrow(), arrow_schema::DataType::Decimal128(38, 10));
/// assert_eq!(Decimal128Type::from_arrow(&decimal.to_arrow()), Ok(decimal));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawDecimal128")
)]
pub struct Decimal128Type {
    precision: u8,
    scale: i8,
}

impl DecimalType for Decimal128Type {
    const MAX_PRECISION: u8 = DECIMAL128_MAX_PRECISION;

    fn from_parts(precision: u8, scale: i8) -> Result<Self, DataTypeError> {
        validate(precision, scale, Self::MAX_PRECISION)?;
        Ok(Self { precision, scale })
    }

    fn precision(&self) -> u8 {
        self.precision
    }

    fn scale(&self) -> i8 {
        self.scale
    }
}

impl DataType for Decimal128Type {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Decimal128
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Decimal128(self.precision, self.scale)
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Decimal128(precision, scale) => Self::from_parts(*precision, *scale),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "decimal128",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        vec![
            DataTypeId::Decimal128.to_u8(),
            self.precision,
            self.scale.cast_unsigned(),
        ]
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        match DataTypeId::Decimal128.strip_tag(bytes)? {
            [precision, scale] => Self::from_parts(*precision, scale.cast_signed()),
            payload => Err(DataTypeError::InvalidByteLength {
                expected: 2,
                actual: payload.len(),
            }),
        }
    }
}

impl PrimitiveType for Decimal128Type {
    type Native = i128;
    const BIT_WIDTH: usize = 128;
}

impl fmt::Display for Decimal128Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decimal128({}, {})", self.precision, self.scale)
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawDecimal128 {
    precision: u8,
    scale: i8,
}

#[cfg(feature = "serde")]
impl TryFrom<RawDecimal128> for Decimal128Type {
    type Error = DataTypeError;

    fn try_from(raw: RawDecimal128) -> Result<Self, Self::Error> {
        Self::from_parts(raw.precision, raw.scale)
    }
}
