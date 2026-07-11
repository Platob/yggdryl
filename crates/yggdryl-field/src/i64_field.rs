//! [`I64Field`] — a named, nullable `int64` field.

use super::primitive::primitive_field;

primitive_field!(I64Field, I64Type, i64, "int64");
