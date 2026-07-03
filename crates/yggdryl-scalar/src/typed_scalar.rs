//! The typed [`TypedScalar`] trait: a [`Scalar`](super::Scalar) holding a native `T`.

use super::Scalar;
use yggdryl_dtype::DataType;

/// A [`Scalar`](super::Scalar) whose value is the native Rust type `T` — a
/// single, possibly-null value — where `T` may be unsized (e.g. `str`).
///
/// The data type `DT` and value type `T` are explicit generic parameters: `DT` pins
/// the base's associated [`DataType`](super::Scalar::DataType) and `T` pins the inherited
/// [`Scalar::Value`](super::Scalar::Value), so `value` yields `Option<&T>`. `DT` is
/// only bound to [`DataType`](yggdryl_dtype::DataType) — deliberately not
/// [`TypedDataType<T>`](yggdryl_dtype::TypedDataType), whose owned-value byte codec
/// would force `T: Sized` — so a string scalar can expose the borrowed `Option<&str>`
/// while its data type still codecs owned `String`s through
/// [`TypedDataType<String>`](yggdryl_dtype::TypedDataType).
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::Int64Type;
/// use yggdryl_scalar::{Int64Scalar, Scalar, TypedScalar};
///
/// let answer = Int64Scalar::new(42);
/// assert_eq!(answer.value(), Some(&42));
///
/// // `TypedScalar<Int64Type, i64>` lets generic code accept any int64 scalar.
/// fn take<S: TypedScalar<Int64Type, i64>>(scalar: &S) -> bool {
///     scalar.is_null()
/// }
/// assert!(!take(&answer));
/// assert!(take(&Int64Scalar::null()));
/// ```
pub trait TypedScalar<DT: DataType, T: ?Sized>: Scalar<DataType = DT, Value = T> {}
