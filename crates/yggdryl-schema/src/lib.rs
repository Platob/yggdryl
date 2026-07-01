//! # yggdryl-schema
//!
//! The Arrow-flavoured schema layer for yggdryl. It is built as two mirror-image
//! layers — the data types under [`dtype`](mod@self) and the fields under
//! `field` — that follow the same pattern (see `CLAUDE.md`):
//!
//! - [`DataType`] / [`Field`] are the object-safe base traits. Both are
//!   [`NestedFields`], so the child-field lookups
//!   ([`children_fields`](NestedFields::children_fields) /
//!   [`child_field_at`](NestedFields::child_field_at) /
//!   [`child_field_by`](NestedFields::child_field_by) /
//!   [`child_field`](NestedFields::child_field)) work on both.
//! - Category markers pair up: [`PrimitiveType`] / [`PrimitiveField`],
//!   [`LogicalType`] (`inner_type`) / [`LogicalField`] (`inner_field`), and
//!   [`NestedType`] / [`NestedField`].
//! - The signed [`Int8Type`]…[`Int64Type`] and unsigned [`UInt8Type`]…[`UInt64Type`]
//!   (with their [`Int8Field`]…[`UInt64Field`] counterparts) are the first concrete
//!   types — all primitive.
//! - [`DataTypeId`] is the type discriminant, [`Metadata`] the byte-keyed field
//!   metadata, and [`SchemaError`] the error type.
//!
//! New types land one module per concern, and a change to one layer is mirrored in
//! the other.

mod dtype;
mod error;
mod field;
mod nested_fields;

pub use dtype::{
    DataType, DataTypeId, Int128Type, Int16Type, Int256Type, Int32Type, Int64Type, Int8Type,
    LogicalType, NestedType, PrimitiveType, UInt128Type, UInt16Type, UInt256Type, UInt32Type,
    UInt64Type, UInt8Type,
};
pub use error::SchemaError;
pub use field::{
    Field, Int128Field, Int16Field, Int256Field, Int32Field, Int64Field, Int8Field, LogicalField,
    Metadata, NestedField, PrimitiveField, UInt128Field, UInt16Field, UInt256Field, UInt32Field,
    UInt64Field, UInt8Field,
};
pub use nested_fields::NestedFields;
