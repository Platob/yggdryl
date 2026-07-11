//! [`I8Buffer`] — a contiguous buffer of `i8` values.

use super::primitive::primitive_buffer;

primitive_buffer!(I8Buffer, i8, I8Field);
