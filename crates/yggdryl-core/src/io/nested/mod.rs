//! `io::nested` — the **nested / composite** typed layer: the sibling of [`fixed`](crate::io::fixed)
//! and [`var`](crate::io::var) for types whose values are composed of other columns. Phase one ships
//! the **struct** family; `list` (Arrow `List`) and `map` follow.
//!
//! A nested column holds *child columns of arbitrary type* — including other nested columns — so
//! this module introduces two recursive, type-erased carriers the flat leaf model lacks:
//!
//! - [`Column`] — the erased **data** column: a **thin enum over the crate's existing typed Series**
//!   that only wraps and delegates, so every op (length, serialization, equality, Arrow conversion)
//!   calls the wrapped `Serie`'s own implementation. It is the child carrier a [`StructSerie`] holds.
//! - [`ColumnField`] — the erased, recursive **field** descriptor: a leaf (the flat
//!   [`Field`](crate::io::fixed::Field), reused as `var` does) or a nested field. It is what a
//!   [`StructField`] schema holds as children, and it maps recursively to/from Arrow.
//!
//! [`Value`] is the erased **cell value** an erased [`Column::get`] yields (and a [`StructScalar`]
//! row is built from). Per the crate rules, Arrow stays a physical detail behind the `arrow`
//! feature and never appears in a public signature; the erased carriers are plain value enums so the
//! core builds without Arrow.

mod column;
mod column_field;
pub mod struct_;
mod value;

pub use column::Column;
pub use column_field::ColumnField;
pub use struct_::{StructField, StructScalar, StructSerie, StructType};
pub use value::Value;

// The one shared Arrow helper `StructSerie` reuses for its top-level validity (both in `nested`).
#[cfg(feature = "arrow")]
pub(crate) use column::validity_from_arrow;
