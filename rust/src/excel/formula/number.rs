//! Numeric boundaries shared by the Excel formula evaluator.
//!
//! Arithmetic retains its binary result; it is not rounded after each cell
//! in a SUM. Comparison normalizes through the existing format decimal on
//! the stack. ROUND uses that same decimal shape; cancellation is separate.

use super::super::cell::ExcelError;
use super::super::format::Digits;
use crate::Arithmetic;

/// An authored number lexeme has fifteen truncated significant decimal
/// digits and a smaller domain than a computed binary result. A minus sign
/// is the parser's unary operator, never part of this proven lexeme.
pub(crate) fn literal(text: &str) -> Option<f64> {
    let value = Digits::from_decimal(text).as_f64()?;
    if value > 9.999_999_999_999_99e307 {
        return None;
    }
    finite(value).ok()
}

/// A representable Excel result: overflow/NaN is an error, subnormals and
/// signed zero are positive zero. Normal binary values remain unchanged.
pub(crate) fn finite(value: f64) -> Result<f64, ExcelError> {
    if !value.is_finite() {
        Err(ExcelError::Num)
    } else if value == 0.0 || value.is_subnormal() {
        Ok(0.0)
    } else {
        Ok(value)
    }
}

/// Add finite operands once. A subnormal operand or raw result has an
/// unsettled computed-value versus saved-cache representation (`None`).
/// A root opposite-sign residual below eight ULPs of the first operand is
/// Excel's corrected positive zero; nested normal results retain binary64.
pub(crate) fn add(left: f64, right: f64, root: bool) -> Option<Result<f64, ExcelError>> {
    if left.is_subnormal() || right.is_subnormal() {
        return None;
    }
    let left = match finite(left) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let right = match finite(right) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let raw = Arithmetic::Add.apply_float(left, right);
    if raw.is_subnormal() {
        return None;
    }
    let result = match finite(raw) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    if root
        && left != 0.0
        && right != 0.0
        && result != 0.0
        && left.is_sign_negative() != right.is_sign_negative()
    {
        // All three values are normal after finite; exponent extraction is
        // exact and avoids multiplying ULP(left) near f64::MAX.
        let first_exponent = ((left.to_bits() >> 52) & 0x7ff) as i32 - 1023;
        let result_exponent = ((result.to_bits() >> 52) & 0x7ff) as i32 - 1023;
        if first_exponent - result_exponent >= 50 {
            return Some(Ok(0.0));
        }
    }
    Some(Ok(result))
}

/// Excel MOD in the observed quotient domain. The transition between
/// 2^40 and 2^41 is mixed in native results, so it remains unsettled.
/// `None` holds the existing cache until that boundary and subnormal inputs
/// have a complete policy; the confirmed high quotient and subnormal
/// remainder are #NUM! errors.
pub(crate) fn modulus(left: f64, right: f64) -> Option<Result<f64, ExcelError>> {
    if right == 0.0 {
        return Some(Err(ExcelError::Div0));
    }
    if left.is_subnormal() || right.is_subnormal() {
        return None;
    }
    let left = match finite(left) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let right = match finite(right) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let quotient = Arithmetic::Div.apply_float(left, right).abs();
    if quotient >= 2_199_023_255_552.0 {
        return Some(Err(ExcelError::Num));
    }
    if quotient > 1_099_511_627_776.0 {
        return None;
    }
    let remainder = Arithmetic::Rem.apply_float(left, right);
    if remainder.is_subnormal() {
        return Some(Err(ExcelError::Num));
    }
    if remainder != 0.0 && remainder.is_sign_negative() != right.is_sign_negative() {
        add(remainder, right, false)
    } else {
        Some(finite(remainder))
    }
}

/// The design's fifteen-significant-digit numeric equality.
///
/// Comparing the normalized decimals themselves avoids overflowing a
/// finite f64::MAX merely by converting its rounded decimal back to binary.
pub(crate) fn equal(left: f64, right: f64) -> Result<bool, ExcelError> {
    let left = finite(left)?;
    let right = finite(right)?;
    if left == right {
        return Ok(true);
    }
    if left.is_sign_negative() != right.is_sign_negative() {
        return Ok(false);
    }
    Ok(Digits::from_f64(left) == Digits::from_f64(right))
}

/// Excel ROUND over the shared fifteen-digit decimal. `None` means a
/// nonfinite decimal-place argument has no native calibration yet.
pub(crate) fn round(value: f64, places: f64) -> Option<Result<f64, ExcelError>> {
    if !places.is_finite() {
        return None;
    }
    let value = match finite(value) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let mut decimal = Digits::from_f64(value);
    // Only decimal places near the at-most-fifteen active digits matter.
    // A saturating cast maps any larger finite magnitude to the same
    // no-op or zero branch of Digits::round.
    decimal.round(places.trunc() as i32);
    let rounded = decimal.as_f64()?;
    let signed = if value.is_sign_negative() {
        -rounded
    } else {
        rounded
    };
    Some(finite(signed))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! The primitive numeric boundaries pinned before evaluator integration.

    use crate::excel::ExcelError;

    /// Apply the finite, underflow and signed-zero result rule.
    pub fn finite(value: f64) -> Result<f64, ExcelError> {
        super::finite(value)
    }

    /// Add through the numeric primitive with expression-root context.
    pub fn add(left: f64, right: f64, root: bool) -> Option<Result<f64, ExcelError>> {
        super::add(left, right, root)
    }

    /// Apply the observed bounded MOD policy to already resolved numbers.
    pub fn modulus(left: f64, right: f64) -> Option<Result<f64, ExcelError>> {
        super::modulus(left, right)
    }

    /// Compare through the shared fifteen-digit decimal representation.
    pub fn equal(left: f64, right: f64) -> Result<bool, ExcelError> {
        super::equal(left, right)
    }

    /// Round a native number and decimal-place argument, if calibrated.
    pub fn round(value: f64, places: f64) -> Option<Result<f64, ExcelError>> {
        super::round(value, places)
    }
}
