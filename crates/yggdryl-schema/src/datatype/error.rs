//! The error raised while constructing or converting data types.

use core::fmt;
use std::error::Error;

use arrow_schema::DataType as ArrowDataType;

use crate::TimeUnit;

/// Why a data type could not be constructed or converted.
///
/// ```
/// use yggdryl_schema::Decimal128;
///
/// let error = Decimal128::from_parts(0, 0).unwrap_err();
/// assert_eq!(error.to_string(), "precision 0 out of range, expected 1..=38");
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DataTypeError {
    /// `from_arrow` was handed an Arrow type this yggdryl type does not map to.
    ArrowTypeMismatch {
        /// The rendered name of the expected Arrow type.
        expected: &'static str,
        /// The Arrow type actually received.
        actual: ArrowDataType,
    },
    /// A decimal precision outside the type's supported range.
    PrecisionOutOfRange {
        /// The rejected precision.
        precision: u8,
        /// The largest precision the type supports.
        max: u8,
    },
    /// A decimal scale whose magnitude exceeds the precision.
    ScaleOutOfRange {
        /// The rejected scale.
        scale: i8,
        /// The precision the scale must fit within.
        precision: u8,
    },
    /// A temporal unit the type cannot represent.
    TimeUnitMismatch {
        /// The rendered set of accepted units.
        expected: &'static str,
        /// The unit actually received.
        actual: TimeUnit,
    },
    /// A negative fixed-size width.
    NegativeFixedSize {
        /// The rejected size.
        size: i32,
    },
    /// A byte payload of the wrong length for a fixed-size encoding.
    InvalidByteLength {
        /// The length the encoding requires.
        expected: usize,
        /// The length actually received.
        actual: usize,
    },
    /// A byte payload that failed to decode.
    InvalidBytes {
        /// What failed, and how to fix it.
        message: String,
    },
}

impl fmt::Display for DataTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArrowTypeMismatch { expected, actual } => write!(
                f,
                "expected Arrow type {expected}, got {actual} — \
                 use the yggdryl type that maps to {actual}"
            ),
            Self::PrecisionOutOfRange { precision, max } => {
                write!(f, "precision {precision} out of range, expected 1..={max}")
            }
            Self::ScaleOutOfRange { scale, precision } => write!(
                f,
                "scale {scale} exceeds precision {precision}, \
                 expected a magnitude of at most the precision"
            ),
            Self::TimeUnitMismatch { expected, actual } => {
                write!(f, "unsupported time unit {actual}, expected {expected}")
            }
            Self::NegativeFixedSize { size } => {
                write!(f, "negative size {size}, expected 0 or more")
            }
            Self::InvalidByteLength { expected, actual } => {
                write!(f, "expected {expected} bytes, got {actual}")
            }
            Self::InvalidBytes { message } => f.write_str(message),
        }
    }
}

impl Error for DataTypeError {}
