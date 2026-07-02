//! The [`Int64Scalar`]: a single, possibly-null [`Int64`](super::Int64) value.

use super::{Int64, RawScalar, Scalar};

/// A single [`Int64`] value, or null — a native `Option<i64>`.
///
/// The first concrete scalar: it implements the raw [`RawScalar<Int64>`] surface and
/// the typed [`Scalar<i64>`], so `value` yields `Option<&i64>`.
///
/// ```
/// use yggdryl_data::{Int64Scalar, RawDataType, RawScalar};
///
/// let answer = Int64Scalar::new(42);
/// assert!(!answer.is_null());
/// assert_eq!(answer.value(), Some(&42));
/// assert_eq!(answer.data_type().name(), "int64");
///
/// let missing = Int64Scalar::null();
/// assert!(missing.is_null());
/// assert_eq!(missing.value(), None);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Int64Scalar {
    data_type: Int64,
    value: Option<i64>,
}

impl Int64Scalar {
    /// A scalar holding `value`.
    pub fn new(value: i64) -> Self {
        Self {
            data_type: Int64,
            value: Some(value),
        }
    }

    /// A null scalar.
    pub fn null() -> Self {
        Self {
            data_type: Int64,
            value: None,
        }
    }
}

impl RawScalar<Int64> for Int64Scalar {
    type Value = i64;

    fn data_type(&self) -> &Int64 {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }

    fn value(&self) -> Option<&i64> {
        self.value.as_ref()
    }
}

impl Scalar<i64> for Int64Scalar {
    type Type = Int64;
}
