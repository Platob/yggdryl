//! # yggdryl-scalar
//!
//! The value + dynamic-schema layer for yggdryl. It owns the dynamic value
//! ([`AnyValue`] / [`StructValue`]), the dynamic and nested schema
//! ([`AnyType`] / [`AnyField`], [`StructType`] / [`StructField`]) that the
//! `yggdryl-schema` crate's typed primitives compose into, and the **scalar** — a
//! single value paired with the [`Field`](yggdryl_schema::Field) that describes it.
//! The scalar types wear simple names since they live under the `scalar` namespace:
//!
//! - [`AnyValue`] is the dynamic value (primitives + the recursive [`StructValue`]),
//!   with atomic accessors (`as_i32`, …). An Arrow schema is a [`StructField`], so the
//!   dynamic layer carries the full recursive Arrow round-trip (`to_arrow` /
//!   `from_arrow`), built on the schema crate's `ArrowSchema` node.
//! - [`Scalar`]`<T>` is the base trait: every scalar exposes its
//!   [`value`](Scalar::value) and [`field`](Scalar::field), and — through the field —
//!   its [`name`](Scalar::name), [`dtype`](Scalar::dtype) and
//!   [`metadata`](Scalar::metadata). [`PrimitiveScalar`] pairs with
//!   [`PrimitiveField`](yggdryl_schema::PrimitiveField).
//! - The signed [`Int8`]…[`Int256`] and unsigned [`UInt8`]…[`UInt256`] are the concrete
//!   primitive scalars, [`Any`] the dynamic scalar, and [`Struct`] the nested scalar
//!   built from a **collection** of child scalars — the way scalars compose into rows.
//!
//! ```
//! use yggdryl_scalar::{Any, Int32, Scalar, Struct};
//!
//! // Build primitive scalars from native values, compose them into a row.
//! let row = Struct::new(
//!     "row",
//!     vec![Any::from(1i32), Int32::from(2).with_name("n".into()).into()],
//! );
//! assert_eq!(row.len(), 2);
//! ```

mod any_field;
mod any_scalar;
mod any_type;
mod any_value;
mod arrow;
mod integer_scalar;
mod primitive_scalar;
mod scalar;
mod struct_field;
mod struct_scalar;
mod struct_type;
mod struct_value;

pub use any_field::AnyField;
pub use any_scalar::Any;
pub use any_type::AnyType;
pub use any_value::AnyValue;
pub use arrow::ArrowSchemaExt;
pub use integer_scalar::{
    Int128, Int16, Int256, Int32, Int64, Int8, UInt128, UInt16, UInt256, UInt32, UInt64, UInt8,
};
pub use primitive_scalar::PrimitiveScalar;
pub use scalar::Scalar;
pub use struct_field::StructField;
pub use struct_scalar::Struct;
pub use struct_type::StructType;
pub use struct_value::StructValue;
// Re-export the schema vocabulary the dynamic/scalar layer builds on, so this crate is
// a one-stop for the full value + schema surface.
pub use yggdryl_schema::{
    ArrowArray, ArrowError, ArrowSchema, DataType, DataTypeId, Field, Metadata, PrimitiveField,
    PrimitiveType,
};
// The 256-bit native value types live in the core crate; re-export for convenience.
pub use yggdryl_core::{I256, U256};
