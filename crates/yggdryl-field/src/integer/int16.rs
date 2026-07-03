//! The [`Int16`] field.
//!
//! A nullable `int16` column: a name paired with the
//! [`int16`](yggdryl_dtype::Int16) data type (native `i16`).
//!
//! ```
//! use yggdryl_field::{RawField, Int16};
//! use yggdryl_field::yggdryl_dtype::RawDataType;
//!
//! let id = Int16::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int16", false));
//! assert_eq!(Int16::from_arrow(&id.to_arrow()).unwrap(), id);
//! ```

crate::integer::int_field!(Int16, i16, "int16");
