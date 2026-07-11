//! [`I16Buffer`] — a contiguous buffer of `i16` values.

use super::primitive::primitive_buffer;

primitive_buffer!(I16Buffer, i16, I16Field);
