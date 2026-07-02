//! The error raised while constructing or converting arrays.

use core::fmt;
use std::error::Error;

use yggdryl_scalar::ScalarError;
use yggdryl_schema::DataTypeError;

/// Why an array could not be constructed or converted.
///
/// ```
/// use yggdryl_array::{ArrayError, PrimitiveArray};
/// use yggdryl_schema::Int32;
///
/// let column = PrimitiveArray::from_native(Int32, vec![1, 2, 3]);
/// assert_eq!(
///     column.slice(2, 2).unwrap_err(),
///     ArrayError::SliceOutOfBounds { offset: 2, length: 2, len: 3 }
/// );
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ArrayError {
    /// A validity bitmap whose length differs from the values'.
    LengthMismatch {
        /// The number of values.
        values: usize,
        /// The validity bitmap's length.
        validity: usize,
    },
    /// A slice reaching past the end of the array.
    SliceOutOfBounds {
        /// The requested start.
        offset: usize,
        /// The requested length.
        length: usize,
        /// The array's length.
        len: usize,
    },
    /// A values payload of the wrong byte length for the array's layout.
    InvalidByteLength {
        /// The length the layout requires.
        expected: usize,
        /// The length actually received.
        actual: usize,
    },
    /// The array's data type failed to construct or convert.
    DataType(DataTypeError),
    /// A scalar extracted or decoded from the array failed to construct.
    Scalar(ScalarError),
    /// A byte payload that failed to decode.
    InvalidBytes {
        /// What failed, and how to fix it.
        message: String,
    },
}

impl fmt::Display for ArrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthMismatch { values, validity } => write!(
                f,
                "validity bitmap covers {validity} slots but there are {values} values — \
                 build them with the same length"
            ),
            Self::SliceOutOfBounds {
                offset,
                length,
                len,
            } => write!(
                f,
                "slice of {length} at offset {offset} reaches past the array's {len} elements"
            ),
            Self::InvalidByteLength { expected, actual } => {
                write!(f, "expected {expected} value bytes, got {actual}")
            }
            Self::DataType(error) => error.fmt(f),
            Self::Scalar(error) => error.fmt(f),
            Self::InvalidBytes { message } => f.write_str(message),
        }
    }
}

impl Error for ArrayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DataType(error) => Some(error),
            Self::Scalar(error) => Some(error),
            _ => None,
        }
    }
}

impl From<DataTypeError> for ArrayError {
    fn from(error: DataTypeError) -> Self {
        Self::DataType(error)
    }
}

impl From<ScalarError> for ArrayError {
    fn from(error: ScalarError) -> Self {
        Self::Scalar(error)
    }
}
