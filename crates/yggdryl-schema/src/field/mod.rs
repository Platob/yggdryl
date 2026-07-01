//! The field layer: the [`Field`] base trait, its category markers
//! ([`PrimitiveField`] / [`LogicalField`] / [`NestedField`]) and the concrete
//! fields. It mirrors the [`dtype`](crate::dtype) layer one-to-one.

mod binary_field;
// `field/field.rs` holds the base `Field` trait, mirroring `dtype/data_type.rs`.
#[allow(clippy::module_inception)]
mod field;
mod logical_field;
mod metadata;
mod nested_field;
mod primitive_field;

pub use binary_field::BinaryField;
pub use field::Field;
pub use logical_field::LogicalField;
pub use metadata::Metadata;
pub use nested_field::NestedField;
pub use primitive_field::PrimitiveField;
