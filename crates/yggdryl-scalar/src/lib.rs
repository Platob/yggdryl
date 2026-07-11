//! **yggdryl-scalar** — the Apache Arrow **scalar** layer.
//!
//! A scalar is a single, possibly-null value of a [`yggdryl_dtype`] data type. It is
//! described by the same trait hierarchy shape as the [`dtype`](yggdryl_dtype) and
//! [`field`](https://docs.rs/yggdryl-field) layers below:
//!
//! * [`Scalar`] — the FFI-opaque base: [`is_null`](Scalar::is_null),
//!   [`arrow_data_type`](Scalar::arrow_data_type), and the
//!   [`serialize_bytes`](Scalar::serialize_bytes) codec.
//! * [`TypedScalar<DT, T>`] — the Rust-only extension exposing the typed
//!   [`value`](TypedScalar::value) and the concrete data type `DT`.
//! * the **category** traits [`PrimitiveScalar`] / [`TypedPrimitiveScalar<DT, T>`], with
//!   [`LogicalScalar`] / [`NestedScalar`] (and their typed variants) as scaffolding.
//!
//! The concrete **primitive** scalars are the ten native numerics ([`I8Scalar`] …
//! [`F64Scalar`]) plus [`BooleanScalar`], all stamped from one `primitive_scalar!`
//! macro — the value↔bytes codec is delegated to the data type's
//! [`TypedDataType`](yggdryl_dtype::TypedDataType), so `Boolean` is not a special case
//! here. Each round-trips through bytes (a null flag then the value's little-endian bytes
//! when present) and compares/hashes by those bytes, so the float scalars behave bitwise
//! and a present value never equals a null.

mod primitive;

mod boolean_scalar;
mod f32_scalar;
mod f64_scalar;
mod i16_scalar;
mod i32_scalar;
mod i64_scalar;
mod i8_scalar;
mod logical_scalar;
mod nested_scalar;
mod primitive_scalar;
mod scalar;
mod scalar_error;
mod typed_logical_scalar;
mod typed_nested_scalar;
mod typed_primitive_scalar;
mod typed_scalar;
mod u16_scalar;
mod u32_scalar;
mod u64_scalar;
mod u8_scalar;

pub use boolean_scalar::BooleanScalar;
pub use f32_scalar::F32Scalar;
pub use f64_scalar::F64Scalar;
pub use i16_scalar::I16Scalar;
pub use i32_scalar::I32Scalar;
pub use i64_scalar::I64Scalar;
pub use i8_scalar::I8Scalar;
pub use logical_scalar::LogicalScalar;
pub use nested_scalar::NestedScalar;
pub use primitive_scalar::PrimitiveScalar;
pub use scalar::Scalar;
pub use scalar_error::ScalarError;
pub use typed_logical_scalar::TypedLogicalScalar;
pub use typed_nested_scalar::TypedNestedScalar;
pub use typed_primitive_scalar::TypedPrimitiveScalar;
pub use typed_scalar::TypedScalar;
pub use u16_scalar::U16Scalar;
pub use u32_scalar::U32Scalar;
pub use u64_scalar::U64Scalar;
pub use u8_scalar::U8Scalar;
