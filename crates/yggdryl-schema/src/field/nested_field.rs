//! The [`NestedField`] category marker.

use crate::field::Field;

/// Marks a field whose type is nested — it carries child fields via
/// [`children_fields`](crate::NestedFields::children_fields) (mirrors
/// [`NestedType`](crate::NestedType)).
pub trait NestedField: Field {}
