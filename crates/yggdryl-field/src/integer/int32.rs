//! The [`Int32`] field.
//!
//! A nullable `int32` column: a name paired with the
//! [`int32`](yggdryl_dtype::Int32) data type (native `i32`).
//!
//! ```
//! use yggdryl_field::{RawField, Int32};
//! use yggdryl_field::yggdryl_dtype::RawDataType;
//!
//! let id = Int32::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int32", false));
//! assert_eq!(Int32::from_arrow(&id.to_arrow()).unwrap(), id);
//! ```

crate::integer::int_field!(Int32, i32, "int32");
