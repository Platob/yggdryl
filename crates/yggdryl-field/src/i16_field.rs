//! [`I16Field`] — a named, nullable `int16` field.

use super::primitive::primitive_field;

primitive_field!(I16Field, I16Type, i16, "int16");
