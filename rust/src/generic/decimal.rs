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
//! let price = Value::decimal(1_050, 2); // 10.50
//!
//! assert_eq!(price.as_decimal(), Some((1_050, 2)));
//! assert_eq!(price, Value::decimal(105, 1)); // the same number, less precision
//! assert_eq!(price.decimal_unscaled_at(4), Some(105_000));
//! ```

use std::cmp::Ordering;

use super::value::Value;

impl Value {
    /// Build an exact decimal from an unscaled integer and a scale.
    ///
    /// The value is `unscaled * 10^-scale`, so `Value::decimal(1_050, 2)` is
    /// `10.50`. A negative scale multiplies instead, exactly as Arrow allows.
    pub const fn decimal(unscaled: i128, scale: i8) -> Self {
        Self::Decimal(unscaled, scale)
    }

    /// Return the unscaled integer and scale when this is an exact decimal.
    pub const fn as_decimal(&self) -> Option<(i128, i8)> {
        match self {
            Self::Decimal(unscaled, scale) => Some((*unscaled, *scale)),
            _ => None,
        }
    }

    /// Return whether this value is an exact decimal.
    pub const fn is_decimal(&self) -> bool {
        matches!(self, Self::Decimal(..))
    }

    /// Return this decimal's unscaled integer at `scale`, when it is exact.
    ///
    /// A column declares its own scale, so a decimal written into one has to be
    /// restated at that scale. Restating to fewer fractional digits would throw
    /// digits away, so it answers `None` rather than rounding.
    pub fn decimal_unscaled_at(&self, scale: i8) -> Option<i128> {
        let (unscaled, current) = self.as_decimal()?;
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
}

/// Strip the trailing zeros a decimal's coefficient carries.
///
/// Equal numbers share exactly one normal form, which is what lets `Hash` agree
/// with the numeric `Ord` below without either of them widening the coefficient.
pub(super) const fn normalize(unscaled: i128, scale: i8) -> (i128, i8) {
    if unscaled == 0 {
        return (0, 0);
    }
    let mut unscaled = unscaled;
    let mut scale = scale;
    while unscaled % 10 == 0 && scale > i8::MIN {
        unscaled /= 10;
        scale -= 1;
    }
    (unscaled, scale)
}

/// Compare two decimals by the number each one names.
pub(super) fn compare(
    left_unscaled: i128,
    left_scale: i8,
    right_unscaled: i128,
    right_scale: i8,
) -> Ordering {
    let (left_unscaled, left_scale) = normalize(left_unscaled, left_scale);
    let (right_unscaled, right_scale) = normalize(right_unscaled, right_scale);
    if left_scale == right_scale {
        return left_unscaled.cmp(&right_unscaled);
    }
    let sign = left_unscaled.signum().cmp(&right_unscaled.signum());
    if sign != Ordering::Equal {
        return sign;
    }

    // Same sign, different scale: bring the coefficient with fewer fractional
    // digits up to the other's scale and compare magnitudes. A product that no
    // longer fits is by that fact the larger magnitude, because the coefficient
    // it is compared against did fit.
    let left_magnitude = left_unscaled.unsigned_abs();
    let right_magnitude = right_unscaled.unsigned_abs();
    let magnitudes = if left_scale < right_scale {
        scale_up(
            left_magnitude,
            i32::from(right_scale) - i32::from(left_scale),
        )
        .map_or(Ordering::Greater, |left| left.cmp(&right_magnitude))
    } else {
        scale_up(
            right_magnitude,
            i32::from(left_scale) - i32::from(right_scale),
        )
        .map_or(Ordering::Less, |right| left_magnitude.cmp(&right))
    };
    if left_unscaled.is_negative() {
        magnitudes.reverse()
    } else {
        magnitudes
    }
}

/// Multiply a magnitude by ten `digits` times, or report that it overflowed.
fn scale_up(magnitude: u128, digits: i32) -> Option<u128> {
    (0..digits).try_fold(magnitude, |magnitude, _| magnitude.checked_mul(10))
}

/// Multiply a signed coefficient by ten `digits` times, or report the overflow.
fn scale_up_signed(unscaled: i128, digits: i32) -> Option<i128> {
    (0..digits).try_fold(unscaled, |unscaled, _| unscaled.checked_mul(10))
}

#[cfg(test)]
mod tests;
