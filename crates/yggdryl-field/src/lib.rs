//! **yggdryl-field** — the Apache Arrow **field** layer.
//!
//! A field is a named, nullable [`yggdryl_dtype`] data type. It is described by the same
//! trait hierarchy shape as the [`dtype`](yggdryl_dtype) layer below and the
//! [`scalar`](https://docs.rs/yggdryl-scalar) layer above:
//!
//! * [`Field`] — the FFI-opaque base: [`name`](Field::name),
//!   [`is_nullable`](Field::is_nullable), [`arrow_data_type`](Field::arrow_data_type) /
//!   [`to_arrow`](Field::to_arrow), and the [`serialize_bytes`](Field::serialize_bytes)
//!   codec.
//! * [`TypedField<DT, T>`] — the Rust-only extension exposing the concrete data type
//!   `DT` and native value type `T`.
//! * the **category** traits [`PrimitiveField`] / [`TypedPrimitiveField<DT, T>`], with
//!   [`LogicalField`] / [`NestedField`] (and their typed variants) as scaffolding.
//!
//! The concrete **primitive** fields are the ten native numerics ([`I8Field`] …
//! [`F64Field`]) plus [`BooleanField`], all stamped from one `primitive_field!`
//! macro — a field never touches the value codec, so `Boolean` is not a special case
//! here (unlike the dtype and buffer layers). Each converts to and from an Arrow
//! [`Field`](arrow_schema::Field) and round-trips through bytes.
//!
//! ## Arrow interop is Rust-only
//!
//! [`to_arrow`](Field::to_arrow) / `from_arrow` exchange `arrow_schema` values, which do
//! not cross the FFI boundary, so — like the dtype layer's — they are **not** replicated
//! in the Python and Node bindings (which expose the name, nullability, data type, byte
//! codec, and value semantics).

mod primitive;

mod boolean_field;
mod f32_field;
mod f64_field;
mod field;
mod field_error;
mod i16_field;
mod i32_field;
mod i64_field;
mod i8_field;
mod logical_field;
mod nested_field;
mod primitive_field;
mod to_field;
mod typed_field;
mod typed_logical_field;
mod typed_nested_field;
mod typed_primitive_field;
mod u16_field;
mod u32_field;
mod u64_field;
mod u8_field;

pub use boolean_field::BooleanField;
pub use f32_field::F32Field;
pub use f64_field::F64Field;
pub use field::Field;
pub use field_error::FieldError;
pub use i16_field::I16Field;
pub use i32_field::I32Field;
pub use i64_field::I64Field;
pub use i8_field::I8Field;
pub use logical_field::LogicalField;
pub use nested_field::NestedField;
pub use primitive_field::PrimitiveField;
pub use to_field::ToField;
pub use typed_field::TypedField;
pub use typed_logical_field::TypedLogicalField;
pub use typed_nested_field::TypedNestedField;
pub use typed_primitive_field::TypedPrimitiveField;
pub use u16_field::U16Field;
pub use u32_field::U32Field;
pub use u64_field::U64Field;
pub use u8_field::U8Field;
