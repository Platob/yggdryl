//! [`ScalarError`] — the failure modes of the scalar layer.

use core::fmt;

use yggdryl_dtype::DTypeError;

/// An error raised while decoding a [`Scalar`](crate::Scalar).
///
/// A scalar is always present, so its only decode failure is the value bytes not decoding
/// for the scalar's data type (e.g. a wrong length). The message names the remedy — passed
/// through from the underlying [`DTypeError`] — so the fix is knowable from the error alone
/// (rule 12). In the bindings it surfaces as a Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_scalar::{I64Scalar, ScalarError};
///
/// // Too few bytes for an int64 value is a guided decode error.
/// let err = I64Scalar::deserialize_bytes(&[1, 2, 3]).unwrap_err();
/// assert!(matches!(err, ScalarError::Dtype(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScalarError {
    /// The value bytes did not decode for the scalar's data type. Carries the
    /// [`DTypeError`].
    Dtype(DTypeError),
}

impl fmt::Display for ScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
