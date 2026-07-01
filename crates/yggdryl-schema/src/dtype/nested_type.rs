//! The [`NestedType`] category marker.

use crate::dtype::DataType;

/// Marks a type composed of child fields (a list, struct, map or union). A nested
/// type overrides [`children_fields`](crate::NestedFields::children_fields) to return
/// them; the shared lookups ([`child_field_at`](crate::NestedFields::child_field_at)
/// / [`child_field_by`](crate::NestedFields::child_field_by) /
/// [`child_field`](crate::NestedFields::child_field)) then work over it.
pub trait NestedType: DataType {}
