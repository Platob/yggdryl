//! The `struct` type: [`StructType`] and its traits [`Struct`] / [`TypedStruct`].
//!
//! A struct value is one row of an ordered set of named child fields. [`StructType`]
//! is the concrete, *dynamic* data type — it carries Arrow
//! [`Fields`](arrow_schema::Fields) losslessly, like the dynamic
//! [`UnionType`](crate::UnionType) — with [`Struct`] its untyped surface and the
//! typed [`TypedStruct`] reserved for statically-shaped structs. The matching field
//! and scalar live in `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{arrow_schema, DataType, StructType};
//!
//! let point = StructType::new(arrow_schema::Fields::from(vec![
//!     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
//! ]));
//! assert_eq!((point.name(), point.arrow_format().as_str()), ("struct", "+s"));
//! assert_eq!(StructType::from_arrow(&point.to_arrow()).unwrap(), point);
//! ```

mod data_type;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod r#struct;
mod typed_struct;

pub use data_type::StructType;
pub use r#struct::Struct;
pub use typed_struct::TypedStruct;
