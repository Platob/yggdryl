//! [`I64Scalar`] — a single `int64` value.

use super::primitive::primitive_scalar;

primitive_scalar!(I64Scalar, I64Type, i64, "int64", 1);
