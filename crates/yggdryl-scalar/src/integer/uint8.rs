//! The [`UInt8`] scalar.
//!
//! A single, possibly-null `uint8` value (native `u8`) of the
//! [`uint8`](yggdryl_dtype::UInt8) data type.
//!
//! ```
//! use yggdryl_scalar::{RawScalar, UInt8};
//!
//! assert_eq!(UInt8::new(42).value(), Some(&42));
//! assert!(UInt8::null().is_null());
//! ```

crate::integer::int_scalar!(UInt8, u8, "uint8", UInt8Array);
