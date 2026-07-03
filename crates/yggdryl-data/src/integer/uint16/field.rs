//! The [`UInt16Field`] field of the [`UInt16Type`](super::UInt16Type) data type.

use super::UInt16Type;

crate::integer::int_field!(UInt16Field, UInt16Type, u16, "uint16", UInt16);
