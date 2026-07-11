//! [`U8Scalar`] — a single, possibly-null `uint8` value.

use super::primitive::primitive_scalar;

primitive_scalar!(U8Scalar, U8Type, u8, "uint8", 1);
