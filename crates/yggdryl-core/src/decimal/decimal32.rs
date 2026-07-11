//! [`Decimal32`] — a 32-bit fixed-width decimal (`i32` mantissa + scale).

use super::primitive::decimal_primitive;

decimal_primitive!(Decimal32, i32, 32, 4);
