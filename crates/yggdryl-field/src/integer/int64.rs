//! The [`Int64`] field.
//!
//! A nullable `int64` column: a name paired with the
//! [`int64`](yggdryl_dtype::Int64) data type (native `i64`).
//!
//! ```
//! use yggdryl_field::{RawField, Int64};
//! use yggdryl_field::yggdryl_dtype::RawDataType;
//!
//! let id = Int64::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "int64", false));
//! assert_eq!(Int64::from_arrow(&id.to_arrow()).unwrap(), id);
//! ```

crate::integer::int_field!(Int64, i64, "int64");
