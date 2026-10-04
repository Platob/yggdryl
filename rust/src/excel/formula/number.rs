//! Numeric boundaries shared by the Excel formula evaluator.
//!
//! Arithmetic retains its binary result; it is not rounded after each cell
//! in a SUM. Comparison normalizes through the existing format decimal on
//! the stack. ROUND uses that same decimal shape; cancellation is separate.

use super::super::cell::ExcelError;
use super::super::format::Digits;
use super::functions::Function;
use crate::{Arithmetic, Float64};
use std::cmp::Ordering;

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

/// INT's Excel decimal intake precedes the shared native floor kernel.
/// A raw f64 just below an integer can round to that integer at fifteen
/// significant digits, unlike direct binary64 floor.
pub(crate) fn integer_floor(value: f64) -> Option<Result<f64, ExcelError>> {
    if value.is_subnormal() { return None; }
    let value = match finite(value) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let rounded = Digits::from_f64(value).as_f64()?;
    if !rounded.is_finite() || rounded.is_subnormal() { return None; }
    let rounded = if value.is_sign_negative() { -rounded } else { rounded };
    let result = crate::Scalar::from(rounded).checked_floor();
    Some(match result {
        Ok(value) => finite(value.as_f64().expect("the shared floor preserves float64")),
        Err(_) => Err(ExcelError::Num),
    })
}

/// Excel TRUNC over the shared fifteen-digit decimal. `None` holds an
/// unresolved nonfinite decimal-place argument or subnormal input.
pub(crate) fn truncate(value: f64, places: f64) -> Option<Result<f64, ExcelError>> {
    if !places.is_finite() || value.is_subnormal() { return None; }
    let value = match finite(value) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    if places == 0.0 {
        let rounded = Digits::from_f64(value).as_f64()?;
        if !rounded.is_finite() || rounded.is_subnormal() { return None; }
        let rounded = if value.is_sign_negative() { -rounded } else { rounded };
        let scalar = crate::Scalar::from(rounded).checked_trunc();
        return Some(match scalar {
            Ok(value) => finite(value.as_f64().expect("the shared truncation preserves float64")),
            Err(_) => Err(ExcelError::Num),
        });
    }
    let mut decimal = Digits::from_f64(value);
    decimal.truncate(places.trunc() as i32);
    let truncated = decimal.as_f64()?;
    if !truncated.is_finite() || truncated.is_subnormal() { return None; }
    Some(finite(if value.is_sign_negative() { -truncated } else { truncated }))
}

/// ROUNDUP/ROUNDDOWN use the same 15-digit decimal as ROUND, with an
/// away/toward-zero digit decision. A subnormal input/output is held until
/// the raw-result versus saved-cache boundary is representable.
pub(crate) fn round_direction(value: f64, places: f64, up: bool) -> Option<Result<f64, ExcelError>> {
    if !places.is_finite() || value.is_subnormal() { return None; }
    let value = match finite(value) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let mut decimal = Digits::from_f64(value);
    if up { decimal.round_away(places.trunc() as i32); }
    else { decimal.truncate(places.trunc() as i32); }
    let result = decimal.as_f64()?;
    if result.is_subnormal() { return None; }
    Some(finite(if value.is_sign_negative() { -result } else { result }))
}

/// The existing generic floor owner operates on a bound binary Float64.
fn floor_native(value: f64) -> f64 {
    crate::Scalar::from(value).checked_floor()
        .expect("the rounding family binds Float64")
        .as_f64().expect("the shared floor preserves Float64")
}

fn ceil_native(value: f64) -> f64 { -floor_native(-value) }

