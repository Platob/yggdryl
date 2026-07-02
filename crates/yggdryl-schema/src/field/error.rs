//! The error raised while converting fields.

use core::fmt;
use std::error::Error;

use crate::bytes::BytesError;
use crate::DataTypeError;

/// Why a field could not be converted.
///
/// ```
/// use yggdryl_schema::{Field, FieldError, Int8};
///
/// let arrow = arrow_schema::Field::new("id", arrow_schema::DataType::Utf8, false);
/// assert!(matches!(
///     Field::<Int8>::from_arrow(&arrow),
///     Err(FieldError::DataType(_))
/// ));
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FieldError {
    /// The field's data type failed to construct or convert.
    DataType(DataTypeError),
    /// A byte payload that failed to decode.
    InvalidBytes {
        /// What failed, and how to fix it.
        message: String,
    },
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataType(error) => error.fmt(f),
            Self::InvalidBytes { message } => f.write_str(message),
        }
    }
}

impl Error for FieldError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DataType(error) => Some(error),
            Self::InvalidBytes { .. } => None,
        }
    }
}

impl From<DataTypeError> for FieldError {
    fn from(error: DataTypeError) -> Self {
        Self::DataType(error)
    }
}

impl From<BytesError> for FieldError {
    fn from(error: BytesError) -> Self {
        Self::InvalidBytes {
            message: error.to_string(),
        }
    }
}

// Nested data types embed fields, so a field failure surfaces as a data-type
// failure one level up (e.g. `List::from_bytes` decoding its child).
impl From<FieldError> for DataTypeError {
    fn from(error: FieldError) -> Self {
        match error {
            FieldError::DataType(error) => error,
            FieldError::InvalidBytes { message } => DataTypeError::InvalidBytes { message },
        }
    }
}
