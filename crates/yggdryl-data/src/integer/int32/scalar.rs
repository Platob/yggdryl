//! The [`Int32`] scalar of the [`Int32Type`](super::Int32Type) data type.

use super::Int32Type;

crate::integer::int_scalar!(Int32, Int32Type, i32, "int32", Int32Array, Int32);
