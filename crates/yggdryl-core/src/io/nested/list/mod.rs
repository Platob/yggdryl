//! `io::nested::list` — the **list** family: a variable-size sequence of a single element type
//! (Arrow `List`, `i32` offsets). It mirrors the `struct_` family's file layout
//! (`dtype` / `field` / `scalar` / `serie`) over the shared `io` root traits:
//!
//! | root trait ([`crate::io`]) | concrete (list) |
//! | --- | --- |
//! | [`DataType`](crate::io::DataType) | [`ListType`] |
//! | [`FieldType`](crate::io::FieldType) | [`ListField`] (the centralized schema) |
//! | [`ScalarType`](crate::io::ScalarType) | [`ListScalar`] |
//! | [`SerieType`](crate::io::SerieType) | [`ListSerie`] |
//!
//! Physically a list column is a single flattened child column plus `i32` offsets: row `i` is
//! `child[offsets[i] .. offsets[i + 1]]`. [`ListField`] is the **single source of truth** for a
//! list's shape (↔ an Arrow `List` `Field`); [`ListSerie`] bridges to Arrow's `ListArray`. Only the
//! `i32`-offset `List` is modeled; `LargeList` / `FixedSizeList` are reserved at their own type ids.

mod dtype;
mod field;
pub(crate) mod scalar;
mod serie;

pub use dtype::ListType;
pub use field::ListField;
pub use scalar::ListScalar;
pub use serie::ListSerie;
