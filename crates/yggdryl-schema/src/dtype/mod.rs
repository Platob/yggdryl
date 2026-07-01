//! The data-type layer: the [`DataType`] base trait, its category markers
//! ([`PrimitiveType`] / [`LogicalType`] / [`NestedType`]) and the concrete types.

mod binary_type;
mod data_type;
mod data_type_id;
mod logical_type;
mod nested_type;
mod primitive_type;

pub use binary_type::BinaryType;
pub use data_type::DataType;
pub use data_type_id::DataTypeId;
pub use logical_type::LogicalType;
pub use nested_type::NestedType;
pub use primitive_type::PrimitiveType;
