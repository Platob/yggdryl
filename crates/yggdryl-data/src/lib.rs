//! # yggdryl-data
//!
//! The Apache Arrow-centralized data-model layer for yggdryl, built on top of
//! `yggdryl-core`. It defines the physical type system — data types, fields and
//! scalars — with zero-copy FFI and Arrow interop in mind.
//!
//! This is the scaffold: the abstract base traits [`RawDataType`], [`RawField`] and
//! [`RawScalar`] that concrete types (`Int32`, `Utf8`, `Boolean`, …), their scalars
//! and their Arrow bridges implement as the layer grows. Add each new type in its own
//! module file under `datatype/`, re-exported at the crate root — following the rules
//! in `CLAUDE.md`.

mod datatype;
pub use datatype::{RawDataType, RawField, RawScalar};
