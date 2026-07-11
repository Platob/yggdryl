//! [`BooleanScalar`] — a single, possibly-null `boolean` value.

use super::primitive::primitive_scalar;

// The value codec is delegated to `BooleanType`, so `boolean` uses the same macro as
// the numerics (its value encodes as one 0/1 byte).
primitive_scalar!(BooleanScalar, BooleanType, bool, "boolean", true);
