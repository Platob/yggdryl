//! [`I32Field`] — a named, nullable `int32` field.

use super::primitive::primitive_field;

primitive_field!(I32Field, I32Type, i32, "int32");
