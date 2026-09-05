//! Width-accurate temporal values with explicit units and zones.
//!
//! ```
//! use yggdryl::{Scalar, TemporalFamily, TimeUnit, Timezone};
//!
//! let day = Scalar::from_date(20_000, TimeUnit::Day, Timezone::NAIVE)?;
//! let at = Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC)?;
//! assert_eq!(day.as_date().map(|value| value.bit_width()), Some(32));
//! assert_eq!(at.as_datetime().map(|value| value.family()), Some(TemporalFamily::DateTime));
//! # Ok::<(), yggdryl::Error>(())
//! ```

use smol_str::format_smolstr;

use crate::types::arithmetic::{Arithmetic, invalid_binary};
use crate::types::ascii::iso;
use crate::types::decimal::scalars::exact_value_parts;
use crate::types::typed::define_scalar_type;
use crate::types::value::{ValidationFailure, expected};
use crate::{DataType, Error, I256, Result, Scalar, TimeUnit, Timezone};

/// Operations shared by every temporal representation.
pub trait TemporalValue: crate::ScalarValue {
    /// The semantic temporal family.
    const FAMILY: TemporalFamily;
    /// The physical count width in bits.
    const BIT_WIDTH: u8;

    /// Return the stored count widened to 64 bits.
    fn count(&self) -> i64;
    /// Return the count's unit.
    fn unit(&self) -> TimeUnit;
    /// Return the explicit timezone marker.
    fn timezone(&self) -> Timezone;
    /// Convert this value to another valid unit.
    fn with_unit(self, unit: TimeUnit) -> Result<Self>;
    /// Restate this value with another valid timezone marker.
    fn with_timezone(self, timezone: Timezone) -> Result<Self>;
}

define_scalar_type!(DateTime64Scalar, super::DateTime64Type, "datetime64");
define_scalar_type!(
    Date32Scalar,
    super::Date32,
    "date32",
    crate::DataType::Date32
);
define_scalar_type!(
    Date64Scalar,
    super::Date64,
    "date64",
    crate::DataType::Date64
);
define_scalar_type!(Time32Scalar, super::Time32, "time32");
define_scalar_type!(Time64Scalar, super::Time64, "time64");
define_scalar_type!(Duration32Scalar, super::Duration32, "duration32");
define_scalar_type!(Duration64Scalar, super::Duration64, "duration64");
define_scalar_type!(IntervalScalar, super::Interval, "interval");

/// One logical temporal family, independent of its physical width.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TemporalFamily {
    /// Calendar dates.
    Date,
    /// Times of day.
    Time,
    /// Epoch or wall-clock datetimes.
    DateTime,
    /// Elapsed durations.
    Duration,
}

impl TemporalFamily {
    /// Return the canonical family name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTime => "datetime",
            Self::Duration => "duration",
        }
    }
}

/// A borrowed, width-aware view over any temporal [`Scalar`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TemporalRef<'a> {
    family: TemporalFamily,
    count: i64,
    unit: TimeUnit,
    timezone: &'a Timezone,
    bit_width: u8,
}

impl<'a> TemporalRef<'a> {
    const fn new(
        family: TemporalFamily,
        count: i64,
        unit: TimeUnit,
        timezone: &'a Timezone,
        bit_width: u8,
    ) -> Self {
        Self {
            family,
            count,
            unit,
            timezone,
            bit_width,
        }
    }

    /// Return the logical temporal family.
    pub const fn family(self) -> TemporalFamily {
        self.family
    }

    /// Return the stored physical count.
    pub const fn count(self) -> i64 {
        self.count
    }

    /// Return the stored resolution.
    pub const fn unit(self) -> TimeUnit {
        self.unit
    }

