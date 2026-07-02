//! The [`UInt16Scalar`] scalar of the [`UInt16`](super::UInt16) data type.

use super::UInt16;

crate::integer::int_scalar!(UInt16Scalar, UInt16, u16, "uint16", UInt16Array);
