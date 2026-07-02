//! The subtrait every fixed-point decimal data type satisfies.

use crate::{DataTypeError, NumericType};

/// A [`NumericType`] whose values are fixed-point decimals: a validated
/// precision (total significant digits, `1..=MAX_PRECISION`) and scale
/// (digits after the decimal point; negative scales shift the point left,
/// and the magnitude never exceeds the precision).
///
/// Implementors supply [`from_parts`](DecimalType::from_parts) and the two
/// accessors; the functional updates come provided.
///
/// ```
/// use yggdryl_schema::{Decimal128Type, DecimalType};
///
/// let money = Decimal128Type::from_parts(38, 2).unwrap();
/// assert_eq!(money.with_scale(4).unwrap().scale(), 4);
/// assert_eq!(Decimal128Type::MAX_PRECISION, 38);
/// ```
pub trait DecimalType: NumericType {
    /// The largest precision the backing width can hold.
    const MAX_PRECISION: u8;

    /// Builds the type from a precision and scale, validating both.
    fn from_parts(precision: u8, scale: i8) -> Result<Self, DataTypeError>;

    /// The total number of significant digits.
    fn precision(&self) -> u8;

    /// The number of digits after the decimal point.
    fn scale(&self) -> i8;

    /// Returns a copy with any of the parts overridden; omitted parts come
    /// from `self`.
    fn copy(&self, precision: Option<u8>, scale: Option<i8>) -> Result<Self, DataTypeError> {
        Self::from_parts(
            precision.unwrap_or_else(|| self.precision()),
            scale.unwrap_or_else(|| self.scale()),
        )
    }

    /// Returns a copy with the precision replaced.
    fn with_precision(&self, precision: u8) -> Result<Self, DataTypeError> {
        self.copy(Some(precision), None)
    }

    /// Returns a copy with the scale replaced.
    fn with_scale(&self, scale: i8) -> Result<Self, DataTypeError> {
        self.copy(None, Some(scale))
    }
}

/// Validates a precision and scale against a width's maximum precision — the
/// one check every decimal implementation shares.
pub(crate) fn validate(precision: u8, scale: i8, max: u8) -> Result<(), DataTypeError> {
    if precision == 0 || precision > max {
        return Err(DataTypeError::PrecisionOutOfRange { precision, max });
    }
    if scale.unsigned_abs() > precision {
        return Err(DataTypeError::ScaleOutOfRange { scale, precision });
    }
    Ok(())
}
