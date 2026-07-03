//! The [`UInt8Field`] field of the [`UInt8Type`](super::UInt8Type) data type.

use super::UInt8Type;

crate::integer::int_field!(UInt8Field, UInt8Type, u8, "uint8", UInt8);
