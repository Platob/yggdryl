//! `struct_` — the **struct family**: the project's centralized, table-like nested carrier.
//!
//! [`StructField`] is the schema (name + nullability + metadata + child fields), [`StructScalar`] is
//! one row, and [`StructSerie`] is the table itself — an ordered set of equal-length heterogeneous
//! [`Column`](super::Column) children with graph discovery (`column` / `column_by_name` /
//! `column_path`) and deep, in-place mutation of an inner series. It implements
//! [`Scalar`](crate::typed::Scalar) / [`Serie`](crate::typed::Serie), so a struct is itself a column
//! and nests inside another struct. (The module is named `struct_` because `struct` is a keyword.)

mod field;
mod scalar;
mod serie;

pub use field::StructField;
pub use scalar::StructScalar;
pub use serie::StructSerie;
