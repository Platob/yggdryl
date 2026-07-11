//! [`TypedScalar<DT, T>`] — the value-typed extension of [`Scalar`].

use yggdryl_dtype::DataType;

use crate::Scalar;

/// A [`Scalar`] that exposes its typed [`value`](TypedScalar::value) and its concrete data
/// type `DT`.
///
/// A scalar is always present, so `value` returns the value directly (never an `Option`).
/// Carrying the two generic parameters (`DT: DataType` and the native `T`), it is
/// **Rust-only**, like `TypedConverter<S, T>` in the core; the bindings expose the concrete
/// scalars (which fix `DT` and `T`) and the byte-level [`Scalar`] surface.
///
/// ```
/// use yggdryl_dtype::{DataType, I64Type};
/// use yggdryl_scalar::{I64Scalar, TypedScalar};
///
/// let present = I64Scalar::new(7);
/// assert_eq!(present.value(), 7);
/// let dt: I64Type = present.data_type();
/// assert_eq!(dt.name(), "int64");
/// // The default scalar wraps the data type's default value.
/// assert_eq!(I64Scalar::default_scalar().value(), 0);
/// ```
pub trait TypedScalar<DT: DataType, T>: Scalar {
    /// The scalar's value (always present).
    fn value(&self) -> T;

    /// The scalar's concrete data type.
    fn data_type(&self) -> DT;

    /// The default scalar of this type — its data type's
    /// [`default_value`](yggdryl_dtype::TypedDataType::default_value) wrapped as a scalar.
    fn default_scalar() -> Self
    where
        Self: Sized;
}
