//! [`ScalarError`] — the failure modes of the scalar layer.

use core::fmt;

use yggdryl_dtype::DTypeError;

/// An error raised while decoding a [`Scalar`](crate::Scalar).
///
/// Each message names the remedy — the missing null flag, the unexpected flag byte, the
/// stray value bytes on a null, or the underlying value-decode error — so the fix is
/// knowable from the error alone (rule 12). In the bindings it surfaces as a Python
/// `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_scalar::ScalarError;
///
/// assert!(ScalarError::InvalidNullFlag { flag: 2 }.to_string().contains("expected 0"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScalarError {
    /// A serialised scalar with no bytes at all — it needs at least its 1-byte null
    /// flag. Pass a payload produced by `serialize_bytes`.
    EmptyPayload,
    /// A null flag that is neither `0` (null) nor `1` (present). Pass `0` or `1`.
    InvalidNullFlag {
        /// The offending flag byte.
        flag: u8,
    },
    /// A null scalar (flag `0`) followed by stray value bytes. A null carries no value.
    NullWithValue {
        /// The number of unexpected trailing bytes.
        len: usize,
    },
    /// The value bytes did not decode for the scalar's data type. Carries the
    /// [`DTypeError`].
    Dtype(DTypeError),
}

impl fmt::Display for ScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPayload => write!(
                f,
                "a serialised scalar needs at least its 1-byte null flag; got an empty \
                 payload"
            ),
            Self::InvalidNullFlag { flag } => write!(
                f,
                "invalid scalar null flag {flag}; expected 0 (null) or 1 (present)"
            ),
            Self::NullWithValue { len } => write!(
                f,
                "a null scalar carries no value, but {len} trailing value byte(s) \
                 followed the null flag"
            ),
            Self::Dtype(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ScalarError {}

impl From<DTypeError> for ScalarError {
    fn from(error: DTypeError) -> Self {
        Self::Dtype(error)
    }
}
