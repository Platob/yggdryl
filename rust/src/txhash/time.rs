//! Unix counts: one resolution restated as another, an instant read out of a
//! value, and the clock read once.
//!
//! A unix count here is always UTC. A zoned datetime already stores the
//! instant relative to the epoch, and a naive one is read as the wall clock
//! expressed as if it were UTC, which is the convention every naive timestamp
//! in the project already uses - so neither ever moves the count, and the
//! only question an intake answers is the resolution.

use std::time::{SystemTime, UNIX_EPOCH};

use smol_str::format_smolstr;

use crate::{DataType, Error, Result, Scalar, TemporalFamily, TimeUnit, Timezone};

/// The resolution a unix count carries when a caller names none.
///
/// Microseconds are what a Python `datetime`, a Parquet `TIMESTAMP(MICROS)`
/// and an Arrow `Timestamp(Microsecond)` all hold exactly, and a 64-bit
/// microsecond count reaches the year 294,000 either side of the epoch, so
/// the default neither loses a wall clock nor runs out.
pub const DEFAULT_UNIT: TimeUnit = TimeUnit::Microsecond;

/// The operation name an overflowing restatement is refused under.
const RESTATE: &str = "unix restatement";

/// Accept a unit a unix count can be stated in.
///
/// # Errors
///
/// Returns [`Error::InvalidDataType`] for a day count or an interval layout,
/// which are not clock resolutions.
pub(crate) fn validate_unit(unit: TimeUnit) -> Result<()> {
    if unit.is_arrow_time() {
        return Ok(());
    }
    Err(Error::InvalidDataType {
        kind: "TxHash",
        reason: format_smolstr!(
            "unix unit must be second, millisecond, microsecond, or nanosecond, got {unit}"
        ),
    })
}

/// Nanoseconds in one count of a fixed-length unit.
const fn nanoseconds_per(unit: TimeUnit) -> Option<i64> {
    match unit {
        TimeUnit::Day => Some(86_400_000_000_000),
        TimeUnit::Second => Some(1_000_000_000),
        TimeUnit::Millisecond => Some(1_000_000),
        TimeUnit::Microsecond => Some(1_000),
        TimeUnit::Nanosecond => Some(1),
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => None,
    }
}

/// Restate a count of `from` as a count of `into`.
///
/// A coarser source scales exactly and refuses to overflow; a finer source
/// floors toward negative infinity, so the answer is the bucket of `into` the
/// instant falls in and two instants keep their order across the
/// restatement. `from` may be the day count a date holds; `into` is always a
/// clock resolution.
///
/// ```
/// use yggdryl::{TimeUnit, txhash};
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(txhash::restate_unix(1_700_000_000, TimeUnit::Second, TimeUnit::Microsecond)?, 1_700_000_000_000_000);
/// assert_eq!(txhash::restate_unix(1_999, TimeUnit::Nanosecond, TimeUnit::Microsecond)?, 1);
/// // Flooring, not truncation: the instant before the epoch stays before it.
/// assert_eq!(txhash::restate_unix(-1, TimeUnit::Nanosecond, TimeUnit::Microsecond)?, -1);
/// assert_eq!(txhash::restate_unix(19_723, TimeUnit::Day, TimeUnit::Second)?, 1_704_067_200);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`Error::InvalidDataType`] when either unit is not one a unix
/// count can carry, and [`Error::ArithmeticOverflow`] when the scaled count
/// does not fit 64 bits.
pub fn restate_unix(count: i64, from: TimeUnit, into: TimeUnit) -> Result<i64> {
    validate_unit(into)?;
    let Some(source) = nanoseconds_per(from) else {
        return Err(Error::InvalidDataType {
            kind: "TxHash",
            reason: format_smolstr!("an interval layout {from} is not a unix count"),
        });
    };
    let target = match nanoseconds_per(into) {
        Some(target) => target,
        None => unreachable!("a clock resolution has a fixed length"),
    };
    if source == target {
        return Ok(count);
    }
    if source > target {
        return count
            .checked_mul(source / target)
            .ok_or(Error::ArithmeticOverflow {
                operation: RESTATE,
                kind: "int64",
            });
    }
    Ok(count.div_euclid(target / source))
}

