//! The [`UInt64`] field.
//!
//! A nullable `uint64` column: a name paired with the
//! [`uint64`](yggdryl_dtype::UInt64) data type (native `u64`).
//!
//! ```
//! use yggdryl_field::{RawField, UInt64};
//! use yggdryl_field::yggdryl_dtype::RawDataType;
//!
//! let id = UInt64::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint64", false));
//! assert_eq!(UInt64::from_arrow(&id.to_arrow()).unwrap(), id);
//! ```

crate::integer::int_field!(UInt64, u64, "uint64");
