//! # yggdryl-scalar
//!
//! The scalar value layer for yggdryl: a **scalar** is a single value paired with the
//! schema [`Field`](yggdryl_schema::Field) that describes it. It is the value-layer
//! mirror of the schema's `DataType` / `Field`, following the same pattern:
//!
//! - [`Scalar`]`<T>` is the base trait, generic over the native value type `T`. Every
//!   scalar exposes its [`value`](Scalar::value) and [`field`](Scalar::field), and —
//!   through the field — its [`name`](Scalar::name), [`dtype`](Scalar::dtype) and
//!   [`metadata`](Scalar::metadata). The [`PrimitiveScalar`] marker pairs with
//!   [`PrimitiveField`](yggdryl_schema::PrimitiveField).
//! - The signed [`Int8Scalar`]…[`Int256Scalar`] and unsigned [`UInt8Scalar`]…
//!   [`UInt256Scalar`] are the concrete primitive scalars — each built straight from
//!   its native Rust value (`i8`…`i128` / [`I256`], `u8`…`u128` / [`U256`]).
//! - [`AnyScalar`] is the dynamic scalar (an [`Any`](yggdryl_schema::Any) value + an
//!   [`AnyField`](yggdryl_schema::AnyField)), likewise built from any native type.
//! - [`StructScalar`] is the nested scalar, built from a **collection** of child
//!   scalars — the way scalars compose into rows. A struct scalar can itself be a
//!   child (`StructScalar` → [`AnyScalar`]), so scalars nest recursively.
//!
//! ```
//! use yggdryl_scalar::{AnyScalar, Int32Scalar, Scalar, StructScalar};
//!
//! // Build primitive scalars from native values, compose them into a row.
//! let row = StructScalar::new(
//!     "row",
//!     vec![AnyScalar::from(1i32), Int32Scalar::from(2).with_name("n".into()).into()],
//! );
//! assert_eq!(row.value().len(), 2);
//! ```

mod any_scalar;
mod integer_scalar;
mod primitive_scalar;
mod scalar;
mod struct_scalar;

pub use any_scalar::AnyScalar;
pub use integer_scalar::{
    Int128Scalar, Int16Scalar, Int256Scalar, Int32Scalar, Int64Scalar, Int8Scalar, UInt128Scalar,
    UInt16Scalar, UInt256Scalar, UInt32Scalar, UInt64Scalar, UInt8Scalar,
};
pub use primitive_scalar::PrimitiveScalar;
pub use scalar::Scalar;
pub use struct_scalar::StructScalar;
// The 256-bit native value types live in the core crate; re-export for convenience.
pub use yggdryl_core::{I256, U256};
