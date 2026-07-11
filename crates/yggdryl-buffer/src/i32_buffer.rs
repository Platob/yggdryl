//! [`I32Buffer`] — a contiguous buffer of `i32` values.

use super::primitive::primitive_buffer;

primitive_buffer!(I32Buffer, i32, I32Field);
