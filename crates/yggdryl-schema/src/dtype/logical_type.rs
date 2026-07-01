//! The [`LogicalType`] category marker.

use crate::dtype::DataType;

/// Marks a logical type layered over a single physical/storage type, exposed through
/// [`inner_type`](LogicalType::inner_type) (e.g. a dictionary over its value type, a
/// timestamp over its integer storage). Its
/// [`children_fields`](crate::NestedFields::children_fields) come from that inner
/// type.
pub trait LogicalType: DataType {
    /// The physical type this logical type is stored as.
    fn inner_type(&self) -> &dyn DataType;
}
