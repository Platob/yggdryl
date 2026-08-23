//! Exact decimals, stored the way an exact decimal has to be stored.
//!
//! A decimal is a coefficient and a scale: the number is `unscaled * 10^-scale`.
//! That is the one representation that round-trips, because a decimal fraction
//! such as `0.1` has no finite binary expansion, so a float can only ever hold
//! the nearest double to it. It is also exactly how Arrow stores `Decimal32`
//! through `Decimal256`, so the value converts to a column without a decision.
//!
//! The scale is carried rather than normalized away, because the scale is data:
//! `1.50` and `1.5` are the same number written to different precision, and a
//! schema declaring scale 2 wants the first spelling back. Two decimals still
//! compare and hash by the *number* they name, so the two spellings are one
//! mapping key.
//!
//! ```
//! use yggdryl::Value;
//!
//! let price = Value::d128(1_050, 2); // 10.50
//!
//! assert_eq!(price.as_d128(), Some((1_050, 2)));
//! assert_eq!(price, Value::d128(105, 1));
//! assert_eq!(price.decimal_unscaled_at(4), Some(105_000));
//! ```

use std::cmp::Ordering;

use super::value::Value;
use crate::I256;

impl Value {
    /// Build an exact decimal from an unscaled integer and a scale.
    ///
    /// The value is `unscaled * 10^-scale`, so `Value::d128(1_050, 2)` is
    /// `10.50`. A negative scale multiplies instead, exactly as Arrow allows.
    pub const fn d128(unscaled: i128, scale: i8) -> Self {
        Self::D128(unscaled, scale)
    }

    /// Build an exact decimal with a 256-bit coefficient.
    pub const fn d256(unscaled: I256, scale: i8) -> Self {
        Self::D256(unscaled, scale)
    }

    /// Return the coefficient and scale when this is a 128-bit decimal.
    pub const fn as_d128(&self) -> Option<(i128, i8)> {
        match self {
            Self::D128(unscaled, scale) => Some((*unscaled, *scale)),
            _ => None,
        }
    }

    /// Return the coefficient and scale when this is a 256-bit decimal.
    pub const fn as_d256(&self) -> Option<(I256, i8)> {
        match self {
            Self::D256(unscaled, scale) => Some((*unscaled, *scale)),
            _ => None,
        }
    }

    /// Return whether this value is an exact decimal.
    pub const fn is_decimal(&self) -> bool {
        matches!(self, Self::D128(..) | Self::D256(..))
    }

    /// Return this decimal's unscaled integer at `scale`, when it is exact.
    ///
    /// A column declares its own scale, so a decimal written into one has to be
    /// restated at that scale. Restating to fewer fractional digits would throw
    /// digits away, so it answers `None` rather than rounding.
    pub fn decimal_unscaled_at(&self, scale: i8) -> Option<i128> {
        let (unscaled, current) = match self {
            Self::D128(unscaled, scale) => (*unscaled, *scale),
            Self::D256(unscaled, scale) => (unscaled.as_i128()?, *scale),
            _ => return None,
        };
        let shift = i32::from(scale) - i32::from(current);
        match shift.cmp(&0) {
            Ordering::Equal => Some(unscaled),
            Ordering::Greater => scale_up_signed(unscaled, shift),
            // Removing digits is only exact when every digit removed is a zero.
            Ordering::Less => {
                let divisor = scale_up_signed(1, -shift)?;
                (unscaled % divisor == 0).then(|| unscaled / divisor)
            }
        }
    }

    /// Return this decimal's 256-bit coefficient at `scale`, when exact.
    pub fn decimal256_unscaled_at(&self, scale: i8) -> Option<I256> {
        let (unscaled, current) = match self {
            Self::D128(unscaled, current) => (I256::from_i128(*unscaled), *current),
            Self::D256(unscaled, current) => (*unscaled, *current),
            _ => return None,
        };
        let shift = i32::from(scale) - i32::from(current);
        match shift.cmp(&0) {
            Ordering::Equal => Some(unscaled),
            Ordering::Greater => (0..shift).try_fold(unscaled, |held, _| held.checked_mul_ten()),
            Ordering::Less => (0..-shift).try_fold(unscaled, |held, _| held.divided_by_ten()),
        }
    }

