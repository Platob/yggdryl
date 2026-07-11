//! [`Decimal128`] — a 128-bit fixed-width decimal (`i128` mantissa + scale).

use super::primitive::decimal_primitive;
use crate::{Decimal32, Decimal64};

decimal_primitive!(Decimal128, i128, 128, 16);

/// Widens a [`Decimal32`] to a `Decimal128` (same scale; always exact).
impl From<Decimal32> for Decimal128 {
    fn from(value: Decimal32) -> Self {
        Self::new(value.mantissa() as i128, value.scale())
    }
}

/// Widens a [`Decimal64`] to a `Decimal128` (same scale; always exact).
impl From<Decimal64> for Decimal128 {
    fn from(value: Decimal64) -> Self {
        Self::new(value.mantissa() as i128, value.scale())
    }
}
