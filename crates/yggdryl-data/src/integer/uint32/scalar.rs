//! The [`UInt32Scalar`] scalar of the [`UInt32`](super::UInt32) data type.

use super::UInt32;

crate::integer::int_scalar!(UInt32Scalar, UInt32, u32, "uint32", UInt32Array);
