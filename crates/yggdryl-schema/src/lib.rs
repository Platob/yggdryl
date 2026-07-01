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
//!   (with their [`Int8Field`]…[`UInt256Field`] counterparts) are the concrete
//!   primitive types. Their native types are the Rust integers `i8`…`i128` /
//!   `u8`…`u128`, plus the core [`I256`] / [`U256`] for the 256-bit widths.
//! - [`DataTypeId`] is the type discriminant and [`Metadata`] the byte-keyed field
//!   metadata.
//! - Every primitive type and field round-trips through Apache Arrow via the
//!   [`ArrowSchema`] node (`to_arrow_scalar` / `from_arrow_scalar`). The dynamic and
//!   nested layer (`Any`, `Struct`, `AnyType`/`AnyField`, `StructType`/`StructField`)
//!   — and their recursive Arrow round-trip — live in the `yggdryl-scalar` crate,
//!   built on the [`ArrowSchema`] / [`ArrowArray`] nodes and helpers exposed here.
//!
//! New types land one module per concern, and a change to one layer is mirrored in
//! the other.

mod arrow;
mod dtype;
mod field;

pub use arrow::{ArrowArray, ArrowError, ArrowSchema};
pub use dtype::{
    DataType, DataTypeId, Int128Type, Int16Type, Int256Type, Int32Type, Int64Type, Int8Type,
    PrimitiveType, UInt128Type, UInt16Type, UInt256Type, UInt32Type, UInt64Type, UInt8Type,
};
pub use field::{
    Field, Int128Field, Int16Field, Int256Field, Int32Field, Int64Field, Int8Field, Metadata,
    PrimitiveField, UInt128Field, UInt16Field, UInt256Field, UInt32Field, UInt64Field, UInt8Field,
};
// The 256-bit native value types live in the core crate; re-export for convenience.
pub use yggdryl_core::{I256, U256};
