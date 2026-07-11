//! [`F32Scalar`] — a single, possibly-null `float32` value.

use super::primitive::primitive_scalar;

primitive_scalar!(F32Scalar, F32Type, f32, "float32", 1.5);
