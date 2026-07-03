//! The [`UInt64Field`] field of the [`UInt64Type`](super::UInt64Type) data type.

use super::UInt64Type;

crate::integer::int_field!(UInt64Field, UInt64Type, u64, "uint64", UInt64);