    /// Return the explicit timezone marker.
    pub const fn timezone(self) -> &'a Timezone {
        self.timezone
    }

    /// Return the physical count width.
    pub const fn bit_width(self) -> u8 {
        self.bit_width
    }

    /// Return the deterministic hash of the complete exact view.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Rebuild and validate the exact scalar variant this view came from.
    pub fn into_scalar(self) -> Result<Scalar> {
        match (self.family, self.bit_width) {
            (TemporalFamily::Date, 32) => {
                Scalar::date32_in(narrow_i32(self.count, "date32")?, self.unit, *self.timezone)
            }
            (TemporalFamily::Date, 64) => Scalar::date64_in(self.count, self.unit, *self.timezone),
            (TemporalFamily::Time, 32) => {
                Scalar::time32(narrow_i32(self.count, "time32")?, self.unit, *self.timezone)
            }
            (TemporalFamily::Time, 64) => Scalar::time64(self.count, self.unit, *self.timezone),
            (TemporalFamily::DateTime, 64) => {
                Scalar::datetime64(self.count, self.unit, *self.timezone)
            }
            (TemporalFamily::Duration, 32) => Scalar::duration32_in(
                narrow_i32(self.count, "duration32")?,
                self.unit,
                *self.timezone,
            ),
            (TemporalFamily::Duration, 64) => {
                Scalar::duration64_in(self.count, self.unit, *self.timezone)
            }
            _ => Err(invalid("invalid temporal family width")),
        }
    }
}

impl Scalar {
    /// Build the exact date width selected by its unit.
    pub fn from_date(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match unit {
            TimeUnit::Day => Self::date32_in(narrow_i32(count, "date32")?, unit, zone),
            TimeUnit::Millisecond => Self::date64_in(count, unit, zone),
            _ => Err(invalid("date unit must be day or millisecond")),
        }
    }

    /// Build the exact time-of-day width selected by its unit.
    pub fn from_time(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match unit {
            TimeUnit::Second | TimeUnit::Millisecond => {
                Self::time32(narrow_i32(count, "time32")?, unit, zone)
            }
            TimeUnit::Microsecond | TimeUnit::Nanosecond => Self::time64(count, unit, zone),
            _ => Err(invalid(
                "time unit must be second, millisecond, microsecond, or nanosecond",
            )),
        }
    }

    /// Build a 64-bit epoch or wall-clock datetime.
    pub fn from_datetime(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        Self::datetime64(count, unit, zone)
    }

    /// Build the narrowest duration width that holds `count`.
    pub fn from_duration(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        match i32::try_from(count) {
            Ok(count) => Self::duration32_in(count, unit, zone),
            Err(_) => Self::duration64_in(count, unit, zone),
        }
    }

    /// Build a Date32 day count.
    pub const fn date32(days: i32) -> Self {
        Self::Date32(days, TimeUnit::Day, Timezone::NAIVE)
    }

    /// Build a Date32 after validating its unit and zone.
    pub fn date32_in(days: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(unit == TimeUnit::Day, "date32 unit must be day")?;
        require(zone.is_naive(), "date32 timezone must be NAIVE")?;
        Ok(Self::Date32(days, unit, zone))
    }

    /// Build a Date64 millisecond count.
    pub const fn date64(milliseconds: i64) -> Self {
        Self::Date64(milliseconds, TimeUnit::Millisecond, Timezone::NAIVE)
    }

