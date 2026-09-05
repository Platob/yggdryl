//! Decimal construction and precision/scale validation.

use smol_str::format_smolstr;

use crate::types::invalid;
use crate::{DataType, DataTypeId, Error, Result};

/// One exact-decimal datatype and its precision and scale.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DecimalType {
    /// Decimal backed by 32 bits.
    Decimal32 { precision: u8, scale: i8 },
    /// Decimal backed by 64 bits.
    Decimal64 { precision: u8, scale: i8 },
    /// Decimal backed by 128 bits.
    Decimal128 { precision: u8, scale: i8 },
    /// Decimal backed by 256 bits.
    Decimal256 { precision: u8, scale: i8 },
}

impl DecimalType {
    /// Return the exact datatype identifier.
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Decimal32 { .. } => DataTypeId::Decimal32,
            Self::Decimal64 { .. } => DataTypeId::Decimal64,
            Self::Decimal128 { .. } => DataTypeId::Decimal128,
            Self::Decimal256 { .. } => DataTypeId::Decimal256,
        }
    }

    /// Validate and convert this family member into the root datatype.
    pub fn into_dtype(self) -> Result<DataType> {
        match self {
            Self::Decimal32 { precision, scale } => DataType::decimal32(precision, scale),
            Self::Decimal64 { precision, scale } => DataType::decimal64(precision, scale),
            Self::Decimal128 { precision, scale } => DataType::decimal128(precision, scale),
            Self::Decimal256 { precision, scale } => DataType::decimal256(precision, scale),
        }
    }
}

impl From<DecimalType> for DataType {
    fn from(value: DecimalType) -> Self {
        match value {
            DecimalType::Decimal32 { precision, scale } => Self::Decimal32 { precision, scale },
            DecimalType::Decimal64 { precision, scale } => Self::Decimal64 { precision, scale },
            DecimalType::Decimal128 { precision, scale } => Self::Decimal128 { precision, scale },
            DecimalType::Decimal256 { precision, scale } => Self::Decimal256 { precision, scale },
        }
    }
}

impl TryFrom<&DataType> for DecimalType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value {
            DataType::Decimal32 { precision, scale } => Ok(Self::Decimal32 {
                precision: *precision,
                scale: *scale,
            }),
            DataType::Decimal64 { precision, scale } => Ok(Self::Decimal64 {
                precision: *precision,
                scale: *scale,
            }),
            DataType::Decimal128 { precision, scale } => Ok(Self::Decimal128 {
                precision: *precision,
                scale: *scale,
            }),
            DataType::Decimal256 { precision, scale } => Ok(Self::Decimal256 {
                precision: *precision,
                scale: *scale,
            }),
            other => Err(Error::InvalidDataType {
                kind: "decimal",
                reason: format_smolstr!("expected a decimal datatype, got {other}"),
            }),
        }
    }
}

impl DataType {
    /// Creates a Decimal32.
    pub fn decimal32(precision: u8, scale: i8) -> Result<Self> {
        validate_decimal("Decimal32", precision, scale, 9)?;
        Ok(Self::Decimal32 { precision, scale })
    }

    /// Creates a Decimal64.
    pub fn decimal64(precision: u8, scale: i8) -> Result<Self> {
        validate_decimal("Decimal64", precision, scale, 18)?;
        Ok(Self::Decimal64 { precision, scale })
    }

    /// Creates the most compact Arrow-compatible decimal for the requested
    /// precision and scale.
    ///
    /// Precisions through 9, 18, 38, and 76 use Decimal32, Decimal64,
    /// Decimal128, and Decimal256 respectively. Validation is delegated to the
    /// selected explicit constructor, including the requirement that a
    /// positive scale cannot exceed the precision.
    pub fn decimal(precision: u8, scale: i8) -> Result<Self> {
        match precision {
            0..=9 => Self::decimal32(precision, scale),
            10..=18 => Self::decimal64(precision, scale),
            19..=38 => Self::decimal128(precision, scale),
            _ => Self::decimal256(precision, scale),
        }
    }

    /// Creates a Decimal128.
    pub fn decimal128(precision: u8, scale: i8) -> Result<Self> {
        validate_decimal("Decimal128", precision, scale, 38)?;
        Ok(Self::Decimal128 { precision, scale })
    }

    /// Creates a Decimal256.
    pub fn decimal256(precision: u8, scale: i8) -> Result<Self> {
        validate_decimal("Decimal256", precision, scale, 76)?;
        Ok(Self::Decimal256 { precision, scale })
    }
}

pub(crate) fn validate_decimal(
    kind: &'static str,
    precision: u8,
    scale: i8,
    maximum: u8,
) -> Result<()> {
    if precision == 0 || precision > maximum {
        return Err(invalid(
            kind,
            format_smolstr!("precision must be between 1 and {maximum}: {precision}"),
        ));
    }
    if scale > 0 && scale.unsigned_abs() > precision {
        return Err(invalid(
            kind,
            format_smolstr!("positive scale cannot exceed precision: {scale} > {precision}"),
        ));
    }
    Ok(())
}
