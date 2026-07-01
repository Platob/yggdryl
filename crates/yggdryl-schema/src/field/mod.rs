//! The field layer: the [`Field`] base trait, its [`PrimitiveField`] category marker
//! and the concrete primitive fields. It mirrors the [`dtype`](crate::dtype) layer
//! one-to-one; the dynamic / nested fields live in the `yggdryl-scalar` crate.

// `field/field.rs` holds the base `Field` trait, mirroring `dtype/data_type.rs`.
#[allow(clippy::module_inception)]
mod field;
mod integer_field;
mod metadata;
mod primitive_field;

pub use field::Field;
pub use integer_field::{
    Int128Field, Int16Field, Int256Field, Int32Field, Int64Field, Int8Field, UInt128Field,
    UInt16Field, UInt256Field, UInt32Field, UInt64Field, UInt8Field,
};
pub use metadata::Metadata;
pub use primitive_field::PrimitiveField;
