//! The 128-bit fixed-point decimal data type.

use core::fmt;

use arrow_schema::{DataType as ArrowDataType, DECIMAL128_MAX_PRECISION};

use crate::{DataType, DataTypeError, DataTypeId, PrimitiveType};

/// A fixed-point decimal backed by a 128-bit integer, mapping to Arrow
/// `Decimal128(precision, scale)`.
///
/// `precision` (1..=38) is the total number of significant digits and `scale`
/// the number of digits after the decimal point (negative scales shift the
/// point left); the scale's magnitude never exceeds the precision.
///
/// ```
/// use yggdryl_schema::{DataType, Decimal128};
///
/// let decimal = Decimal128::from_parts(38, 10).unwrap();
/// assert_eq!(decimal.to_arrow(), arrow_schema::DataType::Decimal128(38, 10));
/// assert_eq!(Decimal128::from_arrow(&decimal.to_arrow()), Ok(decimal));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawDecimal128")
)]
pub struct Decimal128 {
    precision: u8,
    scale: i8,
}

impl Decimal128 {
    /// Builds the type from a precision and scale, validating both.
    ///
    /// ```
    /// use yggdryl_schema::Decimal128;
    ///
    /// assert!(Decimal128::from_parts(0, 0).is_err()); // expected 1..=38
    /// assert!(Decimal128::from_parts(10, 11).is_err()); // |scale| > precision
    /// ```
    pub fn from_parts(precision: u8, scale: i8) -> Result<Self, DataTypeError> {
        if precision == 0 || precision > DECIMAL128_MAX_PRECISION {
            return Err(DataTypeError::PrecisionOutOfRange {
                precision,
                max: DECIMAL128_MAX_PRECISION,
            });
        }
        if scale.unsigned_abs() > precision {
            return Err(DataTypeError::ScaleOutOfRange { scale, precision });
        }
        Ok(Self { precision, scale })
    }

    /// The total number of significant digits.
    pub fn precision(&self) -> u8 {
        self.precision
    }

    /// The number of digits after the decimal point.
    pub fn scale(&self) -> i8 {
        self.scale
    }

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    pub fn copy(&self, precision: Option<u8>, scale: Option<i8>) -> Result<Self, DataTypeError> {
        Self::from_parts(
            precision.unwrap_or(self.precision),
            scale.unwrap_or(self.scale),
        )
    }

    /// Returns a copy with the precision replaced.
    pub fn with_precision(&self, precision: u8) -> Result<Self, DataTypeError> {
        self.copy(Some(precision), None)
    }

    /// Returns a copy with the scale replaced.
    pub fn with_scale(&self, scale: i8) -> Result<Self, DataTypeError> {
        self.copy(None, Some(scale))
    }
}

impl DataType for Decimal128 {
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
        vec![self.precision, self.scale.cast_unsigned()]
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        match bytes {
            [precision, scale] => Self::from_parts(*precision, scale.cast_signed()),
            _ => Err(DataTypeError::InvalidByteLength {
                expected: 2,
                actual: bytes.len(),
            }),
        }
    }
}

impl PrimitiveType for Decimal128 {
    type Native = i128;
    const BIT_WIDTH: usize = 128;
}

impl fmt::Display for Decimal128 {
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
impl TryFrom<RawDecimal128> for Decimal128 {
    type Error = DataTypeError;

    fn try_from(raw: RawDecimal128) -> Result<Self, Self::Error> {
        Self::from_parts(raw.precision, raw.scale)
    }
}