    /// Build a Date64 after validating its unit and zone.
    pub fn date64_in(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit == TimeUnit::Millisecond,
            "date64 unit must be millisecond",
        )?;
        require(zone.is_naive(), "date64 timezone must be NAIVE")?;
        Ok(Self::Date64(count, unit, zone))
    }

    /// Build a 32-bit time of day.
    pub fn time32(count: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            matches!(unit, TimeUnit::Second | TimeUnit::Millisecond),
            "time32 unit must be second or millisecond",
        )?;
        require(
            zone.is_naive(),
            "time32 timezone must be NAIVE because its datatype has no timezone",
        )?;
        Ok(Self::Time32(count, unit, zone))
    }

    /// Build a 64-bit time of day.
    pub fn time64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond),
            "time64 unit must be microsecond or nanosecond",
        )?;
        require(
            zone.is_naive(),
            "time64 timezone must be NAIVE because its datatype has no timezone",
        )?;
        Ok(Self::Time64(count, unit, zone))
    }

    /// Build an instant or wall-clock datetime at 64-bit width.
    pub fn datetime64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_arrow_time(),
            "datetime64 requires an Arrow time unit",
        )?;
        Ok(Self::DateTime64(count, unit, zone))
    }

    /// Parse a timezone and build a 64-bit datetime.
    pub fn datetime64_in(count: i64, unit: TimeUnit, zone: &str) -> Result<Self> {
        Self::datetime64(count, unit, Timezone::from_str(zone)?)
    }

    /// Build a 32-bit duration.
    pub fn duration32(count: i32, unit: TimeUnit) -> Result<Self> {
        Self::duration32_in(count, unit, Timezone::NAIVE)
    }

    /// Build a 32-bit duration after validating its explicit timezone marker.
    pub fn duration32_in(count: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_temporal(),
            "duration32 requires a fixed temporal unit",
        )?;
        require(zone.is_naive(), "duration32 timezone must be NAIVE")?;
        Ok(Self::Duration32(count, unit, zone))
    }

    /// Build a 64-bit duration.
    pub fn duration64(count: i64, unit: TimeUnit) -> Result<Self> {
        Self::duration64_in(count, unit, Timezone::NAIVE)
    }

    /// Build a 64-bit duration after validating its explicit timezone marker.
    pub fn duration64_in(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
        require(
            unit.is_temporal(),
            "duration64 requires a fixed temporal unit",
        )?;
        require(zone.is_naive(), "duration64 timezone must be NAIVE")?;
        Ok(Self::Duration64(count, unit, zone))
    }

    /// Return Date32's count, unit, and zone.
    pub const fn as_date32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Date32(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Date64's count, unit, and zone.
    pub const fn as_date64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Date64(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Time32's count, unit, and zone.
    pub const fn as_time32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Time32(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Time64's count, unit, and zone.
    pub const fn as_time64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Time64(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return DateTime64's count, unit, and zone.
    pub const fn as_datetime64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::DateTime64(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Duration32's count, unit, and zone.
    pub const fn as_duration32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
        match self {
            Self::Duration32(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Return Duration64's count, unit, and zone.
    pub const fn as_duration64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
        match self {
            Self::Duration64(count, unit, zone) => Some((*count, *unit, zone)),
            _ => None,
        }
    }

    /// Borrow the shared family view of any temporal value.
    pub const fn as_temporal(&self) -> Option<TemporalRef<'_>> {
        match self {
            Self::Date32(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Date,
                *count as i64,
                *unit,
                zone,
                32,
            )),
            Self::Date64(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Date,
                *count,
                *unit,
                zone,
                64,
            )),
            Self::Time32(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Time,
                *count as i64,
                *unit,
                zone,
                32,
            )),
            Self::Time64(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Time,
                *count,
                *unit,
                zone,
                64,
            )),
            Self::DateTime64(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::DateTime,
                *count,
                *unit,
                zone,
                64,
            )),
            Self::Duration32(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Duration,
                *count as i64,
                *unit,
                zone,
                32,
            )),
            Self::Duration64(count, unit, zone) => Some(TemporalRef::new(
                TemporalFamily::Duration,
                *count,
                *unit,
                zone,
                64,
            )),
            _ => None,
        }
    }

    /// Borrow this value as either exact date width.
    pub const fn as_date(&self) -> Option<TemporalRef<'_>> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Date) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as either exact time-of-day width.
    pub const fn as_time(&self) -> Option<TemporalRef<'_>> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Time) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as a datetime without exposing its physical suffix.
    pub const fn as_datetime(&self) -> Option<TemporalRef<'_>> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::DateTime) => Some(value),
            _ => None,
        }
    }

    /// Borrow this value as either exact duration width.
    pub const fn as_duration(&self) -> Option<TemporalRef<'_>> {
        match self.as_temporal() {
            Some(value) if matches!(value.family(), TemporalFamily::Duration) => Some(value),
            _ => None,
        }
    }

    /// Return the non-optional timezone carried by any temporal.
    pub const fn temporal_timezone(&self) -> Option<&Timezone> {
        match self.as_temporal() {
            Some(value) => Some(value.timezone()),
            None => None,
        }
    }

    /// Read one temporal from its classic text spelling, at the exact width,
    /// unit and zone `dtype` declares.
    ///
    /// This is the crate's one text reading of a temporal: the row evaluator,
    /// the field-directed record parsers and the Arrow cast leaf all arrive
    /// here, so a spelling reads the same count wherever it is met. The unit
    /// the spelling names is restated in the declared one and has to land
    /// exactly - `10:00:00.500` is no `time32(second)` - and the zone is the
    /// datatype's: a zoned datetime wants an offset in the text, while a
    /// `NAIVE` datetime refuses one.
    ///
    /// # Errors
    ///
    /// Returns the parse error the spelling raised, or an invalid-value error
    /// when the count does not fit the declared unit and width.
    pub(crate) fn from_temporal_text(dtype: &DataType, text: &str) -> Result<Self> {
        match dtype {
            DataType::Date32 => Ok(Self::date32(iso::parse_date(text)?)),
            DataType::Date64 => i64::from(iso::parse_date(text)?)
                .checked_mul(86_400_000)
                .map(Self::date64)
                .ok_or_else(|| invalid("date64 count must fit signed 64 bits")),
            DataType::Time32(unit) => {
                let count = restated(clock_of_day(text)?, *unit, "time32")?;
                Self::time32(narrow_i32(count, "time32")?, *unit, Timezone::NAIVE)
            }
            DataType::Time64(unit) => {
                let count = restated(clock_of_day(text)?, *unit, "time64")?;
                Self::time64(count, *unit, Timezone::NAIVE)
            }
            DataType::DateTime64 { unit, timezone } if timezone.is_naive() => {
                let count = restated(iso::parse_datetime(text)?, *unit, "datetime64")?;
                Self::datetime64(count, *unit, Timezone::NAIVE)
            }
            DataType::DateTime64 { unit, timezone } => {
                let (count, source, _) = iso::parse_timestamp(text)?;
                let count = restated((count, source), *unit, "datetime64")?;
                Self::datetime64(count, *unit, *timezone)
            }
            DataType::Duration32(unit) => {
                let count = restated(iso::parse_duration(text)?, *unit, "duration32")?;
                Self::duration32(narrow_i32(count, "duration32")?, *unit)
            }
            DataType::Duration64(unit) => {
                let count = restated(iso::parse_duration(text)?, *unit, "duration64")?;
                Self::duration64(count, *unit)
            }
            other => Err(invalid(format!("{other} holds no temporal text"))),
        }
    }

    /// Spell this temporal the classic way, when it has a classic spelling.
    ///
    /// This is the crate's one text spelling of a temporal: an expression
    /// literal, a cast to text and the Arrow cast leaf all render here, so a
    /// value reads back as what it printed. A reading with no classic
    /// spelling - a date beyond four-digit years, an interval layout -
    /// answers `None`, as [`iso`] does.
    // The name states the conversion direction, as the text codecs' own
    // `into_*` readers do; the spelling is built, so it cannot borrow.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn into_temporal_text(&self) -> Option<smol_str::SmolStr> {
        match self {
            Self::Date32(days, _, _) => iso::format_date(*days),
            Self::Date64(count, _, _) => i32::try_from(count.div_euclid(86_400_000))
                .ok()
                .and_then(iso::format_date),
            Self::Time32(count, unit, _) => iso::format_time(i64::from(*count), *unit),
            Self::Time64(count, unit, _) => iso::format_time(*count, *unit),
            Self::DateTime64(count, unit, zone) if zone.is_naive() => {
                iso::format_datetime(*count, *unit)
            }
            Self::DateTime64(count, unit, zone) => iso::format_timestamp(*count, *unit, zone),
            Self::Duration32(count, unit, _) => iso::format_duration(i64::from(*count), *unit),
            Self::Duration64(count, unit, _) => iso::format_duration(*count, *unit),
            _ => None,
        }
    }

    /// Return this temporal's count restated in `unit`, when exact.
    pub fn temporal_count_at(&self, unit: TimeUnit) -> Option<i64> {
        let temporal = self.as_temporal()?;
        let (count, current) = (temporal.count(), temporal.unit());
        if current == unit {
            return Some(count);
        }
        let nanoseconds = i128::from(count) * nanoseconds_per(current)?;
        let divisor = nanoseconds_per(unit)?;
        (nanoseconds % divisor == 0)
            .then(|| i64::try_from(nanoseconds / divisor).ok())
            .flatten()
    }

    /// Return whether this is a temporal value.
    pub const fn is_temporal(&self) -> bool {
        self.as_temporal().is_some()
    }

    /// Return the datatype this temporal materializes into.
    pub fn temporal_dtype(&self) -> Option<DataType> {
        self.is_temporal().then(|| self.dtype().ok()).flatten()
    }
}

