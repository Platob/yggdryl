//! [`U32Scalar`] — a single, possibly-null `uint32` value.

use super::primitive::primitive_scalar;

primitive_scalar!(U32Scalar, U32Type, u32, "uint32", 1);
