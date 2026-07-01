//! The [`PrimitiveField`] category marker.

use crate::field::Field;

/// Marks a field whose type is primitive — generic over its native value type `T`
/// (mirrors [`PrimitiveType`](crate::PrimitiveType)).
///
/// ```
/// use yggdryl_schema::{Int32Field, PrimitiveField};
///
/// fn takes_primitive<T, F: PrimitiveField<T>>(_f: &F) {}
/// takes_primitive(&Int32Field::new("x"));
/// ```
pub trait PrimitiveField<T>: Field<T> {}
