//! # yggdryl-scalar
//!
//! The scalar value layer for yggdryl: a **scalar** is a single value paired with the
//! schema [`Field`](yggdryl_schema::Field) that describes it. It is the value-layer
//! mirror of the schema's `DataType` / `Field`, following the same pattern. The types
//! wear simple names — [`Any`], [`Struct`], [`Int64`] — since they already live under
//! the `scalar` namespace:
//!
//! - [`Scalar`]`<T>` is the base trait, generic over the native value type `T`. Every
//!   scalar exposes its [`value`](Scalar::value) and [`field`](Scalar::field), and —
//!   through the field — its [`name`](Scalar::name), [`dtype`](Scalar::dtype) and
//!   [`metadata`](Scalar::metadata). The [`PrimitiveScalar`] marker pairs with
//!   [`PrimitiveField`](yggdryl_schema::PrimitiveField).
//! - The signed [`Int8`]…[`Int256`] and unsigned [`UInt8`]…[`UInt256`] are the concrete
//!   primitive scalars — each built straight from its native Rust value (`i8`…`i128` /
//!   [`I256`], `u8`…`u128` / [`U256`]).
//! - [`Any`] is the dynamic scalar (an [`Any`](yggdryl_schema::Any) value + an
//!   [`AnyField`](yggdryl_schema::AnyField)), likewise built from any native type, with
//!   atomic accessors (`as_i32`, …) reading the value at its native type.
//! - [`Struct`] is the nested scalar, built from a **collection** of child scalars —
//!   the way scalars compose into rows. A struct scalar can itself be a child
//!   ([`Struct`] → [`Any`]), so scalars nest recursively.
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

mod any_scalar;
mod integer_scalar;
mod primitive_scalar;
mod scalar;
mod struct_scalar;

pub use any_scalar::Any;
pub use integer_scalar::{
    Int128, Int16, Int256, Int32, Int64, Int8, UInt128, UInt16, UInt256, UInt32, UInt64, UInt8,
};
pub use primitive_scalar::PrimitiveScalar;
pub use scalar::Scalar;
pub use struct_scalar::Struct;
// The 256-bit native value types live in the core crate; re-export for convenience.
pub use yggdryl_core::{I256, U256};
