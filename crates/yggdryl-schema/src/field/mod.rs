//! The field layer: the [`Field`] base trait, its category markers
//! ([`PrimitiveField`] / [`LogicalField`] / [`NestedField`]) and the concrete
//! fields. It mirrors the [`dtype`](crate::dtype) layer one-to-one.

// `field/field.rs` holds the base `Field` trait, mirroring `dtype/data_type.rs`.
#[allow(clippy::module_inception)]
mod field;
mod integer_field;
mod logical_field;
mod metadata;
mod nested_field;
mod primitive_field;

pub use field::Field;
pub use integer_field::{
    Int128Field, Int16Field, Int256Field, Int32Field, Int64Field, Int8Field, UInt128Field,
    UInt16Field, UInt256Field, UInt32Field, UInt64Field, UInt8Field,
};
pub use logical_field::LogicalField;
pub use metadata::Metadata;
pub use nested_field::NestedField;
pub use primitive_field::PrimitiveField;
