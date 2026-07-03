//! The [`UInt32`] scalar.
//!
//! A single, possibly-null `uint32` value (native `u32`) of the
//! [`uint32`](yggdryl_dtype::UInt32) data type.
//!
//! ```
//! use yggdryl_scalar::{RawScalar, UInt32};
//!
//! assert_eq!(UInt32::new(42).value(), Some(&42));
//! assert!(UInt32::null().is_null());
//! ```

crate::integer::int_scalar!(UInt32, u32, "uint32", UInt32Array);
