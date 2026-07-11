//! [`U16Field`] — a named, nullable `uint16` field.

use super::primitive::primitive_field;

primitive_field!(U16Field, U16Type, u16, "uint16");
