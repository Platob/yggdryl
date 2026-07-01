//! The [`PrimitiveType`] category marker.

use crate::dtype::DataType;

/// Marks a primitive type — a scalar (e.g. an integer), generic over its native
/// value type `T`.
///
/// ```
/// use yggdryl_schema::{Int32Type, PrimitiveType};
///
/// fn takes_primitive<T, D: PrimitiveType<T>>(_d: &D) {}
/// takes_primitive(&Int32Type::new());
/// ```
pub trait PrimitiveType<T>: DataType<T> {}