/// Read a time of day, naming the type that reads a zoned clock instead.
///
/// An offset makes a clock an instant, and the message says so rather than
/// reporting the offset as trailing text.
fn clock_of_day(text: &str) -> Result<(i64, TimeUnit)> {
    let zoned = text.ends_with(['Z', 'z'])
        || text
            .len()
            .checked_sub(6)
            .is_some_and(|start| matches!(text.as_bytes()[start], b'+' | b'-'));
    if zoned {
        return Err(invalid(
            "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
        ));
    }
    iso::parse_time(text)
}

/// Restate a parsed count in the unit its datatype declares, when exact.
fn restated((count, source): (i64, TimeUnit), unit: TimeUnit, kind: &'static str) -> Result<i64> {
    if source == unit {
        return Ok(count);
    }
    let restate = |count: i128| -> Option<i64> {
        let nanoseconds = count.checked_mul(nanoseconds_per(source)?)?;
        let divisor = nanoseconds_per(unit)?;
        (nanoseconds % divisor == 0)
            .then(|| i64::try_from(nanoseconds / divisor).ok())
            .flatten()
    };
    restate(i128::from(count)).ok_or_else(|| invalid(format!("{kind} count is no exact {unit}")))
}

fn narrow_i32(count: i64, kind: &'static str) -> Result<i32> {
    i32::try_from(count).map_err(|_| invalid(format!("{kind} count must fit signed 32 bits")))
}

