//! [`F64Scalar`] — a single, possibly-null `float64` value.

use super::primitive::primitive_scalar;

primitive_scalar!(F64Scalar, F64Type, f64, "float64", 1.5);
