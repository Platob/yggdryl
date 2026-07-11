//! [`U16Scalar`] — a single, possibly-null `uint16` value.

use super::primitive::primitive_scalar;

primitive_scalar!(U16Scalar, U16Type, u16, "uint16", 1);
