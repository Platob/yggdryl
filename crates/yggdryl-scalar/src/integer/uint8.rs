//! The [`UInt8Scalar`] scalar.
//!
//! A single, possibly-null `uint8` value (native `u8`) of the
//! [`UInt8Type`](yggdryl_dtype::UInt8Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, UInt8Scalar};
//! use yggdryl_scalar::yggdryl_dtype::UInt8Type;
//!
//! assert_eq!(UInt8Scalar::new(42).value(), Some(&42));
//! assert!(UInt8Scalar::null().is_null());
//! assert_eq!(UInt8Type.scalar(42), UInt8Scalar::new(42)); // the data type is the factory
//! ```

crate::integer::int_scalar!(UInt8Scalar, UInt8Type, u8, "uint8", UInt8Array);
