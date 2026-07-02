//! The 256-bit fixed-point decimal data type.

use core::fmt;

use arrow_buffer::i256;
use arrow_schema::{DataType as ArrowDataType, DECIMAL256_MAX_PRECISION};

use crate::datatype::decimal::decimal_type::validate;
use crate::{DataType, DataTypeError, DataTypeId, DecimalType, PrimitiveType};

/// A fixed-point decimal backed by a 256-bit integer, mapping to Arrow
/// `Decimal256Type(precision, scale)`.
///
/// `precision` (1..=76) is the total number of significant digits and `scale`
/// the number of digits after the decimal point (negative scales shift the
/// point left); the scale's magnitude never exceeds the precision.
///
/// ```
/// use yggdryl_schema::{DataType, Decimal256Type, DecimalType};
///
/// let decimal = Decimal256Type::from_parts(76, 10).unwrap();
/// assert_eq!(decimal.to_arrow(), arrow_schema::DataType::Decimal256(76, 10));
/// assert_eq!(Decimal256Type::from_arrow(&decimal.to_arrow()), Ok(decimal));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawDecimal256")
)]
pub struct Decimal256Type {
    precision: u8,
    scale: i8,
}

impl DecimalType for Decimal256Type {
    const MAX_PRECISION: u8 = DECIMAL256_MAX_PRECISION;

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

impl DataType for Decimal256Type {
    fn type_id(&self) -> DataTypeId {
        DataTypeId::Decimal256
    }

    fn to_arrow(&self) -> ArrowDataType {
        ArrowDataType::Decimal256(self.precision, self.scale)
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Decimal256(precision, scale) => Self::from_parts(*precision, *scale),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "decimal256",
                actual: other.clone(),
            }),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        vec![
            DataTypeId::Decimal256.to_u8(),
            self.precision,
            self.scale.cast_unsigned(),
        ]
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        match DataTypeId::Decimal256.strip_tag(bytes)? {
            [precision, scale] => Self::from_parts(*precision, scale.cast_signed()),
            payload => Err(DataTypeError::InvalidByteLength {
                expected: 2,
                actual: payload.len(),
            }),
        }
    }
}

impl PrimitiveType for Decimal256Type {
    type Native = i256;
    const BIT_WIDTH: usize = 256;
}

impl fmt::Display for Decimal256Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decimal256({}, {})", self.precision, self.scale)
    }
}

/// Mirror of the serialized fields, deserialized first so `try_from`
/// re-validates on the way in.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawDecimal256 {
    precision: u8,
    scale: i8,
}

#[cfg(feature = "serde")]
impl TryFrom<RawDecimal256> for Decimal256Type {
    type Error = DataTypeError;

    fn try_from(raw: RawDecimal256) -> Result<Self, Self::Error> {
        Self::from_parts(raw.precision, raw.scale)
    }
}
