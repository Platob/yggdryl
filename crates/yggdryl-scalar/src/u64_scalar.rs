//! [`U64Scalar`] — a single, possibly-null `uint64` value.

use super::primitive::primitive_scalar;

primitive_scalar!(U64Scalar, U64Type, u64, "uint64", 1);
