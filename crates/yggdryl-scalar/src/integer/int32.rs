//! The [`Int32Scalar`] scalar.
//!
//! A single, possibly-null `int32` value (native `i32`) of the
//! [`Int32Type`](yggdryl_dtype::Int32Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, Int32Scalar};
//! use yggdryl_scalar::yggdryl_dtype::Int32Type;
//!
//! assert_eq!(Int32Scalar::new(42).value(), Some(&42));
//! assert!(Int32Scalar::null().is_null());
//! assert_eq!(Int32Type.scalar(42), Int32Scalar::new(42)); // the data type is the factory
//! ```

crate::integer::int_scalar!(Int32Scalar, Int32Type, i32, "int32", Int32Array);
