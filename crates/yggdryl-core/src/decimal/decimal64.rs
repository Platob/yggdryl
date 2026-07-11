//! [`Decimal64`] — a 64-bit fixed-width decimal (`i64` mantissa + scale).

use super::primitive::decimal_primitive;
use crate::Decimal32;

decimal_primitive!(Decimal64, i64, 64, 8);

/// Widens a [`Decimal32`] to a `Decimal64` (same scale; always exact).
impl From<Decimal32> for Decimal64 {
    fn from(value: Decimal32) -> Self {
        Self::new(value.mantissa() as i64, value.scale())
    }
}
