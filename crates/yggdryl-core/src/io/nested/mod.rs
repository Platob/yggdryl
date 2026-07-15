//! `io::nested` — the **nested / composite** typed layer: the sibling of [`fixed`](crate::io::fixed)
//! and [`var`](crate::io::var) for types whose values are composed of other columns. Phase one ships
//! the **struct** family; `list` (Arrow `List`) and `map` follow.
//!
//! Nested columns hold *child columns of arbitrary type* — including other nested columns. Rather
//! than a bespoke column type, they build entirely on the family-agnostic erased primitives at the
//! [`io`](crate::io) root:
//!
//! - [`AnySerie`](crate::io::AnySerie) (held as `Box<dyn AnySerie>`) — the erased child column;
//!   every concrete `Serie` implements it, and [`StructSerie`] does too (the recursion).
//! - [`AnyField`](crate::io::AnyField) — the recursive erased field a schema holds as children.
//! - [`AnyScalar`](crate::io::AnyScalar) — the erased cell a struct row is built from.
//!
//! So the nested types are thin: [`StructField`] is a validated struct-shaped `AnyField`, and
//! [`StructSerie`] is a schema + `Vec<Box<dyn AnySerie>>`, delegating length / serialization /
//! equality / Arrow to the wrapped Series. Arrow stays behind the `arrow` feature and never appears
//! in a public signature.

pub mod struct_;

pub use struct_::{StructField, StructScalar, StructSerie, StructType};
