//! [`U32Field`] — a named, nullable `uint32` field.

use super::primitive::primitive_field;

primitive_field!(U32Field, U32Type, u32, "uint32");
