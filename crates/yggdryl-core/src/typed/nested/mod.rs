//! `nested` — the **recursive, heterogeneous** column layer grown on top of the flat typed families.
//!
//! Where [`fixedbyte`](crate::typed::fixedbyte) / [`varbyte`](crate::typed::varbyte) give one column
//! **one** element type, `nested` lets columns of **different** types compose and recurse. Two erased
//! keystones make that possible:
//!
//! - [`Column`] — a tagged union over every concrete data column, so a heterogeneous set of children
//!   can coexist; its element is the erased [`Value`].
//! - [`ColumnField`] — the recursive schema descriptor parallel to `Column` (a leaf's
//!   [`HeaderField`](crate::typed::HeaderField) or a nested struct's [`StructField`]).
//!
//! On top of them the [`struct_`] family provides the first nested carrier: [`StructSerie`] (the
//! "table"), [`StructScalar`] (one row), and [`StructField`] (the schema). A struct is itself a
//! [`Serie`](crate::typed::Serie), so it nests inside another struct — navigation flows **downward**
//! through [`StructSerie::column_path`] into inner children.
//!
//! `List` and `Map` are reserved: their [`DataTypeId`](crate::datatype_id::DataTypeId) band members
//! already exist, and their carriers plus the matching [`Column`] / [`Value`] / [`ColumnField`]
//! variants land in a later phase — the enums are `#[non_exhaustive]` to keep that additive.

mod column;
mod column_field;
mod value;

pub mod struct_;

pub use column::Column;
pub use column_field::ColumnField;
pub use struct_::{StructField, StructScalar, StructSerie};
pub use value::Value;
