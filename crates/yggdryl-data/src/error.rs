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
        /// The Arrow type the conversion requires, e.g. `"Int64Type"`.
        expected: String,
        /// The Arrow type actually provided.
        got: String,
    },
    /// The Arrow array handed to a scalar `from_arrow` did not hold exactly one value.
    InvalidScalarLength {
        /// The number of values the array actually held.
        got: usize,
    },
    /// The element type has no fixed byte width, so a byte-encoded sequence cannot
    /// be split into elements.
    IndeterminateElementWidth {
        /// The name of the element data type without a fixed width.
        data_type: String,
    },
    /// The per-element null buffer handed to an array constructor did not match the
    /// element buffer's length.
    MismatchedNullBufferLength {
        /// The element buffer's length the null buffer must match.
        expected: usize,
        /// The number of flags the null buffer actually held.
        got: usize,
    },
    /// An `as_*` accessor was called on a null scalar, which holds no value.
    NullValue,
    /// The scalar's value does not convert *exactly* to the `as_*` target type —
    /// a narrowing or sign change out of range, a float that would round, or
    /// bytes that are not valid UTF-8.
    InexactConversion {
        /// The offending value (or a short description of it).
        value: String,
        /// The requested target type, e.g. `"i8"`.
        target: &'static str,
    },
    /// The scalar's data type has no conversion to the `as_*` target type at all
    /// (e.g. an integer read as `str`).
    UnsupportedConversion {
        /// The scalar's data type name, e.g. `"int64"`.
        data_type: String,
        /// The requested target type, e.g. `"str"`.
        target: &'static str,
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
            DataError::IndeterminateElementWidth { data_type } => {
                write!(
                    f,
                    "the element type {data_type} has no fixed byte width; decode from Arrow \
                     instead of bytes"
                )
            }
            DataError::MismatchedNullBufferLength { expected, got } => {
                write!(
                    f,
                    "expected a null buffer of length {expected} but got {got}; pass one \
                     validity flag per element"
                )
            }
            DataError::NullValue => {
                write!(
                    f,
                    "the scalar is null and holds no value; check is_null() first"
                )
            }
            DataError::InexactConversion { value, target } => {
                write!(
                    f,
                    "{value} is not exactly representable as {target}; read a target that \
                     holds the value"
                )
            }
            DataError::UnsupportedConversion { data_type, target } => {
                write!(f, "{data_type} scalars have no {target} conversion")
            }
        }
    }
}

impl std::error::Error for DataError {}
