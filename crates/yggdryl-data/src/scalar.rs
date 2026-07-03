//! The typed [`Scalar`] trait: a [`RawScalar`](super::RawScalar) holding a native `T`.

use super::{RawDataType, RawScalar};

/// A [`RawScalar`](super::RawScalar) whose value is the native Rust type `T` — a
/// single, possibly-null value — where `T` may be unsized (e.g. `str`).
///
/// It pins the inherited [`RawScalar::Value`](super::RawScalar::Value) to `T`, so
/// `value` yields `Option<&T>`, and names the concrete data type as the associated
/// [`Type`](Scalar::Type). `Type` is only bound to [`RawDataType`] — deliberately not
/// [`DataType<T>`](super::DataType), whose owned-value byte codec would force `T:
/// Sized` — so a string scalar can expose the borrowed `Option<&str>` while its data
/// type still codecs owned `String`s through [`DataType<String>`](super::DataType).
///
/// ```
/// use yggdryl_data::{Int64, RawScalar, Scalar};
///
/// let answer = Int64::new(42);
/// assert_eq!(answer.value(), Some(&42));
///
/// // `Scalar<i64>` lets generic code accept any int64 scalar.
/// fn take<S: Scalar<i64>>(scalar: &S) -> bool {
///     scalar.is_null()
/// }
/// assert!(!take(&answer));
/// assert!(take(&Int64::null()));
/// ```
pub trait Scalar<T: ?Sized>: RawScalar<Self::Type, Value = T> {
    /// The concrete data type of this scalar.
    type Type: RawDataType;
}
