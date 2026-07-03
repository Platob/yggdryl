//! The [`Int64`] scalar.
//!
//! A single, possibly-null `int64` value (native `i64`) of the
//! [`int64`](yggdryl_dtype::Int64) data type.
//!
//! ```
//! use yggdryl_scalar::{RawScalar, Int64};
//!
//! assert_eq!(Int64::new(42).value(), Some(&42));
//! assert!(Int64::null().is_null());
//! ```

crate::integer::int_scalar!(Int64, i64, "int64", Int64Array);
