//! [`TypedPrimitiveScalar<DT, T>`] — the typed primitive category.

use yggdryl_dtype::DataType;

use crate::{PrimitiveScalar, TypedScalar};

/// A [`PrimitiveScalar`] that also exposes its typed value
/// ([`TypedScalar<DT, T>`](crate::TypedScalar)).
///
/// Blanket-implemented, so every concrete primitive scalar is automatically a
/// `TypedPrimitiveScalar` for its `(DT, T)` — mirroring
/// [`TypedPrimitiveType`](yggdryl_dtype::TypedPrimitiveType). Generic, so Rust-only.
///
/// ```
/// use yggdryl_dtype::I64Type;
/// use yggdryl_scalar::{I64Scalar, TypedPrimitiveScalar};
/// fn _accepts<DT: yggdryl_dtype::DataType, T, S: TypedPrimitiveScalar<DT, T>>(_: &S) {}
/// _accepts::<I64Type, i64, _>(&I64Scalar::new(1));
/// ```
pub trait TypedPrimitiveScalar<DT: DataType, T>: PrimitiveScalar + TypedScalar<DT, T> {}

impl<DT: DataType, T, S> TypedPrimitiveScalar<DT, T> for S where
    S: PrimitiveScalar + TypedScalar<DT, T>
{
}
