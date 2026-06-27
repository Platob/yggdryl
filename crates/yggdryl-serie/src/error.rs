//! The [`SerieError`] type and the [`SerieResult`] alias used across the crate.

use std::fmt;

use yggdryl_schema::SchemaError;

/// A `Result` whose error is a [`SerieError`].
pub type SerieResult<T> = Result<T, SerieError>;

/// Error returned when a [`Serie`](crate::Serie) cannot be built, converted or
/// addressed. Messages are actionable — they name the mismatch or the missing
/// support, never just that it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerieError {
    /// A schema-layer error surfaced while building or converting a column.
    Schema(SchemaError),
    /// An Apache Arrow error (array build, slice or cast).
    Arrow(String),
    /// The column's [`Field`](yggdryl_schema::Field) type does not match the backing
    /// Arrow array.
    TypeMismatch {
        /// The Arrow type the field maps to.
        expected: String,
        /// The Arrow type the array actually is.
        found: String,
    },
    /// An index was past the end of the column.
    OutOfBounds {
        /// The requested index.
        index: usize,
        /// The column length.
        len: usize,
    },
    /// The operation has no equivalent for this type yet; the message names what to do
    /// instead.
    Unsupported(String),
}

impl fmt::Display for SerieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SerieError::Schema(err) => write!(f, "{err}"),
            SerieError::Arrow(msg) => write!(f, "arrow error: {msg}"),
            SerieError::TypeMismatch { expected, found } => write!(
                f,
                "field type maps to '{expected}' but the array is '{found}'"
            ),
            SerieError::OutOfBounds { index, len } => {
                write!(
                    f,
                    "index {index} is out of bounds for a serie of length {len}"
                )
            }
            SerieError::Unsupported(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for SerieError {}

impl From<SchemaError> for SerieError {
    fn from(err: SchemaError) -> SerieError {
        SerieError::Schema(err)
    }
}

impl From<arrow_schema::ArrowError> for SerieError {
    fn from(err: arrow_schema::ArrowError) -> SerieError {
        SerieError::Arrow(err.to_string())
    }
}
