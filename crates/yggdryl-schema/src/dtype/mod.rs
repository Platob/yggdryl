//! The data-type layer: the [`DataType`] base trait, its [`PrimitiveType`] category
//! marker and the concrete types.

mod any_type;
mod data_type;
mod data_type_id;
mod integer_type;
mod primitive_type;
mod struct_type;

pub use any_type::AnyType;
pub use data_type::DataType;
pub use data_type_id::DataTypeId;
pub use integer_type::{
    Int128Type, Int16Type, Int256Type, Int32Type, Int64Type, Int8Type, UInt128Type, UInt16Type,
    UInt256Type, UInt32Type, UInt64Type, UInt8Type,
};
pub use primitive_type::PrimitiveType;
pub use struct_type::StructType;
