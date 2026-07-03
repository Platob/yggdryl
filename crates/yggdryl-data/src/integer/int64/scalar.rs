//! The [`Int64`] scalar of the [`Int64Type`](super::Int64Type) data type.

use super::Int64Type;

crate::integer::int_scalar!(Int64, Int64Type, i64, "int64", Int64Array, Int64);
