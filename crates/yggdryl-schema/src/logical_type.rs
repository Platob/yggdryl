//! The [`LogicalType`] category marker.

use crate::data_type::DataType;

/// Marks a logical type layered over a single physical/storage type — it exposes
/// that storage type through [`inner_type`](LogicalType::inner_type) (e.g. a
/// dictionary over its value type, a timestamp over its integer storage).
pub trait LogicalType: DataType {
    /// The physical type this logical type is stored as.
    fn inner_type(&self) -> &dyn DataType;
}
