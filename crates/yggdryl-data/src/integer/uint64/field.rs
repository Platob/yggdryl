//! The [`UInt64Field`] field of the [`UInt64`](super::UInt64) data type.

use super::UInt64;

crate::integer::int_field!(UInt64Field, UInt64, u64, "uint64");
