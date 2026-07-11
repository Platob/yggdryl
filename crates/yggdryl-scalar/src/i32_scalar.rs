//! [`I32Scalar`] — a single, possibly-null `int32` value.

use super::primitive::primitive_scalar;

primitive_scalar!(I32Scalar, I32Type, i32, "int32", 1);
