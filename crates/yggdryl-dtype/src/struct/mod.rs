//! The `struct` type: [`Struct`] and its traits [`RawStruct`] / [`TypedStruct`].
//!
//! A struct value is one row of an ordered set of named child fields. [`Struct`]
//! is the concrete, *dynamic* data type — it carries Arrow
//! [`Fields`](arrow_schema::Fields) losslessly, like the dynamic
//! [`Union`](crate::Union) — with [`RawStruct`] its untyped surface and the typed
//! [`TypedStruct`] reserved for statically-shaped structs. The matching field and
//! scalar live in `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{arrow_schema, RawDataType, Struct};
//!
//! let point = Struct::new(arrow_schema::Fields::from(vec![
//!     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
//! ]));
//! assert_eq!((point.name(), point.arrow_format().as_str()), ("struct", "+s"));
//! assert_eq!(Struct::from_arrow(&point.to_arrow()).unwrap(), point);
//! ```

mod data_type;
mod raw_struct;
mod typed_struct;

pub use data_type::Struct;
pub use raw_struct::RawStruct;
pub use typed_struct::TypedStruct;
