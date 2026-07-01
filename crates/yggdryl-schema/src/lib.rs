//! # yggdryl-schema
//!
//! The Arrow-flavoured schema layer for yggdryl. It is built as two mirror-image
//! layers — the data types under [`dtype`](mod@self) and the fields under
//! `field` — that follow the same pattern (see `CLAUDE.md`):
//!
//! - [`DataType`]`<T>` / [`Field`]`<T>` are the base traits, each generic over the
//!   native value type `T` it describes and exposing [`default`](DataType::default)
//!   (the zero of `T`). A `DataType<T>` adds `type_id` / `type_name`; a `Field<T>`
//!   adds `name` / `dtype` / `metadata`.
//! - The [`PrimitiveType`] / [`PrimitiveField`] markers pair up.
//! - The signed [`Int8Type`]…[`Int256Type`] and unsigned [`UInt8Type`]…[`UInt256Type`]
//!   (with their [`Int8Field`]…[`UInt256Field`] counterparts) are the first concrete
//!   types — all primitive. Their native types are the Rust integers `i8`…`i128` /
//!   `u8`…`u128`, plus the core [`I256`] / [`U256`] for the 256-bit widths.
//! - [`StructType`] / [`StructField`] are the recursive composite: a struct holds
//!   heterogeneous child [`AnyField`]s (a dynamic [`AnyType`] each), and its value is
//!   a [`Struct`] — an array of [`Any`]. An Arrow schema is just a `StructField`.
//! - [`DataTypeId`] is the type discriminant and [`Metadata`] the byte-keyed field
//!   metadata.
//!
//! New types land one module per concern, and a change to one layer is mirrored in
//! the other.

mod dtype;
mod field;
mod value;

pub use dtype::{
    AnyType, DataType, DataTypeId, Int128Type, Int16Type, Int256Type, Int32Type, Int64Type,
    Int8Type, PrimitiveType, StructType, UInt128Type, UInt16Type, UInt256Type, UInt32Type,
    UInt64Type, UInt8Type,
};
pub use field::{
    AnyField, Field, Int128Field, Int16Field, Int256Field, Int32Field, Int64Field, Int8Field,
    Metadata, PrimitiveField, StructField, UInt128Field, UInt16Field, UInt256Field, UInt32Field,
    UInt64Field, UInt8Field,
};
pub use value::{Any, Struct};
// The 256-bit native value types live in the core crate; re-export for convenience.
pub use yggdryl_core::{I256, U256};