/// EVEN/ODD first inherit the same 15-digit value policy as INT/TRUNC.
pub(crate) fn parity_round(value: f64, odd: bool) -> Option<Result<f64, ExcelError>> {
    if value.is_subnormal() { return None; }
    let value = match finite(value) {
        Ok(value) => value,
        Err(error) => return Some(Err(error)),
    };
    let magnitude = Digits::from_f64(value).as_f64()?;
    let rounded = if odd {
        if magnitude == 0.0 { 1.0 }
        else { 2.0 * ceil_native((magnitude - 1.0) / 2.0) + 1.0 }
    } else {
        2.0 * ceil_native(magnitude / 2.0)
    };
    let result = if value.is_sign_negative() && rounded != 0.0 { -rounded } else { rounded };
    Some(finite(result))
}

/// QUOTIENT keeps raw binary division before truncation; it deliberately
/// differs from CEILING/FLOOR's normalized quotient.
pub(crate) fn quotient(left: f64, right: f64) -> Option<Result<f64, ExcelError>> {
    if right == 0.0 { return Some(Err(ExcelError::Div0)); }
    if left.is_subnormal() || right.is_subnormal() { return None; }
    let (left, right) = match (finite(left), finite(right)) {
        (Ok(left), Ok(right)) => (left, right),
        (Err(error), _) | (_, Err(error)) => return Some(Err(error)),
    };
    let result = Arithmetic::Div.apply_float(left, right);
    if !result.is_finite() { return Some(Err(ExcelError::Num)); }
    let result = crate::Scalar::from(result).checked_trunc()
        .expect("QUOTIENT binds Float64")
        .as_f64().expect("the shared truncation preserves Float64");
    Some(finite(result))
}

/// Legacy multiple rounding and MATH mode share one binary multiple owner.
/// The observed legacy CEILING/FLOOR quotient is normalized to 15 significant
/// digits; MROUND must keep its raw quotient across the half boundary.
pub(crate) fn multiple(
    function: Function, value: f64, significance: f64, mode: f64,
) -> Option<Result<f64, ExcelError>> {
    if value.is_subnormal() || significance.is_subnormal() || !mode.is_finite() { return None; }
    let (value, mut significance) = match (finite(value), finite(significance)) {
        (Ok(value), Ok(significance)) => (value, significance),
        (Err(error), _) | (_, Err(error)) => return Some(Err(error)),
    };
    let math = matches!(function, Function::CeilingDotMath | Function::FloorDotMath);
    if math { significance = significance.abs(); }
    if value == 0.0 { return Some(Ok(0.0)); }
    if significance == 0.0 {
        return Some(if function == Function::Floor {
            Err(ExcelError::Div0)
        } else { Ok(0.0) });
    }
    if function == Function::Mround && value.is_sign_negative() != significance.is_sign_negative() {
        return Some(Err(ExcelError::Num));
    }
    if matches!(function, Function::Ceiling | Function::Floor)
        && value > 0.0 && significance < 0.0 {
        return Some(Err(ExcelError::Num));
    }
    let raw = Arithmetic::Div.apply_float(value, significance);
    if !raw.is_finite() { return None; }
    if raw.is_subnormal() { return Some(Ok(0.0)); }
    let multiple = if function == Function::Mround {
        let magnitude = raw.abs();
        let whole = floor_native(magnitude);
        let rounded = whole + if magnitude - whole >= 0.5 { 1.0 } else { 0.0 };
        if raw.is_sign_negative() { -rounded } else { rounded }
    } else {
        let quotient = Digits::from_f64(raw).as_f64()?;
        let quotient = if raw.is_sign_negative() { -quotient } else { quotient };
        let reverse = math && value < 0.0 && mode != 0.0;
        if matches!(function, Function::Ceiling | Function::CeilingDotMath) != reverse {
            ceil_native(quotient)
        } else {
            floor_native(quotient)
        }
    };
    let result = Arithmetic::Mul.apply_float(multiple, significance);
    if result.is_subnormal() { return None; }
    Some(finite(result))
}

