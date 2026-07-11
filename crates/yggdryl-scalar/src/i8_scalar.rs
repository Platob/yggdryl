//! [`I8Scalar`] — a single `int8` value.

use super::primitive::primitive_scalar;

primitive_scalar!(I8Scalar, I8Type, i8, "int8", 1);
