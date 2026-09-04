//! Temporal scalar validation.

use smol_str::format_smolstr;

use crate::types::value::{ValidationFailure, expected};
use crate::{Scalar, TimeUnit};

pub(crate) fn validate_date64(value: &Scalar) -> std::result::Result<(), ValidationFailure> {
    const MILLIS_PER_DAY: i128 = 86_400_000;
    let Some(number) = value.as_i128() else {
        return Err(expected("date64 whole-day milliseconds", value));
    };
    if i64::try_from(number).is_err() || number % MILLIS_PER_DAY != 0 {
        return Err(ValidationFailure::new(
            "date64 must be signed 64-bit whole-day milliseconds",
        ));
    }
    Ok(())
}

pub(crate) fn validate_time(
    value: &Scalar,
    unit: TimeUnit,
) -> std::result::Result<(), ValidationFailure> {
    let maximum = match unit {
        TimeUnit::Second => 86_400_i128,
        TimeUnit::Millisecond => 86_400_000_i128,
        TimeUnit::Microsecond => 86_400_000_000_i128,
        TimeUnit::Nanosecond => 86_400_000_000_000_i128,
        _ => return Err(ValidationFailure::new("invalid time-of-day unit")),
    };
    let Some(number) = value.as_i128() else {
        return Err(expected("time-of-day integer", value));
    };
    if !(0..maximum).contains(&number) {
        return Err(ValidationFailure::new(format_smolstr!(
            "time-of-day value must be in 0..{maximum} for {unit}"
        )));
    }
    Ok(())
}
