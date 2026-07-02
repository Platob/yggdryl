//! The [`Int32Scalar`] scalar of the [`Int32`](super::Int32) data type.

use super::Int32;

crate::integer::int_scalar!(Int32Scalar, Int32, i32, "int32", Int32Array);
