//! The [`Int16`] scalar.
//!
//! A single, possibly-null `int16` value (native `i16`) of the
//! [`int16`](yggdryl_dtype::Int16) data type.
//!
//! ```
//! use yggdryl_scalar::{RawScalar, Int16};
//!
//! assert_eq!(Int16::new(42).value(), Some(&42));
//! assert!(Int16::null().is_null());
//! ```

crate::integer::int_scalar!(Int16, i16, "int16", Int16Array);
