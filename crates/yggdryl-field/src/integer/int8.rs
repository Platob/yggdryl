//! The [`Int8Field`] field.
//!
//! A nullable `int8` column: a name paired with the
//! [`Int8Type`](yggdryl_dtype::Int8Type) data type (native `i8`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, Int8Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, Int8Type};
//!
//! let id = Int8Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int8", false));
//! assert_eq!(Int8Field::from_arrow(&id.to_arrow()).unwrap(), id);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(Int8Type.field("id", false), id);
//! ```

crate::integer::int_field!(Int8Field, Int8Type, i8, "int8");
