//! The [`UInt16`] scalar of the [`UInt16Type`](super::UInt16Type) data type.

use super::UInt16Type;

crate::integer::int_scalar!(UInt16, UInt16Type, u16, "uint16", UInt16Array, UInt16);
