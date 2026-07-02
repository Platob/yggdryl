//! # yggdryl-data
//!
//! The Apache Arrow-centralized data-model layer for yggdryl, built on top of
//! `yggdryl-core`. It defines the physical type system — data types, fields and
//! scalars — with zero-copy FFI and Arrow interop in mind.
//!
//! The type system comes in three layers of traits, each re-exported at the crate
//! root:
//!
//! - The **untyped base** [`RawDataType`], [`RawField`] and [`RawScalar`] — the
//!   FFI-facing descriptors.
//! - The **typed** [`DataType`], [`Field`] and [`Scalar`], parameterised by a native
//!   Rust type `T` (the [`DataType`] codec bridges a `T` to and from Arrow bytes).
//! - The **categories** [`Primitive`], [`Logical`] and [`Nested`] describing a type's
//!   shape.
//!
//! Concrete types live in per-family modules — the [`integer`] module holds every
//! signed and unsigned integer (each its own data type, field and scalar). Add more
//! following the rules in `CLAUDE.md`.

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

pub mod integer;
pub use integer::{
    Int16, Int16Field, Int16Scalar, Int32, Int32Field, Int32Scalar, Int64, Int64Field, Int64Scalar,
    Int8, Int8Field, Int8Scalar, UInt16, UInt16Field, UInt16Scalar, UInt32, UInt32Field,
    UInt32Scalar, UInt64, UInt64Field, UInt64Scalar, UInt8, UInt8Field, UInt8Scalar,
};
