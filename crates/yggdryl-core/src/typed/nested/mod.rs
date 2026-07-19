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
//! On top of them three nested carriers grow, each a [`Serie`](crate::typed::Serie) that is itself a
//! [`Column`] and so nests inside any other:
//!
//! - [`struct_`] — [`StructSerie`] (the "table"), [`StructScalar`] (one row), [`StructField`] (the
//!   schema). Navigation flows **downward** through [`StructSerie::column_path`] into inner children.
//! - [`list`] — [`ListSerie`] (an offsets buffer over a flattened child [`Column`]), [`ListScalar`]
//!   (one list element), [`ListField`] (the schema, with its item field).
//! - [`map`] — [`MapSerie`] (an offsets buffer over a two-column key / value entries struct),
//!   [`MapScalar`] (one map element), [`MapField`] (the schema, with its key / value fields).

mod column;
mod column_field;
mod convert;
mod value;

pub mod list;
pub mod map;
pub mod struct_;

pub use column::Column;
pub use column_field::ColumnField;
pub(crate) use convert::set_any_dtype_error;
pub use convert::{FromValue, ToValue};
pub use list::{ListField, ListScalar, ListSerie};
pub use map::{MapField, MapScalar, MapSerie};
pub use struct_::{StructField, StructScalar, StructSerie};
pub use value::Value;
