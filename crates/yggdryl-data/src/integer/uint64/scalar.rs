//! The [`UInt64Scalar`] scalar of the [`UInt64`](super::UInt64) data type.

use super::UInt64;

crate::integer::int_scalar!(UInt64Scalar, UInt64, u64, "uint64");
