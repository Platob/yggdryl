//! Decimal scalar validation.

use smol_str::format_smolstr;

use crate::Scalar;
use crate::types::value::{ValidationFailure, expected};

pub(crate) fn validate_decimal_value(
    value: &Scalar,
    precision: u8,
    width: u16,
) -> std::result::Result<(), ValidationFailure> {
    let Some(integer) = value.as_i128() else {
        return Err(expected("unscaled decimal integer", value));
    };
    let fits_width = match width {
        32 => i32::try_from(integer).is_ok(),
        64 => i64::try_from(integer).is_ok(),
        _ => true,
    };
    if !fits_width || decimal_digits(integer.unsigned_abs()) > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal value exceeds precision {precision} or physical width {width}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_decimal256_value(
    value: &Scalar,
    precision: u8,
    scale: i8,
) -> std::result::Result<(), ValidationFailure> {
    let Some(coefficient) = (if value.is_decimal() {
        value.decimal256_unscaled_at(scale)
    } else {
        value.as_i128().map(crate::I256::from_i128)
    }) else {
        return Err(expected("d256", value));
    };
    let encoded = coefficient.to_string();
    let digits = encoded.trim_start_matches('-');
    if digits.len() > usize::from(precision) {
        return Err(ValidationFailure::new(format_smolstr!(
            "decimal256 value exceeds precision {precision}"
        )));
    }
    Ok(())
}

fn decimal_digits(value: u128) -> usize {
    let mut digits = 1;
    let mut remaining = value / 10;
    while remaining > 0 {
        digits += 1;
        remaining /= 10;
    }
    digits
}
