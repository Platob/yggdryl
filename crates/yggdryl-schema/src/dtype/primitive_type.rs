//! The [`PrimitiveType`] category marker.

use crate::dtype::DataType;

/// Marks a primitive type — a scalar with no child fields (its
/// [`children_fields`](crate::NestedFields::children_fields) is empty) and no inner
/// type (e.g. an integer type, a boolean).
///
/// ```
/// use yggdryl_schema::{Int32Type, PrimitiveType};
///
/// fn takes_primitive<T: PrimitiveType>(_t: &T) {}
/// takes_primitive(&Int32Type::new());
/// ```
pub trait PrimitiveType: DataType {}