fn invalid(reason: impl Into<smol_str::SmolStr>) -> Error {
    Error::InvalidRecord {
        path: "$".into(),
        reason: reason.into(),
    }
}

fn require(valid: bool, reason: &'static str) -> Result<()> {
    if valid { Ok(()) } else { Err(invalid(reason)) }
}

const fn nanoseconds_per(unit: TimeUnit) -> Option<i128> {
    match unit {
        TimeUnit::Day => Some(86_400_000_000_000),
        TimeUnit::Second => Some(1_000_000_000),
        TimeUnit::Millisecond => Some(1_000_000),
        TimeUnit::Microsecond => Some(1_000),
        TimeUnit::Nanosecond => Some(1),
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => None,
    }
}

pub(crate) fn temporal_key(count: i64, unit: TimeUnit) -> (u8, i128) {
    let count = i128::from(count);
    match unit {
        TimeUnit::Day => (0, count * 86_400_000_000_000),
        TimeUnit::Second => (0, count * 1_000_000_000),
        TimeUnit::Millisecond => (0, count * 1_000_000),
        TimeUnit::Microsecond => (0, count * 1_000),
        TimeUnit::Nanosecond => (0, count),
        TimeUnit::YearMonth => (1, count),
        TimeUnit::DayTime => (2, count),
        TimeUnit::MonthDayNano => (3, count),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_reject_illegal_width_unit_combinations() {
        assert!(Scalar::time32(1, TimeUnit::Microsecond, Timezone::NAIVE).is_err());
        assert!(Scalar::time64(1, TimeUnit::Millisecond, Timezone::NAIVE).is_err());
        assert!(Scalar::duration32(1, TimeUnit::DayTime).is_err());
        assert!(Scalar::duration64(1, TimeUnit::DayTime).is_err());
        assert!(Scalar::duration32_in(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::duration64_in(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::date32_in(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
        assert!(Scalar::date64_in(1, TimeUnit::Second, Timezone::NAIVE).is_err());
    }

    #[test]
    fn time_of_day_refuses_a_zone_its_datatype_would_lose() {
        for error in [
            Scalar::time32(1, TimeUnit::Second, Timezone::UTC).unwrap_err(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::UTC).unwrap_err(),
        ] {
            let message = error.to_string();
            assert!(message.contains("timezone"), "{message}");
            assert!(message.contains("no timezone"), "{message}");
        }
    }

    #[test]
    fn every_temporal_carries_a_zone() {
        let values = [
            Scalar::date32(1),
            Scalar::date64(86_400_000),
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
            Scalar::duration32(1, TimeUnit::Millisecond).unwrap(),
            Scalar::duration64(1, TimeUnit::Microsecond).unwrap(),
        ];
        assert!(values.iter().all(Scalar::is_temporal));
    }

    #[test]
    fn family_constructors_select_exact_widths() {
        let values = [
            Scalar::from_date(1, TimeUnit::Day, Timezone::NAIVE).unwrap(),
            Scalar::from_date(86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            Scalar::from_time(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::from_time(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
            Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
            Scalar::from_duration(i64::from(i32::MAX), TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE)
                .unwrap(),
        ];
        assert!(matches!(values[0], Scalar::Date32(..)));
        assert!(matches!(values[1], Scalar::Date64(..)));
        assert!(matches!(values[2], Scalar::Time32(..)));
        assert!(matches!(values[3], Scalar::Time64(..)));
        assert!(matches!(values[4], Scalar::DateTime64(..)));
        assert!(matches!(values[5], Scalar::Duration32(..)));
        assert!(matches!(values[6], Scalar::Duration64(..)));

        for count in [i64::from(i32::MIN), i64::from(i32::MAX)] {
            assert!(matches!(
                Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Duration32(..)
            ));
        }
        for count in [i64::from(i32::MIN) - 1, i64::from(i32::MAX) + 1] {
            assert!(matches!(
                Scalar::from_duration(count, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
                Scalar::Duration64(..)
            ));
        }
    }

    #[test]
    fn temporal_family_views_are_exact_and_reversible() {
        let values = [
            Scalar::date32(1),
            Scalar::date64(86_400_000),
            Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time64(1, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
            Scalar::duration32(1, TimeUnit::Millisecond).unwrap(),
            Scalar::duration64(1, TimeUnit::Microsecond).unwrap(),
        ];
        for value in &values {
            let temporal = value.as_temporal().unwrap();
            assert_eq!(&temporal.into_scalar().unwrap(), value);
            assert_eq!(temporal.timezone(), value.temporal_timezone().unwrap());
        }
        assert_eq!(values[0].as_date().unwrap().bit_width(), 32);
        assert_eq!(values[1].as_date().unwrap().bit_width(), 64);
        assert!(values[2].as_time().is_some());
        assert!(values[4].as_datetime().is_some());
        assert!(values[5].as_duration().is_some());
        assert!(Scalar::from(1).as_temporal().is_none());
    }

    #[test]
    fn family_constructors_reject_invalid_parts() {
        assert!(Scalar::from_date(1, TimeUnit::Second, Timezone::NAIVE).is_err());
        assert!(Scalar::from_date(i64::MAX, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_time(1, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_time(1, TimeUnit::Second, Timezone::UTC).is_err());
        assert!(Scalar::from_datetime(1, TimeUnit::Day, Timezone::NAIVE).is_err());
        assert!(Scalar::from_duration(1, TimeUnit::DayTime, Timezone::NAIVE).is_err());
        assert!(Scalar::from_duration(1, TimeUnit::Second, Timezone::UTC).is_err());
    }
}

#[derive(Clone)]
pub(crate) struct TemporalParts {
    pub(crate) family: TemporalFamily,
    pub(crate) unit: TimeUnit,
    pub(crate) zone: Timezone,
    pub(crate) dtype: DataType,
}

pub(crate) fn temporal_value_parts(value: &Scalar) -> Option<TemporalParts> {
    let temporal = value.as_temporal()?;
    let unit = temporal.unit();
    let zone = *temporal.timezone();
    let dtype = match (temporal.family(), temporal.bit_width()) {
        (TemporalFamily::Date, 32) => DataType::Date32,
        (TemporalFamily::Date, 64) => DataType::Date64,
        (TemporalFamily::Time, 32) => DataType::Time32(unit),
        (TemporalFamily::Time, 64) => DataType::Time64(unit),
        (TemporalFamily::DateTime, 64) => DataType::DateTime64 {
            unit,
            timezone: zone,
        },
        (TemporalFamily::Duration, 32) => DataType::Duration32(unit),
        (TemporalFamily::Duration, 64) => DataType::Duration64(unit),
        _ => return None,
    };
    Some(TemporalParts {
        family: temporal.family(),
        unit,
        zone,
        dtype,
    })
}

pub(crate) fn temporal_target(dtype: &DataType) -> Option<(TemporalFamily, TimeUnit)> {
    match dtype {
        DataType::Date32 => Some((TemporalFamily::Date, TimeUnit::Day)),
        DataType::Date64 => Some((TemporalFamily::Date, TimeUnit::Millisecond)),
        DataType::Time32(unit) | DataType::Time64(unit) => Some((TemporalFamily::Time, *unit)),
        DataType::DateTime64 { unit, .. } => Some((TemporalFamily::DateTime, *unit)),
        DataType::Duration32(unit) | DataType::Duration64(unit) => {
            Some((TemporalFamily::Duration, *unit))
        }
        _ => None,
    }
}

pub(crate) fn temporal_result_type(
    left: &Scalar,
    left_parts: TemporalParts,
    operation: Arithmetic,
    right: &Scalar,
    right_parts: TemporalParts,
) -> Result<DataType> {
    match (left_parts.family, right_parts.family, operation) {
        (family, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if family != TemporalFamily::Duration =>
        {
            Ok(left_parts.dtype)
        }
        (TemporalFamily::Duration, family, Arithmetic::Add)
            if family != TemporalFamily::Duration =>
        {
            Ok(right_parts.dtype)
        }
        (family, other, Arithmetic::Sub)
            if family == other && family != TemporalFamily::Duration =>
        {
            if left_parts.zone.is_naive() != right_parts.zone.is_naive() {
                return Err(invalid_binary(
                    operation,
                    left,
                    right,
                    "zoned and timezone-naive temporal values cannot be subtracted",
                ));
            }
            let unit = finer_unit(left_parts.unit, right_parts.unit);
            DataType::duration64(unit)
                .map_err(|error| invalid_binary(operation, left, right, error.to_string()))
        }
        (TemporalFamily::Duration, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub) => {
            let unit = finer_unit(left_parts.unit, right_parts.unit);
            let wide = matches!(left_parts.dtype, DataType::Duration64(_))
                || matches!(right_parts.dtype, DataType::Duration64(_));
            if wide {
                DataType::duration64(unit)
            } else {
                DataType::duration32(unit)
            }
            .map_err(|error| invalid_binary(operation, left, right, error.to_string()))
        }
        _ => Err(invalid_binary(
            operation,
            left,
            right,
            "temporal arithmetic supports temporal +/- duration, temporal subtraction, and duration +/- duration",
        )),
    }
}

fn finer_unit(left: TimeUnit, right: TimeUnit) -> TimeUnit {
    if unit_rank(left) >= unit_rank(right) {
        left
    } else {
        right
    }
}

const fn unit_rank(unit: TimeUnit) -> u8 {
    match unit {
        TimeUnit::Day => 0,
        TimeUnit::Second => 1,
        TimeUnit::Millisecond => 2,
        TimeUnit::Microsecond => 3,
        TimeUnit::Nanosecond => 4,
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => 5,
    }
}

pub(crate) fn temporal_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    target: &DataType,
) -> Result<Scalar> {
    if !matches!(operation, Arithmetic::Add | Arithmetic::Sub) {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "temporal multiplication, division, and remainder are undefined",
        ));
    }
    let left_parts = temporal_value_parts(left)
        .ok_or_else(|| invalid_binary(operation, left, right, "left operand is not temporal"))?;
    let right_parts = temporal_value_parts(right)
        .ok_or_else(|| invalid_binary(operation, left, right, "right operand is not temporal"))?;
    let (target_family, unit) = temporal_target(target).ok_or_else(|| {
        invalid_binary(
            operation,
            left,
            right,
            "the promoted datatype is not temporal",
        )
    })?;
    let (left_count, right_count) = match (left_parts.family, right_parts.family, operation) {
        (family, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if family == target_family =>
        {
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        (TemporalFamily::Duration, family, Arithmetic::Add) if family == target_family => (
            temporal_at(right, unit, operation, left)?,
            temporal_at(left, unit, operation, right)?,
        ),
        (family, other, Arithmetic::Sub)
            if family == other
                && target_family == TemporalFamily::Duration
                && family != TemporalFamily::Duration =>
        {
            if left_parts.zone.is_naive() != right_parts.zone.is_naive() {
                return Err(invalid_binary(
                    operation,
                    left,
                    right,
                    "zoned and timezone-naive temporal values cannot be subtracted",
                ));
            }
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        (TemporalFamily::Duration, TemporalFamily::Duration, Arithmetic::Add | Arithmetic::Sub)
            if target_family == TemporalFamily::Duration =>
        {
            (
                temporal_at(left, unit, operation, right)?,
                temporal_at(right, unit, operation, left)?,
            )
        }
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "operands do not match the promoted temporal result",
            ));
        }
    };
    let held = match operation {
        Arithmetic::Add => left_count.checked_add(right_count),
        Arithmetic::Sub => left_count.checked_sub(right_count),
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "temporal multiplication, division, and remainder are undefined",
            ));
        }
    }
    .ok_or_else(|| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })?;
    temporal_value(target, held, unit).map_err(|_| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })
}

pub(crate) fn duration_integer_arithmetic(
    left: &Scalar,
    operation: Arithmetic,
    right: &Scalar,
    target: &DataType,
) -> Result<Scalar> {
    let (duration, integer, duration_first) = if temporal_value_parts(left)
        .is_some_and(|parts| parts.family == TemporalFamily::Duration)
        && right.as_integer().is_some()
    {
        (left, right, true)
    } else if left.as_integer().is_some()
        && temporal_value_parts(right).is_some_and(|parts| parts.family == TemporalFamily::Duration)
    {
        (right, left, false)
    } else {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "expected one duration and one integer",
        ));
    };
    if !(matches!(operation, Arithmetic::Mul)
        || duration_first && matches!(operation, Arithmetic::Div))
    {
        return Err(invalid_binary(
            operation,
            left,
            right,
            "durations support multiplication by an integer and exact division by an integer",
        ));
    }

    let parts = temporal_value_parts(duration).ok_or_else(|| {
        invalid_binary(operation, left, right, "duration operand is not temporal")
    })?;
    let count = duration
        .temporal_count_at(parts.unit)
        .map(|value| I256::from_i128(i128::from(value)))
        .ok_or_else(|| invalid_binary(operation, left, right, "invalid duration count"))?;
    let scalar = exact_value_parts(integer)
        .map(|parts| parts.0)
        .ok_or_else(|| invalid_binary(operation, left, right, "integer is out of range"))?;
    if scalar.is_zero() && matches!(operation, Arithmetic::Div) {
        return Err(Error::DivisionByZero {
            operation: operation.name(),
        });
    }
    let held = match operation {
        Arithmetic::Mul => count.checked_mul(scalar),
        Arithmetic::Div => {
            if !count
                .checked_rem(scalar)
                .is_some_and(|remainder| remainder.is_zero())
            {
                return Err(Error::InexactArithmetic {
                    operation: operation.name(),
                    kind: temporal_kind_name(target),
                });
            }
            count.checked_div(scalar)
        }
        _ => {
            return Err(invalid_binary(
                operation,
                left,
                right,
                "durations support multiplication or exact division by an integer",
            ));
        }
    }
    .and_then(I256::as_i128)
    .and_then(|value| i64::try_from(value).ok())
    .ok_or_else(|| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })?;
    temporal_value(target, held, parts.unit).map_err(|_| Error::ArithmeticOverflow {
        operation: operation.name(),
        kind: temporal_kind_name(target),
    })
}

fn temporal_at(
    value: &Scalar,
    unit: TimeUnit,
    operation: Arithmetic,
    other: &Scalar,
) -> Result<i64> {
    value.temporal_count_at(unit).ok_or_else(|| {
        invalid_binary(
            operation,
            value,
            other,
            "temporal unit conversion is inexact or out of range",
        )
    })
}

fn temporal_value(dtype: &DataType, count: i64, unit: TimeUnit) -> Result<Scalar> {
    match dtype {
        DataType::Date32 => Scalar::date32_in(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "date32",
            })?,
            unit,
            Timezone::NAIVE,
        ),
        DataType::Date64 => Scalar::date64_in(count, unit, Timezone::NAIVE),
        DataType::Time32(expected) if *expected == unit => Scalar::time32(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "time32",
            })?,
            unit,
            Timezone::NAIVE,
        ),
        DataType::Time64(expected) if *expected == unit => {
            Scalar::time64(count, unit, Timezone::NAIVE)
        }
        DataType::DateTime64 {
            unit: expected,
            timezone,
        } if *expected == unit => Scalar::datetime64(count, unit, *timezone),
        DataType::Duration32(expected) if *expected == unit => Scalar::duration32(
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind: "duration32",
            })?,
            unit,
        ),
        DataType::Duration64(expected) if *expected == unit => Scalar::duration64(count, unit),
        _ => Err(Error::InvalidArithmetic {
            operation: "temporal arithmetic",
            left: temporal_kind_name(dtype),
            right: None,
            reason: "invalid result unit".into(),
        }),
    }
}

const fn temporal_kind_name(dtype: &DataType) -> &'static str {
    match dtype {
        DataType::Date32 => "date32",
        DataType::Date64 => "date64",
        DataType::Time32(_) => "time32",
        DataType::Time64(_) => "time64",
        DataType::DateTime64 { .. } => "datetime64",
        DataType::Duration32(_) => "duration32",
        DataType::Duration64(_) => "duration64",
        _ => "temporal",
    }
}
