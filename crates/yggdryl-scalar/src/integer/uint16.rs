//! The [`UInt16Scalar`] scalar.
//!
//! A single, possibly-null `uint16` value (native `u16`) of the
//! [`UInt16Type`](yggdryl_dtype::UInt16Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, UInt16Scalar};
//! use yggdryl_scalar::yggdryl_dtype::UInt16Type;
//!
//! assert_eq!(UInt16Scalar::new(42).value(), Some(&42));
//! assert!(UInt16Scalar::null().is_null());
//! assert_eq!(UInt16Type.scalar(42), UInt16Scalar::new(42)); // the data type is the factory
//! ```

crate::integer::int_scalar!(UInt16Scalar, UInt16Type, u16, "uint16", UInt16Array);
