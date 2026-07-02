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
//! Concrete types land one file per type under `datatype/`; [`Int64`] and
//! [`Int64Scalar`] are the first. Add more following the rules in `CLAUDE.md`.

mod datatype;
pub use datatype::{
    DataError, DataType, DataTypeId, Field, Int64, Int64Scalar, Logical, Nested, Primitive,
    RawDataType, RawField, RawScalar, Scalar,
};
