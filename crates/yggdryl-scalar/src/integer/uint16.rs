//! The [`UInt16`] scalar.
//!
//! A single, possibly-null `uint16` value (native `u16`) of the
//! [`uint16`](yggdryl_dtype::UInt16) data type.
//!
//! ```
//! use yggdryl_scalar::{RawScalar, UInt16};
//!
//! assert_eq!(UInt16::new(42).value(), Some(&42));
//! assert!(UInt16::null().is_null());
//! ```

crate::integer::int_scalar!(UInt16, u16, "uint16", UInt16Array);
