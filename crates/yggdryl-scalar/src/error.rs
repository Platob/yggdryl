//! The error raised while constructing or converting scalars.

use core::fmt;
use std::error::Error;

use yggdryl_schema::DataTypeError;

/// Why a scalar could not be constructed or converted.
///
/// ```
/// use arrow_buffer::Buffer;
/// use yggdryl_scalar::{Scalar, ScalarError};
/// use yggdryl_schema::Int32;
///
/// let short = Scalar::from_parts(Int32, Some(Buffer::from(vec![0u8; 3])));
/// assert_eq!(
///     short.unwrap_err(),
///     ScalarError::InvalidByteLength { expected: 4, actual: 3 }
/// );
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ScalarError {
    /// A value buffer of the wrong length for the type's layout.
    InvalidByteLength {
        /// The length the layout requires.
        expected: usize,
        /// The length actually received.
        actual: usize,
    },
    /// A value buffer whose start is not aligned for the native type.
    MisalignedBuffer {
        /// The alignment the native type requires.
        align: usize,
    },
    /// A boolean byte that is neither 0 nor 1.
    InvalidBoolean {
        /// The rejected byte.
        value: u8,
    },
    /// A string payload that is not valid UTF-8.
    InvalidUtf8,
    /// The scalar's data type failed to construct or convert.
    DataType(DataTypeError),
    /// A byte payload that failed to decode.
    InvalidBytes {
        /// What failed, and how to fix it.
        message: String,
    },
}

impl fmt::Display for ScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidByteLength { expected, actual } => {
                write!(f, "expected {expected} value bytes, got {actual}")
            }
            Self::MisalignedBuffer { align } => write!(
                f,
                "value buffer is not aligned to {align} bytes — rebuild it with from_native \
                 or copy it into a fresh buffer"
            ),
            Self::InvalidBoolean { value } => {
                write!(f, "invalid boolean byte {value}, expected 0 or 1")
            }
            Self::InvalidUtf8 => f.write_str("string payload is not valid UTF-8"),
            Self::DataType(error) => error.fmt(f),
            Self::InvalidBytes { message } => f.write_str(message),
        }
    }
}

impl Error for ScalarError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DataType(error) => Some(error),
            _ => None,
        }
    }
}

impl From<DataTypeError> for ScalarError {
    fn from(error: DataTypeError) -> Self {
        Self::DataType(error)
    }
}
