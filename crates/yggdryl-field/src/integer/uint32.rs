//! The [`UInt32`] field.
//!
//! A nullable `uint32` column: a name paired with the
//! [`uint32`](yggdryl_dtype::UInt32) data type (native `u32`).
//!
//! ```
//! use yggdryl_field::{RawField, UInt32};
//! use yggdryl_field::yggdryl_dtype::RawDataType;
//!
//! let id = UInt32::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint32", false));
//! assert_eq!(UInt32::from_arrow(&id.to_arrow()).unwrap(), id);
//! ```

crate::integer::int_field!(UInt32, u32, "uint32");
