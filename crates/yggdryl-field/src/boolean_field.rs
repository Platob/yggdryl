//! [`BooleanField`] — a named, nullable `boolean` field.

use super::primitive::primitive_field;

// A field never touches the value codec, so `boolean` uses the same macro as the
// numerics (unlike the dtype layer, where `BooleanType` is hand-written).
primitive_field!(BooleanField, BooleanType, bool, "boolean");
