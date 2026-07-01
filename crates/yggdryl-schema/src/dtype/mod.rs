//! The data-type layer: the [`DataType`] base trait, its category markers
//! ([`PrimitiveType`] / [`LogicalType`] / [`NestedType`]) and the concrete types.

mod data_type;
mod data_type_id;
mod integer_type;
mod logical_type;
mod nested_type;
mod primitive_type;

pub use data_type::DataType;
pub use data_type_id::DataTypeId;
pub use integer_type::{
    Int16Type, Int32Type, Int64Type, Int8Type, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
pub use logical_type::LogicalType;
pub use nested_type::NestedType;
pub use primitive_type::PrimitiveType;
