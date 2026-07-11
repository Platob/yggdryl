//! [`U64Field`] — a named, nullable `uint64` field.

use super::primitive::primitive_field;

primitive_field!(U64Field, U64Type, u64, "uint64");
