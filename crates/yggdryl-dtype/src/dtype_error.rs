//! [`DTypeError`] — the failure modes of the dtype layer.

use core::fmt;

/// An error raised while decoding or converting a [`DataType`](crate::DataType).
///
/// Each message names the remedy — the expected width and offending length, or the
/// expected Arrow variant and the one received — so the fix is knowable from the error
/// alone (rule 12). In the bindings it surfaces as a Python `ValueError` / a thrown
/// `Error`.
///
/// ```
/// use yggdryl_dtype::DTypeError;
///
/// let err = DTypeError::InvalidValueLength { ty: "int64", len: 3, width: 8 };
/// assert!(err.to_string().contains("8-byte"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DTypeError {
    /// A primitive data type carries no parameters, but `deserialize_bytes` received a
    /// non-empty payload. Pass an empty byte slice.
    UnexpectedPayload {
        /// The type name, e.g. `"int64"`.
        ty: &'static str,
        /// The offending payload length.
        len: usize,
    },
    /// A value byte-slice whose length is not the type's fixed value width. Pass
    /// exactly `width` bytes.
    InvalidValueLength {
        /// The type name, e.g. `"int64"`.
        ty: &'static str,
        /// The offending byte length.
        len: usize,
        /// The value width the length must equal.
        width: usize,
    },
    /// An Arrow [`DataType`](arrow_schema::DataType) that does not match the concrete
    /// type being built. Pass the expected Arrow variant.
    ArrowTypeMismatch {
        /// The expected type name, e.g. `"int64"`.
        expected: &'static str,
        /// The Arrow type actually received (its `Debug` form).
        got: String,
    },
}

impl fmt::Display for DTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedPayload { ty, len } => write!(
                f,
                "the {ty} data type carries no parameters; expected an empty byte \
                 payload, got {len} bytes"
            ),
            Self::InvalidValueLength { ty, len, width } => write!(
                f,
                "value byte length {len} is not the {width}-byte width of {ty}; pass \
                 exactly {width} bytes"
            ),
            Self::ArrowTypeMismatch { expected, got } => {
                write!(f, "expected the Arrow type for {expected}, got {got}")
            }
        }
    }
}

impl std::error::Error for DTypeError {}
