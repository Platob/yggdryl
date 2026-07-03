//! The [`Int64Field`] field.
//!
//! A nullable `int64` column: a name paired with the
//! [`Int64Type`](yggdryl_dtype::Int64Type) data type (native `i64`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, Int64Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, Int64Type};
//!
//! let id = Int64Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int64", false));
//! assert_eq!(Int64Field::from_arrow(&id.to_arrow()).unwrap(), id);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(Int64Type.field("id", false), id);
//! ```

crate::integer::int_field!(Int64Field, Int64Type, i64, "int64");
