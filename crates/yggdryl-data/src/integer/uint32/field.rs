//! The [`UInt32Field`] field of the [`UInt32Type`](super::UInt32Type) data type.

use super::UInt32Type;

crate::integer::int_field!(UInt32Field, UInt32Type, u32, "uint32", UInt32);
