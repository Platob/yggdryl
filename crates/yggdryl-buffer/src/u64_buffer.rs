//! [`U64Buffer`] — a contiguous buffer of `u64` values.

use super::primitive::primitive_buffer;

primitive_buffer!(U64Buffer, u64, U64Field);
