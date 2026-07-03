//! The [`UInt8`] scalar of the [`UInt8Type`](super::UInt8Type) data type.

use super::UInt8Type;

crate::integer::int_scalar!(UInt8, UInt8Type, u8, "uint8", UInt8Array, UInt8);
