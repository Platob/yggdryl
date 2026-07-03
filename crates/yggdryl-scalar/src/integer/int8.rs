//! The [`Int8`] scalar.
//!
//! A single, possibly-null `int8` value (native `i8`) of the
//! [`int8`](yggdryl_dtype::Int8) data type.
//!
//! ```
//! use yggdryl_scalar::{RawScalar, Int8};
//!
//! assert_eq!(Int8::new(42).value(), Some(&42));
//! assert!(Int8::null().is_null());
//! ```

crate::integer::int_scalar!(Int8, i8, "int8", Int8Array);
