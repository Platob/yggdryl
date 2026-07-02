//! The [`DataError`] type.

/// An error from a data-model operation, such as decoding a native value from bytes
/// or converting from an Apache Arrow value.
#[derive(Debug)]
#[non_exhaustive]
pub enum DataError {
    /// The bytes handed to a native decoder had the wrong length for the type.
    InvalidByteLength {
        /// The number of bytes the type requires.
        expected: usize,
        /// The number of bytes actually provided.
        got: usize,
    },
    /// The Arrow value handed to `from_arrow` was of a different Arrow type.
    IncompatibleArrowType {
        /// The Arrow type the conversion requires, e.g. `"Int64"`.
        expected: String,
        /// The Arrow type actually provided.
        got: String,
    },
    /// The Arrow array handed to a scalar `from_arrow` did not hold exactly one value.
    InvalidScalarLength {
        /// The number of values the array actually held.
        got: usize,
    },
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::InvalidByteLength { expected, got } => {
                write!(f, "expected {expected} byte(s) but got {got}")
            }
            DataError::IncompatibleArrowType { expected, got } => {
                write!(f, "expected the Arrow type {expected} but got {got}")
            }
            DataError::InvalidScalarLength { got } => {
                write!(
                    f,
                    "a scalar converts from an Arrow array of exactly 1 value but got {got}"
                )
            }
        }
    }
}

impl std::error::Error for DataError {}
