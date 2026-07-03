//! The [`Int8`] scalar of the [`Int8Type`](super::Int8Type) data type.

use super::Int8Type;

crate::integer::int_scalar!(Int8, Int8Type, i8, "int8", Int8Array, Int8);
