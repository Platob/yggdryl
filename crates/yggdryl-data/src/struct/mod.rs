//! The `struct` type: [`StructType`], its traits [`RawStruct`] / [`Struct`], field
//! [`StructField`] and scalar [`StructScalar`].
//!
//! A struct value is one row of an ordered set of named child fields.
//! [`StructType`] is the concrete, *dynamic* data type — it carries Arrow
//! [`Fields`](arrow_schema::Fields) losslessly, like the dynamic
//! [`UnionType`](crate::UnionType) — with [`RawStruct`] its untyped surface and
//! the typed [`Struct`] reserved for statically-shaped structs. [`StructScalar`]
//! is a single, possibly-null row held as one one-element Arrow column per child.
//!
//! ```
//! use yggdryl_data::{arrow_array, arrow_schema, RawDataType, RawScalar, StructScalar, StructType};
//!
//! let point = StructType::new(arrow_schema::Fields::from(vec![
//!     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
//! ]));
//! assert_eq!((point.name(), point.arrow_format().as_str()), ("struct", "+s"));
//!
//! let row = StructScalar::new(
//!     point,
//!     vec![std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([7]))],
//! )
//! .unwrap();
//! assert_eq!(StructScalar::from_arrow(row.to_arrow().as_ref()).unwrap(), row);
//! ```

mod data_type;
mod field;
mod raw_struct;
mod scalar;
#[allow(clippy::module_inception)]
mod r#struct;

pub use data_type::StructType;
pub use field::StructField;
pub use r#struct::Struct;
pub use raw_struct::RawStruct;
pub use scalar::StructScalar;
