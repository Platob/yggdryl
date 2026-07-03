//! The [`Int8Field`] field of the [`Int8Type`](super::Int8Type) data type.

use super::Int8Type;

crate::integer::int_field!(Int8Field, Int8Type, i8, "int8", Int8);
