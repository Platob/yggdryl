//! The [`PrimitiveScalar`] category marker.

use crate::Scalar;

/// Marks a primitive scalar — a single scalar value (e.g. an integer), generic over
/// its native value type `T`. The value-layer mirror of
/// [`PrimitiveField`](yggdryl_schema::PrimitiveField).
///
/// ```
/// use yggdryl_scalar::{Int32, PrimitiveScalar};
///
/// fn takes_primitive<T, S: PrimitiveScalar<T>>(_s: &S) {}
/// takes_primitive(&Int32::from(1));
/// ```
pub trait PrimitiveScalar<T>: Scalar<T> {}
