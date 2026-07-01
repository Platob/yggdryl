//! The [`NestedType`] category marker.

use crate::data_type::DataType;
use crate::field::Field;

/// Marks a type composed of child [`Field`]s (a list, struct, map or union). An
/// implementor supplies [`children_fields`](NestedType::children_fields); lookup by
/// index ([`child_field_at`](NestedType::child_field_at)) and by name
/// ([`child_field_by`](NestedType::child_field_by)) default to scanning it.
pub trait NestedType: DataType {
    /// The child fields, in order.
    fn children_fields(&self) -> &[Field];

    /// The child field at `index`, if any.
    fn child_field_at(&self, index: usize) -> Option<&Field> {
        self.children_fields().get(index)
    }

    /// The first child field named `name`, if any.
    fn child_field_by(&self, name: &str) -> Option<&Field> {
        self.children_fields().iter().find(|f| f.name() == name)
    }
}
