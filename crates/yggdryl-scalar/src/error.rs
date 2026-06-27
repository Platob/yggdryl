//! The [`ScalarError`] type and the [`ScalarResult`] alias — one error enum for the
//! scalar layer, with actionable messages and `From` conversions from the schema and
//! Arrow layers.

use std::fmt;

use yggdryl_schema::SchemaError;

/// An error building, parsing or converting a [`Scalar`](crate::Scalar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalarError {
    /// A value did not match the [`DataType`](yggdryl_schema::DataType) it was paired
    /// with (e.g. a string value given an `int64` type).
    TypeMismatch {
        /// The expected type's canonical string.
        expected: String,
        /// The found value/type's description.
        found: String,
    },
    /// A schema-layer error surfaced (e.g. converting [`Any`](yggdryl_schema::DataType::Any)
    /// to Arrow). Carries the schema message.
    Schema(String),
    /// An Apache Arrow error from array construction, conversion or IPC.
    Arrow(String),
    /// A canonical string / literal was malformed. The message names what was expected.
    Invalid(String),
    /// The type has no scalar/Arrow backend yet (a view, union or run-end value). The
    /// message names the unsupported case and the way around it.
    Unsupported(String),
}

impl fmt::Display for ScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScalarError::TypeMismatch { expected, found } => {
                write!(
                    f,
                    "scalar type mismatch: expected '{expected}', found '{found}'"
                )
            }
            ScalarError::Schema(msg) => write!(f, "{msg}"),
            ScalarError::Arrow(msg) => write!(f, "arrow error: {msg}"),
            ScalarError::Invalid(msg) => write!(f, "invalid scalar literal: {msg}"),
            ScalarError::Unsupported(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ScalarError {}

impl From<SchemaError> for ScalarError {
    fn from(err: SchemaError) -> ScalarError {
        ScalarError::Schema(err.to_string())
    }
}

impl From<arrow_schema::ArrowError> for ScalarError {
    fn from(err: arrow_schema::ArrowError) -> ScalarError {
        ScalarError::Arrow(err.to_string())
    }
}

/// The result type for scalar operations.
pub type ScalarResult<T> = Result<T, ScalarError>;
