//! The [`Int32Field`] field.
//!
//! A nullable `int32` column: a name paired with the
//! [`Int32Type`](yggdryl_dtype::Int32Type) data type (native `i32`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, Int32Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, Int32Type};
//!
//! let id = Int32Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int32", false));
//! assert_eq!(Int32Field::from_arrow(&id.to_arrow()).unwrap(), id);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(Int32Type.field("id", false), id);
//! ```

crate::integer::int_field!(Int32Field, Int32Type, i32, "int32");
