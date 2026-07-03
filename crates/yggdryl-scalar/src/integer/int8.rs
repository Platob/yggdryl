//! The [`Int8Scalar`] scalar.
//!
//! A single, possibly-null `int8` value (native `i8`) of the
//! [`Int8Type`](yggdryl_dtype::Int8Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, Int8Scalar};
//! use yggdryl_scalar::yggdryl_dtype::Int8Type;
//!
//! assert_eq!(Int8Scalar::new(42).value(), Some(&42));
//! assert!(Int8Scalar::null().is_null());
//! assert_eq!(Int8Type.scalar(42), Int8Scalar::new(42)); // the data type is the factory
//! ```

crate::integer::int_scalar!(Int8Scalar, Int8Type, i8, "int8", Int8Array);
