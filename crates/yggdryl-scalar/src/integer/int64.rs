//! The [`Int64Scalar`] scalar.
//!
//! A single, possibly-null `int64` value (native `i64`) of the
//! [`Int64Type`](yggdryl_dtype::Int64Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, Int64Scalar};
//! use yggdryl_scalar::yggdryl_dtype::Int64Type;
//!
//! assert_eq!(Int64Scalar::new(42).value(), Some(&42));
//! assert!(Int64Scalar::null().is_null());
//! assert_eq!(Int64Type.scalar(42), Int64Scalar::new(42)); // the data type is the factory
//! ```

crate::integer::int_scalar!(Int64Scalar, Int64Type, i64, "int64", Int64Array);
