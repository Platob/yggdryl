//! The [`UInt64`] scalar of the [`UInt64Type`](super::UInt64Type) data type.

use super::UInt64Type;

crate::integer::int_scalar!(UInt64, UInt64Type, u64, "uint64", UInt64Array, UInt64);
