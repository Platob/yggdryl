//! The [`LogicalField`] category marker.

use crate::field::Field;

/// Marks a field whose type is logical, exposing the wrapped inner field through
/// [`inner_field`](LogicalField::inner_field) (mirrors
/// [`LogicalType`](crate::LogicalType) and its `inner_type`).
pub trait LogicalField: Field {
    /// The inner field this logical field wraps.
    fn inner_field(&self) -> &dyn Field;
}
