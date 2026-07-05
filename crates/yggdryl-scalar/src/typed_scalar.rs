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
/// It also names the concrete Apache Arrow types this scalar produces: `ArrowScalar`
/// is the array type [`to_arrow_scalar`](super::Scalar::to_arrow_scalar) *wraps* in an
/// [`arrow_array::Scalar`] (the one-element scalar form — `Int64Array` for an `int64`
/// scalar, `ListArray` for a serie), and `ArrowArray` the array type of
/// [`to_arrow_array`](super::Scalar::to_arrow_array) (the array form). `ArrowArray`
/// **defaults to `ArrowScalar`**, since for a plain scalar the array form *is* the
/// one-element scalar array; a serie overrides it to its element array type (so
/// `Int64Serie` is a `TypedScalar<TypedSerieType<Int64Type>, [i64], ListArray, Int64Array>`).
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::Int64Type;
/// use yggdryl_scalar::arrow_array::Int64Array;
/// use yggdryl_scalar::{Int64Scalar, Scalar, TypedScalar};
///
/// let answer = Int64Scalar::new(42);
/// assert_eq!(answer.value(), Some(&42));
///
/// // `TypedScalar<Int64Type, i64, Int64Array>` lets generic code accept any int64
/// // scalar and name its concrete Arrow array type (ArrowArray defaults to it).
/// fn take<S: TypedScalar<Int64Type, i64, Int64Array>>(scalar: &S) -> bool {
///     scalar.is_null()
/// }
/// assert!(!take(&answer));
/// assert!(take(&Int64Scalar::null()));
/// ```
pub trait TypedScalar<
    DT: DataType,
    T: ?Sized,
    ArrowScalar: arrow_array::Array,
    ArrowArray: arrow_array::Array = ArrowScalar,
>: Scalar<DataType = DT, Value = T>
{
}
