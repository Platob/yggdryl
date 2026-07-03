//! The [`Int16Field`] field.
//!
//! A nullable `int16` column: a name paired with the
//! [`Int16Type`](yggdryl_dtype::Int16Type) data type (native `i16`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, Int16Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, Int16Type};
//!
//! let id = Int16Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int16", false));
//! assert_eq!(Int16Field::from_arrow(&id.to_arrow()).unwrap(), id);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(Int16Type.field("id", false), id);
//! ```

crate::integer::int_field!(Int16Field, Int16Type, i16, "int16");
