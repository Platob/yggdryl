//! The [`Int8`] field.
//!
//! A nullable `int8` column: a name paired with the
//! [`int8`](yggdryl_dtype::Int8) data type (native `i8`).
//!
//! ```
//! use yggdryl_field::{RawField, Int8};
//! use yggdryl_field::yggdryl_dtype::RawDataType;
//!
//! let id = Int8::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int8", false));
//! assert_eq!(Int8::from_arrow(&id.to_arrow()).unwrap(), id);
//! ```

crate::integer::int_field!(Int8, i8, "int8");
