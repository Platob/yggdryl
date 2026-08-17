//! Decimal construction and precision/scale validation.

use smol_str::format_smolstr;

use crate::Result;

use super::DataType;
use super::scalar::invalid;

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

    /// Creates the most compact Arrow-compatible wide decimal for the
    /// requested precision and scale.
    ///
    /// Precisions through 38 use [`Self::Decimal128`]; precisions from 39
    /// through 76 use [`Self::Decimal256`]. Validation is delegated to
    /// [`Self::decimal128`] or [`Self::decimal256`], including the requirement
    /// that a positive scale cannot exceed the precision.
    pub fn decimal(precision: u8, scale: i8) -> Result<Self> {
        if precision <= 38 {
            Self::decimal128(precision, scale)
        } else {
            Self::decimal256(precision, scale)
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

pub(super) fn validate_decimal(
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
