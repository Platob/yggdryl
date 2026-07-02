//! The [`Int8Scalar`] scalar of the [`Int8`](super::Int8) data type.

use super::Int8;

crate::integer::int_scalar!(Int8Scalar, Int8, i8, "int8", Int8Array);
