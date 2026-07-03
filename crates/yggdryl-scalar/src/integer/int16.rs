//! The [`Int16Scalar`] scalar.
//!
//! A single, possibly-null `int16` value (native `i16`) of the
//! [`Int16Type`](yggdryl_dtype::Int16Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, Int16Scalar};
//! use yggdryl_scalar::yggdryl_dtype::Int16Type;
//!
//! assert_eq!(Int16Scalar::new(42).value(), Some(&42));
//! assert!(Int16Scalar::null().is_null());
//! assert_eq!(Int16Type.scalar(42), Int16Scalar::new(42)); // the data type is the factory
//! ```

crate::integer::int_scalar!(Int16Scalar, Int16Type, i16, "int16", Int16Array);
