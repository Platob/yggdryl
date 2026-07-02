//! The [`Int16Scalar`] scalar of the [`Int16`](super::Int16) data type.

use super::Int16;

crate::integer::int_scalar!(Int16Scalar, Int16, i16, "int16", Int16Array);
