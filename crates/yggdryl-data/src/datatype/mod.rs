//! The data-model type system.
//!
//! Three layers of traits, plus concrete types:
//!
//! - **Untyped base:** [`RawDataType`], [`RawField`], [`RawScalar`] — the FFI-facing
//!   descriptors.
//! - **Typed:** [`DataType`], [`Field`], [`Scalar`] — the same, tied to a native Rust
//!   type `T` (with the [`DataType`] byte codec).
//! - **Categories:** [`Primitive`], [`Logical`], [`Nested`] — how a type is shaped.
//!
//! Concrete types land one file per type: [`Int64`] and [`Int64Scalar`] are the first.

mod data_type_id;
mod error;
mod raw_data_type;
mod raw_field;
mod raw_scalar;

mod data_type;
mod field;
mod scalar;

mod logical;
mod nested;
mod primitive;

mod int64;
mod int64_scalar;

pub use data_type_id::DataTypeId;
pub use error::DataError;
pub use raw_data_type::RawDataType;
pub use raw_field::RawField;
pub use raw_scalar::RawScalar;

pub use data_type::DataType;
pub use field::Field;
pub use scalar::Scalar;

pub use logical::Logical;
pub use nested::Nested;
pub use primitive::Primitive;

pub use int64::Int64;
pub use int64_scalar::Int64Scalar;
