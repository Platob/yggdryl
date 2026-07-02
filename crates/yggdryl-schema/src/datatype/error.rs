//! The error raised while constructing or converting data types.

use core::fmt;
use std::error::Error;

use arrow_schema::DataType as ArrowDataType;

use crate::TimeUnitId;

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
        actual: TimeUnitId,
    },
    /// A negative fixed-size width.
    NegativeFixedSize {
        /// The rejected size.
        size: i32,
    },
    /// A map entries field that is not a two-field struct with a
    /// non-nullable key.
    InvalidMapEntries {
        /// What failed, and how to fix it.
        message: String,
    },
    /// An integer that is not an assigned data type identifier.
    UnknownTypeId {
        /// The rejected value.
        id: u8,
        /// The largest identifier currently assigned.
        max: u8,
    },
    /// An integer that is not an assigned time unit identifier.
    UnknownTimeUnitId {
        /// The rejected value.
        id: u8,
        /// The largest identifier currently assigned.
        max: u8,
    },
    /// A `ygg.*` metadata key no type understands.
    UnknownMetadata {
        /// The rejected key.
        key: String,
    },
    /// A `ygg.*` metadata value the key does not accept.
    InvalidMetadata {
        /// The key whose value was rejected.
        key: &'static str,
        /// The rejected value.
        value: String,
    },
    /// A `ygg.*` metadata key an anchored type needs but did not receive.
    MissingMetadata {
        /// The missing key.
        key: &'static str,
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
            Self::InvalidMapEntries { message } => f.write_str(message),
            Self::UnknownTypeId { id, max } => {
                write!(f, "unknown data type id {id}, expected 0..={max}")
            }
            Self::UnknownTimeUnitId { id, max } => {
                write!(f, "unknown time unit id {id}, expected 0..={max}")
            }
            Self::UnknownMetadata { key } => {
                write!(
                    f,
                    "unknown metadata key {key} — the ygg.* prefix is reserved"
                )
            }
            Self::InvalidMetadata { key, value } => {
                write!(f, "invalid {key} metadata value \"{value}\"")
            }
            Self::MissingMetadata { key } => write!(
                f,
                "missing {key} metadata — convert through a field, which carries the \
                 ygg.* metadata restoring this type"
            ),
            Self::InvalidByteLength { expected, actual } => {
                write!(f, "expected {expected} bytes, got {actual}")
            }
            Self::InvalidBytes { message } => f.write_str(message),
        }
    }
}

impl Error for DataTypeError {}

impl From<crate::bytes::BytesError> for DataTypeError {
    fn from(error: crate::bytes::BytesError) -> Self {
        Self::InvalidBytes {
            message: error.to_string(),
        }
    }
}