/// Excel information parity: truncate toward zero and use the shared
/// binary remainder kernel. An unsettled subnormal operand retains its cache.
pub(crate) fn odd(value: f64) -> Option<Result<bool, ExcelError>> {
    if value.is_subnormal() {
        return None;
    }
    Some(finite(value).map(|number| Arithmetic::Rem.apply_float(number.trunc(), 2.0) != 0.0))
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

/// Numeric ordering shares equality's decimal boundary; unequal finite
/// values use the generic native float order without a decimal reparse.
pub(crate) fn compare(left: f64, right: f64) -> Result<Ordering, ExcelError> {
    if equal(left, right)? {
        Ok(Ordering::Equal)
    } else {
        Ok(Float64::from_f64(left).cmp(&Float64::from_f64(right)))
    }
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


/// Excel annuities share the generic compound factors; this adapter alone
/// assigns spreadsheet domain errors and normalizes the published result.
pub(crate) fn annuity(function: Function, values: [f64; 5]) -> Option<Result<f64, ExcelError>> {
    if values.iter().any(|value| value.is_subnormal()) { return None; }
    if values.iter().any(|value| !value.is_finite()) { return Some(Err(ExcelError::Num)); }
    let [rate, periods, payment, capital, due] = values;
    if function == Function::Pmt {
        if rate <= -1.0 || periods == 0.0 { return Some(Err(ExcelError::Num)); }
        // Nonzero-rate PMT needs a separately proved discount-rounding order.
        // Zero rate is the exact algebraic limit, with one final division.
        if rate != 0.0 { return None; }
        let total = Arithmetic::Add.apply_float(payment, capital);
        let result = Arithmetic::Div.apply_float(-total, periods);
        if result.is_subnormal() { return None; }
        return Some(finite(result));
    }
    let (growth, payments) = Arithmetic::annuity_factors(rate, periods, due != 0.0);
    if !growth.is_finite() || !payments.is_finite() { return Some(Err(ExcelError::Num)); }
    let result = match function {
        Function::Pv if growth == 0.0 => return Some(Err(ExcelError::Div0)),
        Function::Pv => (-capital - payment * payments) / growth,
        Function::Fv => -capital * growth - payment * payments,
        _ => unreachable!("the annuity call binds PV or FV"),
    };
    if result.is_subnormal() { return None; }
    Some(finite(result))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! The primitive numeric boundaries pinned before evaluator integration.

    use crate::excel::ExcelError;

    /// Apply the Excel annuity boundary after native numeric intake.
    pub fn annuity(future: bool, values: [f64; 5]) -> Option<Result<f64, ExcelError>> {
        super::annuity(if future { super::Function::Fv } else { super::Function::Pv }, values)
    }

    /// Exercise PMT's proven domain without constructing formula operands.
    pub fn payment(values: [f64; 5]) -> Option<Result<f64, ExcelError>> {
        super::annuity(super::Function::Pmt, values)
    }

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

    /// Order through Excel equality followed by the shared native float order.
    pub fn compare(left: f64, right: f64) -> Result<std::cmp::Ordering, ExcelError> {
        super::compare(left, right)
    }

    /// Round away/toward zero at a decimal place, if calibrated.
    pub fn round_direction(value: f64, places: f64, up: bool) -> Option<Result<f64, ExcelError>> {
        super::round_direction(value, places, up)
    }

    /// Advance to the next parity integer away from zero.
    pub fn parity_round(value: f64, odd: bool) -> Option<Result<f64, ExcelError>> {
        super::parity_round(value, odd)
    }

    /// Truncate the raw binary quotient toward zero.
    pub fn quotient(left: f64, right: f64) -> Option<Result<f64, ExcelError>> {
        super::quotient(left, right)
    }

    /// Round a native number and decimal-place argument, if calibrated.
    pub fn round(value: f64, places: f64) -> Option<Result<f64, ExcelError>> {
        super::round(value, places)
    }

    /// Excel INT after fifteen-digit operand intake.
    pub fn integer_floor(value: f64) -> Option<Result<f64, ExcelError>> {
        super::integer_floor(value)
    }

    /// Excel TRUNC with signed decimal-place intake.
    pub fn truncate(value: f64, places: f64) -> Option<Result<f64, ExcelError>> {
        super::truncate(value, places)
    }
}
