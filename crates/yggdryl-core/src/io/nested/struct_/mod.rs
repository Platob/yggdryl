//! `io::nested::struct_` — the **struct** family: an ordered, named set of heterogeneous child
//! columns. It mirrors the leaf families' file layout (`dtype` / `field` / `scalar` / `serie`) over
//! the shared `io` root traits:
//!
//! | root trait ([`crate::io`]) | concrete (struct) |
//! | --- | --- |
//! | [`DataType`](crate::io::DataType) | [`StructType`] |
//! | [`FieldType`](crate::io::FieldType) | [`StructField`] (the centralized schema) |
//! | [`ScalarType`](crate::io::ScalarType) | [`StructScalar`] |
//! | [`SerieType`](crate::io::SerieType) | [`StructSerie`] |
//!
//! [`StructField`] is the **single source of truth** for a struct's shape — it maps to both an Arrow
//! `Field` (`Struct`) and an Arrow `Schema`. [`StructSerie`] is the bridge to Arrow's `StructArray`
//! and `RecordBatch`. The module is named `struct_` (a trailing underscore) because `struct` is a
//! Rust keyword.

mod dtype;
mod field;
pub(crate) mod scalar;
mod serie;

pub use dtype::StructType;
pub use field::StructField;
pub use scalar::StructScalar;
pub use serie::StructSerie;
