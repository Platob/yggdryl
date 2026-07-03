//! The [`UInt64Scalar`] scalar.
//!
//! A single, possibly-null `uint64` value (native `u64`) of the
//! [`UInt64Type`](yggdryl_dtype::UInt64Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, UInt64Scalar};
//! use yggdryl_scalar::yggdryl_dtype::UInt64Type;
//!
//! assert_eq!(UInt64Scalar::new(42).value(), Some(&42));
//! assert!(UInt64Scalar::null().is_null());
//! assert_eq!(UInt64Type.scalar(42), UInt64Scalar::new(42)); // the data type is the factory
//! ```

crate::integer::int_scalar!(UInt64Scalar, UInt64Type, u64, "uint64", UInt64Array);
