//! [`PrimitiveScalar`] — the primitive category of [`Scalar`].

use crate::Scalar;

/// A scalar whose data type is a primitive
/// ([`PrimitiveType`](yggdryl_dtype::PrimitiveType)) — the category of the ten native
/// numeric scalars plus `boolean`.
///
/// The scalar-layer parallel of [`yggdryl_dtype::PrimitiveType`]; marker for now, so
/// generic code can bound on "a primitive scalar" independently of its concrete type.
///
/// ```
/// use yggdryl_scalar::{I64Scalar, PrimitiveScalar};
/// fn is_primitive<S: PrimitiveScalar>(_: &S) {}
/// is_primitive(&I64Scalar::new(1));
/// ```
pub trait PrimitiveScalar: Scalar {}
