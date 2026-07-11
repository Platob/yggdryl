//! [`I8Field`] — a named, nullable `int8` field.

use super::primitive::primitive_field;

primitive_field!(I8Field, I8Type, i8, "int8");
