//! The [`Float32Scalar`] scalar.
//!
//! A single, possibly-null `float32` value (native `f32`) of the
//! [`Float32Type`](yggdryl_dtype::Float32Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, Float32Scalar};
//! use yggdryl_scalar::yggdryl_dtype::Float32Type;
//!
//! assert_eq!(Float32Scalar::new(1.5).value(), Some(&1.5));
//! assert!(Float32Scalar::null().is_null());
//! assert_eq!(Float32Type.scalar(1.5), Float32Scalar::new(1.5)); // the data type is the factory
//!
//! // A whole-number float reads as an integer; a fractional one is inexact.
//! assert_eq!(Float32Scalar::new(3.0).as_i64().unwrap(), 3);
//! assert!(Float32Scalar::new(1.5).as_i64().is_err());
//! assert_eq!(Float32Scalar::new(1.5).as_f64().unwrap(), 1.5); // widens exactly
//! ```

crate::float::float_scalar!(Float32Scalar, Float32Type, f32, "float32", Float32Array);
