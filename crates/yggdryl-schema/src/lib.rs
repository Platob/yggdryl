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
//! - [`BinaryType`] / [`BinaryField`] are the first concrete pair (both primitive).
//! - [`DataTypeId`] is the type discriminant, [`Metadata`] the byte-keyed field
//!   metadata, and [`SchemaError`] the error type.
//!
//! New types land one module per concern, and a change to one layer is mirrored in
//! the other.

mod dtype;
mod error;
mod field;
mod nested_fields;

pub use dtype::{BinaryType, DataType, DataTypeId, LogicalType, NestedType, PrimitiveType};
pub use error::SchemaError;
pub use field::{BinaryField, Field, LogicalField, Metadata, NestedField, PrimitiveField};
pub use nested_fields::NestedFields;
