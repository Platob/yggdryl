//! The [`UInt8Scalar`] scalar of the [`UInt8`](super::UInt8) data type.

use super::UInt8;

crate::integer::int_scalar!(UInt8Scalar, UInt8, u8, "uint8", UInt8Array);
