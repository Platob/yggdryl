//! The [`PrimitiveType`] category marker.

use crate::dtype::DataType;

/// Marks a primitive type — a scalar with no child fields (its
/// [`children_fields`](crate::NestedFields::children_fields) is empty) and no inner
/// type (e.g. [`BinaryType`](crate::BinaryType), a boolean, an integer).
///
/// ```
/// use yggdryl_schema::{BinaryType, PrimitiveType};
///
/// fn takes_primitive<T: PrimitiveType>(_t: &T) {}
/// takes_primitive(&BinaryType::new());
/// ```
pub trait PrimitiveType: DataType {}
