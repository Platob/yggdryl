//! [`U16Buffer`] — a contiguous buffer of `u16` values.

use super::primitive::primitive_buffer;

primitive_buffer!(U16Buffer, u16, U16Field);
