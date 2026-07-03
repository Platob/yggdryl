//! The [`UInt16`] field.
//!
//! A nullable `uint16` column: a name paired with the
//! [`uint16`](yggdryl_dtype::UInt16) data type (native `u16`).
//!
//! ```
//! use yggdryl_field::{RawField, UInt16};
//! use yggdryl_field::yggdryl_dtype::RawDataType;
//!
//! let id = UInt16::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint16", false));
//! assert_eq!(UInt16::from_arrow(&id.to_arrow()).unwrap(), id);
//! ```

crate::integer::int_field!(UInt16, u16, "uint16");
