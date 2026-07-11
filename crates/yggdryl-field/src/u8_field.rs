//! [`U8Field`] — a named, nullable `uint8` field.

use super::primitive::primitive_field;

primitive_field!(U8Field, U8Type, u8, "uint8");
