//! The [`Int64Field`] field of the [`Int64Type`](super::Int64Type) data type.

use super::Int64Type;

crate::integer::int_field!(Int64Field, Int64Type, i64, "int64", Int64);
