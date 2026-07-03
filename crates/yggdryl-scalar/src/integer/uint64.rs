//! The [`UInt64`] scalar.
//!
//! A single, possibly-null `uint64` value (native `u64`) of the
//! [`uint64`](yggdryl_dtype::UInt64) data type.
//!
//! ```
//! use yggdryl_scalar::{RawScalar, UInt64};
//!
//! assert_eq!(UInt64::new(42).value(), Some(&42));
//! assert!(UInt64::null().is_null());
//! ```

crate::integer::int_scalar!(UInt64, u64, "uint64", UInt64Array);
