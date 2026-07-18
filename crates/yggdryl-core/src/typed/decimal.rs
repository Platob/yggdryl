//! [`Decimal`] — the shared trait of the **fixed-point decimal** element types (`Decimal32` …
//! `Decimal256`).
//!
//! A decimal is a signed **unscaled integer** (`i32` / `i64` / `i128` / [`I256`](super::fixedbyte::I256))
//! plus a **scale** (decimal places) and **precision** (max significant digits) carried in the
//! [`Field`](super::Field) metadata — the value is `unscaled × 10^-scale`. `Decimal` extends
//! [`DataType`](super::DataType) with the width's max precision and a scale-aware
//! [`format`](Decimal::format) for easy interoperability (unscaled `12345`, scale `2` → `"123.45"`).

use super::DataType;

/// The shared surface of a fixed-point decimal type. `MAX_PRECISION` is the most significant base-10
/// digits the backing integer holds; [`format`](Decimal::format) renders an unscaled value at a
/// given scale as a decimal string.
pub trait Decimal: DataType
where
    Self::Native: core::fmt::Display,
{
    /// The maximum significant base-10 digits the backing width holds — `Decimal32` → 9,
    /// `Decimal64` → 18, `Decimal128` → 38, `Decimal256` → 76.
    const MAX_PRECISION: u32;

    /// Renders the unscaled `value` at `scale` decimal places as a decimal string.
    ///
    /// ```
    /// use yggdryl_core::typed::Decimal;
    /// use yggdryl_core::typed::fixedbyte::Decimal128;
    ///
    /// assert_eq!(Decimal128::format(12345, 2), "123.45");
    /// assert_eq!(Decimal128::format(5, 2), "0.05");
    /// assert_eq!(Decimal128::format(-5, 2), "-0.05");
    /// assert_eq!(Decimal128::format(12345, 0), "12345");
    /// ```
    fn format(value: Self::Native, scale: i32) -> String {
        apply_scale(&value.to_string(), scale)
    }
}

/// Places the decimal point in the `unscaled` digit string `scale` places from the right — the
/// shared render behind [`Decimal::format`]. A non-positive `scale` appends `|scale|` zeros (or
/// returns the integer as-is at scale `0`).
pub fn apply_scale(unscaled: &str, scale: i32) -> String {
    if scale <= 0 {
        let mut out = String::from(unscaled);
        for _ in 0..(-scale) {
            out.push('0');
        }
        return out;
    }
    let scale = scale as usize;
    let (sign, digits) = match unscaled.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", unscaled),
    };
    if digits.len() > scale {
        let point = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..point], &digits[point..])
    } else {
        let zeros = "0".repeat(scale - digits.len());
        format!("{sign}0.{zeros}{digits}")
    }
}
