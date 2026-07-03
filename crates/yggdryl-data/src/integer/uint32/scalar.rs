//! The [`UInt32`] scalar of the [`UInt32Type`](super::UInt32Type) data type.

use super::UInt32Type;

crate::integer::int_scalar!(UInt32, UInt32Type, u32, "uint32", UInt32Array, UInt32);
