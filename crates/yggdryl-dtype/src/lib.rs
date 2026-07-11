//! **yggdryl-dtype** — the Apache Arrow **data type** layer.
//!
//! A data type is described by a small trait hierarchy, mirrored in the
//! [`field`](https://docs.rs/yggdryl-field) and
//! [`scalar`](https://docs.rs/yggdryl-scalar) layers above:
//!
//! * [`DataType`] — the FFI-opaque base: [`name`](DataType::name),
//!   [`byte_width`](DataType::byte_width), [`to_arrow`](DataType::to_arrow), and the
//!   [`serialize_bytes`](DataType::serialize_bytes) codec.
//! * [`TypedDataType<T>`] — the Rust-only value-typed extension (the value↔bytes codec
//!   over the native `T`), like the core's `TypedConverter<S, T>`.
//! * the **category** traits [`PrimitiveType`] / [`TypedPrimitiveType<T>`], with
//!   [`LogicalType`] / [`NestedType`] (and their typed variants) as scaffolding for the
//!   logical and nested types to come.
//!
//! The concrete **primitive** types are the ten native numerics ([`I8Type`] …
//! [`F64Type`], stamped from one `primitive_type!` macro) plus the bit-packed
//! [`BooleanType`]. Each converts to and from its Arrow [`DataType`](arrow_schema::DataType)
//! and round-trips through bytes; the numerics also map to the core
//! [`PrimitiveType`](yggdryl_core::PrimitiveType) runtime tag.
//!
//! ## Canonical typing
//!
//! This crate's [`PrimitiveType`] trait is the **canonical** primitive-typing API for
//! every layer above `yggdryl-core`. The core's `PrimitiveType` *enum* stays where it
//! is — the converter is keyed on it and `dtype` depends on `core`, not the reverse —
//! but it is demoted to that low-level FFI tag; the two interoperate through
//! [`PrimitiveType::primitive_tag`] and each type's `from_primitive_tag`.
//!
//! ## Arrow interop is Rust-only
//!
//! [`to_arrow`](DataType::to_arrow) / `from_arrow` exchange `arrow_schema` values, which
//! do not cross the FFI boundary, so — like the core buffers' `from_arrow`/`to_arrow` —
//! they are **not** replicated in the Python and Node bindings (which expose the name,
//! width, byte codec, and value semantics).

mod primitive;

mod boolean_type;
mod data_type;
mod dtype_error;
mod f32_type;
mod f64_type;
mod i16_type;
mod i32_type;
mod i64_type;
mod i8_type;
mod logical_type;
mod nested_type;
mod primitive_type;
mod typed_data_type;
mod typed_logical_type;
mod typed_nested_type;
mod typed_primitive_type;
mod u16_type;
mod u32_type;
mod u64_type;
mod u8_type;

pub use boolean_type::BooleanType;
pub use data_type::DataType;
pub use dtype_error::DTypeError;
pub use f32_type::F32Type;
pub use f64_type::F64Type;
pub use i16_type::I16Type;
pub use i32_type::I32Type;
pub use i64_type::I64Type;
pub use i8_type::I8Type;
pub use logical_type::LogicalType;
pub use nested_type::NestedType;
pub use primitive_type::PrimitiveType;
pub use typed_data_type::TypedDataType;
pub use typed_logical_type::TypedLogicalType;
pub use typed_nested_type::TypedNestedType;
pub use typed_primitive_type::TypedPrimitiveType;
pub use u16_type::U16Type;
pub use u32_type::U32Type;
pub use u64_type::U64Type;
pub use u8_type::U8Type;
