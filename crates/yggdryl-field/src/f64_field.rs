//! [`F64Field`] — a named, nullable `float64` field.

use super::primitive::primitive_field;

primitive_field!(F64Field, F64Type, f64, "float64");
