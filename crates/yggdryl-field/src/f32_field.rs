//! [`F32Field`] — a named, nullable `float32` field.

use super::primitive::primitive_field;

primitive_field!(F32Field, F32Type, f32, "float32");
