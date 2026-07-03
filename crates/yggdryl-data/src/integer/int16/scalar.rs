//! The [`Int16`] scalar of the [`Int16Type`](super::Int16Type) data type.

use super::Int16Type;

crate::integer::int_scalar!(Int16, Int16Type, i16, "int16", Int16Array, Int16);
