//! The [`UInt32Scalar`] scalar.
//!
//! A single, possibly-null `uint32` value (native `u32`) of the
//! [`UInt32Type`](yggdryl_dtype::UInt32Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, UInt32Scalar};
//! use yggdryl_scalar::yggdryl_dtype::UInt32Type;
//!
//! assert_eq!(UInt32Scalar::new(42).value(), Some(&42));
//! assert!(UInt32Scalar::null().is_null());
//! assert_eq!(UInt32Type.scalar(42), UInt32Scalar::new(42)); // the data type is the factory
//! ```

crate::integer::int_scalar!(UInt32Scalar, UInt32Type, u32, "uint32", UInt32Array);
