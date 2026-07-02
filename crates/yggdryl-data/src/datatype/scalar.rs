//! The typed [`Scalar`] trait: a [`RawScalar`](super::RawScalar) holding a native `T`.

use super::{DataType, RawScalar};

/// A [`RawScalar`](super::RawScalar) whose value is the native Rust type `T` — a
/// single, possibly-null value of a typed [`DataType<T>`].
///
/// It pins the inherited [`RawScalar::Value`](super::RawScalar::Value) to `T` and
/// names the concrete data type as the associated [`Type`](Scalar::Type), so
/// `value` (inherited from [`RawScalar`](super::RawScalar)) yields `Option<&T>`.
///
/// ```
/// use yggdryl_data::{Int64Scalar, RawScalar, Scalar};
///
/// let answer = Int64Scalar::new(42);
/// assert_eq!(answer.value(), Some(&42));
///
/// // `Scalar<i64>` lets generic code accept any int64 scalar.
/// fn take<S: Scalar<i64>>(scalar: &S) -> bool {
///     scalar.is_null()
/// }
/// assert!(!take(&answer));
/// assert!(take(&Int64Scalar::null()));
/// ```
pub trait Scalar<T>: RawScalar<Self::Type, Value = T> {
    /// The concrete data type of this scalar.
    type Type: DataType<T>;
}
