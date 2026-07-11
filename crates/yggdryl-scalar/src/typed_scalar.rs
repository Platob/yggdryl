//! [`TypedScalar<DT, T>`] — the value-typed extension of [`Scalar`].

use yggdryl_dtype::DataType;

use crate::Scalar;

/// A [`Scalar`] that exposes its typed [`value`](TypedScalar::value) (`Some` when
/// present, `None` when null) and its concrete data type `DT`.
///
/// Carrying the two generic parameters (`DT: DataType` and the native `T`), it is
/// **Rust-only**, like `TypedConverter<S, T>` in the core; the bindings expose the
/// concrete scalars (which fix `DT` and `T`) and the byte-level [`Scalar`] surface.
///
/// ```
/// use yggdryl_dtype::{DataType, I64Type};
/// use yggdryl_scalar::{I64Scalar, TypedScalar};
///
/// let present = I64Scalar::new(7);
/// assert_eq!(present.value(), Some(7));
/// let dt: I64Type = present.data_type();
/// assert_eq!(dt.name(), "int64");
///
/// assert_eq!(I64Scalar::null().value(), None);
/// ```
pub trait TypedScalar<DT: DataType, T>: Scalar {
    /// The scalar's value — `Some` when present, `None` when null.
    fn value(&self) -> Option<T>;

    /// The scalar's concrete data type.
    fn data_type(&self) -> DT;
}
