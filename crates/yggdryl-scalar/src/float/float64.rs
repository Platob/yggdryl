//! The [`Float64Scalar`] scalar.
//!
//! A single, possibly-null `float64` value (native `f64`) of the
//! [`Float64Type`](yggdryl_dtype::Float64Type) data type.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, Float64Scalar};
//! use yggdryl_scalar::yggdryl_dtype::Float64Type;
//!
//! assert_eq!(Float64Scalar::new(1.5).value(), Some(&1.5));
//! assert!(Float64Scalar::null().is_null());
//! assert_eq!(Float64Type.scalar(1.5), Float64Scalar::new(1.5)); // the data type is the factory
//!
//! // A whole-number float reads as an integer; a value that will not narrow to f32
//! // exactly is inexact.
//! assert_eq!(Float64Scalar::new(3.0).as_i64().unwrap(), 3);
//! assert_eq!(Float64Scalar::new(1.5).as_f32().unwrap(), 1.5); // narrows exactly
//! assert!(Float64Scalar::new(0.1).as_f32().is_err()); // 0.1 has no exact f32
//! ```

crate::float::float_scalar!(Float64Scalar, Float64Type, f64, "float64", Float64Array);