/// Read a value as a unix count of `unit`.
///
/// Every documented spelling of an instant is accepted, and each resolves to
/// one count: an integer is already the count; a datetime is its instant
/// restated, whatever its zone; a date is that day's midnight; text is read
/// as a timestamp, with an offset or without one, and floors from the finest
/// clock resolution. A time of day, a duration, an interval, and a null name
/// no instant and are refused.
///
/// ```
/// use yggdryl::{Scalar, TimeUnit, Timezone, txhash};
///
/// # fn main() -> yggdryl::Result<()> {
/// let unit = TimeUnit::Microsecond;
/// assert_eq!(txhash::unix_from_scalar(&Scalar::from(7_i64), unit)?, 7);
/// let instant = Scalar::from_datetime(1_700_000_000, TimeUnit::Second, Timezone::UTC)?;
/// assert_eq!(txhash::unix_from_scalar(&instant, unit)?, 1_700_000_000_000_000);
/// assert_eq!(txhash::unix_from_scalar(&Scalar::date32(0), unit)?, 0);
/// assert_eq!(
///     txhash::unix_from_scalar(&Scalar::from("1970-01-01T00:00:01Z"), unit)?,
///     1_000_000,
/// );
/// assert!(txhash::unix_from_scalar(&Scalar::Null, unit).is_err());
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error naming the kind that is not an instant, the text that
/// does not read as one, or the count that does not fit.
pub fn unix_from_scalar(value: &Scalar, unit: TimeUnit) -> Result<i64> {
    validate_unit(unit)?;
    if value.is_integer() {
        return value.as_i64().ok_or(Error::ArithmeticOverflow {
            operation: RESTATE,
            kind: "int64",
        });
    }
    if let Some(temporal) = value.as_temporal() {
        return match temporal.family() {
            TemporalFamily::DateTime | TemporalFamily::Date => {
                restate_unix(temporal.count(), temporal.unit(), unit)
            }
            TemporalFamily::Time | TemporalFamily::Duration | TemporalFamily::Interval => {
                Err(not_an_instant(temporal.family().as_str()))
            }
        };
    }
    if let Some(text) = value.as_str() {
        return unix_from_text(text, unit);
    }
    Err(not_an_instant(value.kind()))
}

/// Read timestamp text, with an offset or without one, as a count of `unit`.
fn unix_from_text(text: &str, unit: TimeUnit) -> Result<i64> {
    // The finest resolution first, so a spelling with sub-microsecond digits
    // floors like every other intake rather than being refused as inexact.
    let finest = TimeUnit::Nanosecond;
    let zoned = DataType::DateTime64 {
        unit: finest,
        timezone: Timezone::UTC,
    };
    let naive = DataType::DateTime64 {
        unit: finest,
        timezone: Timezone::NAIVE,
    };
    // A spelling carries an offset or it does not, so exactly one of the two
    // readings can succeed and trying both picks nothing.
    let parsed = Scalar::from_temporal_text(&zoned, text)
        .or_else(|zoned_error| Scalar::from_temporal_text(&naive, text).map_err(|_| zoned_error))?;
    let temporal = match parsed.as_temporal() {
        Some(temporal) => temporal,
        None => unreachable!("timestamp text reads as a datetime"),
    };
    restate_unix(temporal.count(), temporal.unit(), unit)
}

fn not_an_instant(kind: &str) -> Error {
    Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$"),
        reason: format_smolstr!(
            "expected an instant - an integer unix count, a datetime, a date, or timestamp text - got {kind}"
        ),
    }
}

/// Read the system clock as a unix count of `unit`.
///
/// ```
/// use yggdryl::{TimeUnit, txhash};
///
/// # fn main() -> yggdryl::Result<()> {
/// let micros = txhash::unix_now(TimeUnit::Microsecond)?;
/// let seconds = txhash::unix_now(TimeUnit::Second)?;
/// assert!(micros / 1_000_000 >= seconds);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`Error::InvalidDataType`] for a unit that is not a clock
/// resolution, and [`Error::ArithmeticOverflow`] when the clock reads before
/// the epoch or past what 64 bits of `unit` count.
pub fn unix_now(unit: TimeUnit) -> Result<i64> {
    validate_unit(unit)?;
    let elapsed =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::ArithmeticOverflow {
                operation: "clock read",
                kind: "int64",
            })?;
    let count = match unit {
        TimeUnit::Second => u128::from(elapsed.as_secs()),
        TimeUnit::Millisecond => elapsed.as_millis(),
        TimeUnit::Microsecond => elapsed.as_micros(),
        TimeUnit::Nanosecond => elapsed.as_nanos(),
        _ => unreachable!("the unit was validated first"),
    };
    i64::try_from(count).map_err(|_| Error::ArithmeticOverflow {
        operation: "clock read",
        kind: "int64",
    })
}
