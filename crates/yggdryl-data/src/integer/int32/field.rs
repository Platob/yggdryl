//! The [`Int32Field`] field of the [`Int32Type`](super::Int32Type) data type.

use super::Int32Type;

crate::integer::int_field!(Int32Field, Int32Type, i32, "int32", Int32);