    /// Render an exact decimal without passing through a float.
    pub fn as_decimal_utf8(&self) -> Option<String> {
        let (coefficient, scale) = match self {
            Self::D128(coefficient, scale) => (I256::from_i128(*coefficient), *scale),
            Self::D256(coefficient, scale) => (*coefficient, *scale),
            _ => return None,
        };
        Some(decimal_text(coefficient, scale))
    }
}

/// Render a coefficient and scale in ordinary decimal notation.
pub(crate) fn decimal_text(coefficient: I256, scale: i8) -> String {
    let encoded = coefficient.to_string();
    if scale == 0 {
        return encoded;
    }
    let (sign, digits) = encoded
        .strip_prefix('-')
        .map_or(("", encoded.as_str()), |digits| ("-", digits));
    if scale < 0 {
        return format!(
            "{sign}{digits}{}",
            "0".repeat(usize::from(scale.unsigned_abs()))
        );
    }
    let scale = usize::from(scale.unsigned_abs());
    if digits.len() > scale {
        let split = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..split], &digits[split..])
    } else {
        format!("{sign}0.{}{digits}", "0".repeat(scale - digits.len()))
    }
}

/// Strip the trailing zeros a decimal's coefficient carries.
///
/// Equal numbers share exactly one normal form, which is what lets `Hash` agree
/// with the numeric `Ord` below without either of them widening the coefficient.
pub(super) fn normalize(unscaled: I256, scale: i8) -> (I256, i8) {
    if unscaled.is_zero() {
        return (I256::ZERO, 0);
    }
    let mut unscaled = unscaled;
    let mut scale = scale;
    while scale > i8::MIN {
        let Some(reduced) = unscaled.divided_by_ten() else {
            break;
        };
        unscaled = reduced;
        scale -= 1;
    }
    (unscaled, scale)
}

/// Compare two decimals by the number each one names.
pub(super) fn compare(
    left_unscaled: I256,
    left_scale: i8,
    right_unscaled: I256,
    right_scale: i8,
) -> Ordering {
    let (left_unscaled, left_scale) = normalize(left_unscaled, left_scale);
    let (right_unscaled, right_scale) = normalize(right_unscaled, right_scale);
    if left_scale == right_scale {
        return left_unscaled.cmp(&right_unscaled);
    }
    let sign = left_unscaled
        .cmp(&I256::ZERO)
        .cmp(&right_unscaled.cmp(&I256::ZERO));
    if sign != Ordering::Equal {
        return sign;
    }

    // Same sign, different scale: bring the coefficient with fewer fractional
    // digits up to the other's scale and compare magnitudes. A product that no
    // longer fits is by that fact the larger magnitude, because the coefficient
    // it is compared against did fit.
    if left_scale < right_scale {
        scale_up(
            left_unscaled,
            i32::from(right_scale) - i32::from(left_scale),
        )
        .map_or_else(
            || {
                if left_unscaled.is_negative() {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            },
            |left| left.cmp(&right_unscaled),
        )
    } else {
        scale_up(
            right_unscaled,
            i32::from(left_scale) - i32::from(right_scale),
        )
        .map_or_else(
            || {
                if right_unscaled.is_negative() {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            },
            |right| left_unscaled.cmp(&right),
        )
    }
}

/// Multiply a magnitude by ten `digits` times, or report that it overflowed.
fn scale_up(unscaled: I256, digits: i32) -> Option<I256> {
    (0..digits).try_fold(unscaled, |held, _| held.checked_mul_ten())
}

/// Multiply a signed coefficient by ten `digits` times, or report the overflow.
fn scale_up_signed(unscaled: i128, digits: i32) -> Option<i128> {
    (0..digits).try_fold(unscaled, |unscaled, _| unscaled.checked_mul(10))
}

#[cfg(test)]
mod tests;
