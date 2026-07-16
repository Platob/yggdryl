//! `io::nested::map` — the **map** family: an unordered set of `key → value` entries (Arrow `Map`),
//! the **optimized alias** of `List<Struct<{key non-null, value}>>`. It mirrors the `list` / `struct_`
//! families' file layout (`dtype` / `field` / `scalar` / `serie`) over the shared `io` root traits:
//!
//! | root trait ([`crate::io`]) | concrete (map) |
//! | --- | --- |
//! | [`DataType`](crate::io::DataType) | [`MapType`] |
//! | [`FieldType`](crate::io::FieldType) | [`MapField`] (the centralized schema) |
//! | [`ScalarType`](crate::io::ScalarType) | [`MapScalar`] |
//! | [`SerieType`](crate::io::SerieType) | [`MapSerie`] |
//!
//! Physically a map column reuses the `list` + `struct` machinery: a two-column
//! [`StructSerie`](crate::io::nested::StructSerie) of `entries` (`key` non-null, `value` nullable)
//! plus `i32` offsets (row `i` is `entries[offsets[i] .. offsets[i + 1]]`), an optional top-level
//! validity mask, and a `keys_sorted` flag. [`MapField`] is the **single source of truth** for a map's
//! shape (↔ an Arrow `Map` `Field`); [`MapSerie`] bridges to Arrow's `MapArray`.

mod dtype;
mod field;
pub(crate) mod scalar;
mod serie;

pub use dtype::MapType;
pub use field::MapField;
pub use scalar::MapScalar;
pub use serie::MapSerie;
