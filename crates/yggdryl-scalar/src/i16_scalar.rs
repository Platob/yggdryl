//! [`I16Scalar`] — a single `int16` value.

use super::primitive::primitive_scalar;

primitive_scalar!(I16Scalar, I16Type, i16, "int16", 1);
