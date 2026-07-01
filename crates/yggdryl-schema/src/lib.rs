//! # yggdryl-schema
//!
//! The Arrow-flavoured schema layer for yggdryl. [`DataType`] is the base trait
//! every data type implements — it knows its [`DataTypeId`] and its
//! [`type_name`](DataType::type_name). Each concrete type also carries a category
//! marker: [`PrimitiveType`], [`LogicalType`] (which exposes an
//! [`inner_type`](LogicalType::inner_type)) or [`NestedType`] (which exposes child
//! [`Field`]s). [`BinaryType`] is the first concrete type. [`Field`] pairs a name
//! with a data type and optional byte-keyed metadata, offering the functional
//! [`copy`](Field::copy) / `with_*` updates.
//!
//! New types land here one module per concern, each re-exported at the crate root,
//! following the rules in `CLAUDE.md`.

mod binary_type;
mod data_type;
mod data_type_id;
mod field;
mod logical_type;
mod nested_type;
mod primitive_type;

pub use binary_type::BinaryType;
pub use data_type::DataType;
pub use data_type_id::DataTypeId;
pub use field::Field;
pub use logical_type::LogicalType;
pub use nested_type::NestedType;
pub use primitive_type::PrimitiveType;
