//! The [`Int32`] scalar.
//!
//! A single, possibly-null `int32` value (native `i32`) of the
//! [`int32`](yggdryl_dtype::Int32) data type.
//!
//! ```
//! use yggdryl_scalar::{RawScalar, Int32};
//!
//! assert_eq!(Int32::new(42).value(), Some(&42));
//! assert!(Int32::null().is_null());
//! ```

crate::integer::int_scalar!(Int32, i32, "int32", Int32Array);
