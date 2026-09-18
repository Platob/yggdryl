//! Calendar, clock, duration, and interval datatypes.

pub use scalars::{ Date32, Date64, DateTime64, Duration32, Duration64, Interval, TemporalFamily, TemporalValue, Time32, Time64, };
pub(crate) use scalars::{validate_date64, validate_time};
use smol_str::{SmolStr, ToSmolStr, format_smolstr};

use crate::types::invalid;
use crate::types::parser::{Parser, Token, TokenKind, is_closing_or_separator, precision_to_unit};
use crate::types::timezone::{civil_from_days, days_from_civil};
use crate::types::typed::define_field_types;
use crate::{DataType, DataTypeId, Error, Result, TimeUnit, Timezone, TypedField};

#[cfg(feature = "arrow")]
/// Arrow casts owned by this datatype family.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BooleanArray, StringArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
    use arrow_cast::can_cast_types;
    use arrow_schema::DataType as ArrowDataType;
    use arrow_select::zip::zip;

    use crate::arrow::{Error, Result};
    use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
    use crate::types::cast::arrow_cast_exposed;
    use crate::types::cast::text::ingest_text_values;
    use crate::types::cast::columns::is_exposed;
    use crate::{DataType, Field, Scalar};

    /// Whether a source Arrow type holds temporals with a classic spelling.
    ///
    /// The interval layouts are temporal too and have no such spelling, so they
    /// keep Arrow's rendering.
    pub(crate) fn is_temporal_arrow(source: &ArrowDataType) -> bool {
        match source {
            ArrowDataType::Date32
            | ArrowDataType::Date64
            | ArrowDataType::Time32(_)
            | ArrowDataType::Time64(_)
            | ArrowDataType::Timestamp(..)
            | ArrowDataType::Duration(_) => true,
            ArrowDataType::Dictionary(_, values) => is_temporal_arrow(values),
            ArrowDataType::RunEndEncoded(_, values) => is_temporal_arrow(values.data_type()),
            _ => false,
        }
    }

    /// Renders a temporal column as the classic text this crate spells.
    ///
    /// [`Scalar::into_temporal_text`] is what an expression literal and the row
    /// evaluator spell with, and it reads the zone rules this crate owns, so a
    /// zoned instant renders here where Arrow's formatter refuses one. A value
    /// with no classic spelling keeps Arrow's rendering.
    pub(crate) fn render_temporal_text(
        array: &ArrayRef,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let source_type = DataType::from_arrow(array.data_type())?;
        let rows = array.len();
        budget.add_array(field.dtype(), rows)?;
        reserve_vec_bytes::<Option<smol_str::SmolStr>>(budget, rows)?;
        let mut spelled = Vec::with_capacity(rows);
        let mut ours = BooleanBufferBuilder::new(rows);
        let mut unspelled = false;
        for index in 0..rows {
            let text = if is_exposed(exposure, index) && array.is_valid(index) {
                crate::arrow::value::value_from_array(&source_type, array.as_ref(), index)?
                    .into_temporal_text()
            } else {
                None
            };
            let absent = !is_exposed(exposure, index) || array.is_null(index);
            ours.append(absent || text.is_some());
            unspelled |= !absent && text.is_none();
            spelled.push(text);
        }
        // The reservation above charges the offsets a text array carries; the
        // spellings themselves are the payload this loop built.
        budget.add_bytes(spelled.iter().flatten().map(smol_str::SmolStr::len).sum())?;
        let mask = BooleanArray::new(ours.finish(), None);
        let read_here: ArrayRef = Arc::new(StringArray::from_iter(
            spelled
                .iter()
                .map(|text| text.as_ref().map(smol_str::SmolStr::as_str)),
        ));
        if !unspelled {
            return Ok(read_here);
        }
        // Arrow's formatter keeps the readings this crate has no spelling for,
        // such as a date outside four-digit years; where it has none either, this
        // crate's nulls stand.
        let cast = if can_cast_types(array.data_type(), &ArrowDataType::Utf8) {
            let rendered = arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                true,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            );
            match rendered {
                Ok(arrow) => zip(&mask, &read_here.as_ref(), &arrow.as_ref())?,
                Err(_) => read_here,
            }
        } else {
            read_here
        };
        if !safe {
            for index in 0..rows {
                if !mask.value(index) && cast.is_null(index) {
                    return Err(Error::IncompatibleSchema(format!(
                        "field {:?} row {index}: this {source_type} reading has no classic spelling",
                        field.name(),
                    )));
                }
            }
        }
        Ok(cast)
    }

    /// Whether a target datatype holds temporals, however it encodes them.
    pub(crate) fn holds_temporal(target: &DataType) -> bool {
        match target {
            DataType::Date32
            | DataType::Date64
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::DateTime64 { .. }
            | DataType::Duration32(_)
            | DataType::Duration64(_) => true,
            DataType::Dictionary(dictionary) => holds_temporal(dictionary.value()),
            DataType::RunEndEncoded(encoded) => holds_temporal(encoded.values().dtype()),
            _ => false,
        }
    }

    /// Reads a column of temporal text through this crate's own spellings.
    ///
    /// [`Scalar::from_temporal_text`] is what the row evaluator reads one value
    /// with, so a batch and a row cannot answer differently about a spelling this
    /// crate knows - its refusals included: a reading this crate takes but its
    /// declared unit or width cannot hold is null here as it is there, never
    /// Arrow's rounded one. Arrow's kernel answers only what this crate cannot
    /// read at all - a twelve-hour clock, or a bare date entering a zoned column,
    /// which states no zone this crate would invent for it - so the column still
    /// reads everything it used to. A bare date entering a naive datetime is this
    /// crate's own reading, the compact `YYYYMMDD` included, which closes the one
    /// spelling the two tiers did answer differently about.
    pub(crate) fn ingest_temporal_text(
        array: &ArrayRef,
        expected: &ArrowDataType,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        ingest_text_values(
            array,
            expected,
            safe,
            field,
            exposure,
            budget,
            Scalar::from_temporal_text,
        )
    }
}

// ------------------------------------------------------------------------
// Temporal units and validated time-of-day construction.
// ------------------------------------------------------------------------

/// One temporal or interval datatype.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TemporalType {
    /// Day-count date.
    Date32,
    /// Millisecond-count date.
    Date64,
    /// 32-bit time of day.
    Time32(TimeUnit),
    /// 64-bit time of day.
    Time64(TimeUnit),
    /// 64-bit datetime with an explicit timezone.
    DateTime64 { unit: TimeUnit, timezone: Timezone },
    /// 32-bit duration.
    Duration32(TimeUnit),
    /// 64-bit duration.
    Duration64(TimeUnit),
    /// Calendar interval layout.
    Interval(TimeUnit),
}

impl TemporalType {
    /// Return the exact datatype identifier.
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Date32 => DataTypeId::Date32,
            Self::Date64 => DataTypeId::Date64,
            Self::Time32(_) => DataTypeId::Time32,
            Self::Time64(_) => DataTypeId::Time64,
            Self::DateTime64 { .. } => DataTypeId::DateTime64,
            Self::Duration32(_) => DataTypeId::Duration32,
            Self::Duration64(_) => DataTypeId::Duration64,
            Self::Interval(_) => DataTypeId::Interval,
        }
    }

    /// Validate and convert this family member into the root datatype.
    pub fn into_dtype(self) -> Result<DataType> {
        match self {
            Self::Date32 => Ok(DataType::Date32),
            Self::Date64 => Ok(DataType::Date64),
            Self::Time32(unit) => DataType::time32(unit),
            Self::Time64(unit) => DataType::time64(unit),
            Self::DateTime64 { unit, timezone } => DataType::datetime64(unit, timezone),
            Self::Duration32(unit) => DataType::duration32(unit),
            Self::Duration64(unit) => DataType::duration64(unit),
            Self::Interval(unit) if unit.is_interval() => Ok(DataType::Interval(unit)),
            Self::Interval(_) => Err(invalid(
                "Interval",
                "unit must be year_month, day_time, or month_day_nano",
            )),
        }
    }
}

impl From<TemporalType> for DataType {
    fn from(value: TemporalType) -> Self {
        match value {
            TemporalType::Date32 => Self::Date32,
            TemporalType::Date64 => Self::Date64,
            TemporalType::Time32(unit) => Self::Time32(unit),
            TemporalType::Time64(unit) => Self::Time64(unit),
            TemporalType::DateTime64 { unit, timezone } => Self::DateTime64 { unit, timezone },
            TemporalType::Duration32(unit) => Self::Duration32(unit),
            TemporalType::Duration64(unit) => Self::Duration64(unit),
            TemporalType::Interval(unit) => Self::Interval(unit),
        }
    }
}

impl TryFrom<&DataType> for TemporalType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value {
            DataType::Date32 => Ok(Self::Date32),
            DataType::Date64 => Ok(Self::Date64),
            DataType::Time32(unit) => Ok(Self::Time32(*unit)),
            DataType::Time64(unit) => Ok(Self::Time64(*unit)),
            DataType::DateTime64 { unit, timezone } => Ok(Self::DateTime64 {
                unit: *unit,
                timezone: *timezone,
            }),
            DataType::Duration32(unit) => Ok(Self::Duration32(*unit)),
            DataType::Duration64(unit) => Ok(Self::Duration64(*unit)),
            DataType::Interval(unit) => Ok(Self::Interval(*unit)),
            other => Err(Error::InvalidDataType {
                kind: "temporal",
                reason: format_smolstr!("expected a temporal datatype, got {other}"),
            }),
        }
    }
}

impl DataType {
    /// Creates a 64-bit datetime with an explicit timezone marker.
    ///
    /// Use [`Timezone::NAIVE`] for a wall-clock column without timezone
    /// interpretation.
    pub fn datetime64(unit: TimeUnit, timezone: Timezone) -> Result<Self> {
        if !unit.is_arrow_time() {
            return Err(invalid(
                "DateTime64",
                "unit must be second, millisecond, microsecond, or nanosecond",
            ));
        }
        Ok(Self::DateTime64 { unit, timezone })
    }

    /// Creates the Arrow time-of-day type selected by the requested unit.
    ///
    /// Seconds and milliseconds use [`Self::Time32`], while microseconds and
    /// nanoseconds use [`Self::Time64`]. Validation is delegated to
    /// [`Self::time32`] or [`Self::time64`]. Calendar interval layouts are not
    /// time-of-day resolutions and return an error.
    pub fn time(unit: TimeUnit) -> Result<Self> {
        match unit {
            TimeUnit::Second | TimeUnit::Millisecond => Self::time32(unit),
            TimeUnit::Microsecond | TimeUnit::Nanosecond => Self::time64(unit),
            TimeUnit::Day | TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => {
                Err(invalid("Time", "unit must be a temporal resolution"))
            }
        }
    }

    /// Creates a 32-bit time-of-day type with a valid physical unit.
    pub fn time32(unit: TimeUnit) -> Result<Self> {
        validate_time32_unit(unit)?;
        Ok(Self::Time32(unit))
    }

    /// Creates a 64-bit time-of-day type with a valid physical unit.
    pub fn time64(unit: TimeUnit) -> Result<Self> {
        validate_time64_unit(unit)?;
        Ok(Self::Time64(unit))
    }

    /// Creates a 32-bit elapsed-time type with a fixed-length resolution.
    pub fn duration32(unit: TimeUnit) -> Result<Self> {
        validate_duration_unit("Duration32", unit)?;
        Ok(Self::Duration32(unit))
    }

    /// Creates a 64-bit elapsed-time type with a fixed-length resolution.
    ///
    /// Arrow durations are physically 64-bit at every resolution, so Arrow
    /// import always selects this variant.
    pub fn duration64(unit: TimeUnit) -> Result<Self> {
        validate_duration_unit("Duration64", unit)?;
        Ok(Self::Duration64(unit))
    }
}

pub(crate) fn validate_time32_unit(unit: TimeUnit) -> Result<()> {
    if matches!(unit, TimeUnit::Second | TimeUnit::Millisecond) {
        Ok(())
    } else {
        Err(invalid("Time32", "unit must be second or millisecond"))
    }
}

pub(crate) fn validate_time64_unit(unit: TimeUnit) -> Result<()> {
    if matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond) {
        Ok(())
    } else {
        Err(invalid("Time64", "unit must be microsecond or nanosecond"))
    }
}

pub(crate) fn validate_duration_unit(kind: &'static str, unit: TimeUnit) -> Result<()> {
    if unit.is_temporal() {
        Ok(())
    } else {
        Err(invalid(
            kind,
            "unit must be day, second, millisecond, microsecond, or nanosecond",
        ))
    }
}

// ------------------------------------------------------------------------
// Temporal and interval field markers.
// ------------------------------------------------------------------------

define_field_types!(
    DateTime64Type,
    DateTime64,
    crate::DataType::DateTime64 { .. }
);
define_field_types!(Date32Type, Date32, crate::DataType::Date32);
define_field_types!(Date64Type, Date64, crate::DataType::Date64);
define_field_types!(Time32Type, Time32, crate::DataType::Time32(_));
define_field_types!(Time64Type, Time64, crate::DataType::Time64(_));
define_field_types!(Duration32Type, Duration32, crate::DataType::Duration32(_));
define_field_types!(Duration64Type, Duration64, crate::DataType::Duration64(_));
define_field_types!(IntervalType, Interval, crate::DataType::Interval(_));

/// A DateTime64-typed field.
pub type DateTime64Field = TypedField<DateTime64Type>;
// The classic ISO 8601 spellings of the temporals, beside the temporals they
// spell. Crate-private: the structured-text codecs and the scalar renderer are
// its only callers.

// ------------------------------------------------------------------------
// The classic ISO spellings of the temporals.
//
// A temporal used to travel through the text formats as a tagged tuple - a
// unit name beside a count - which no other tool reads. Every other tool
// reads `2026-08-17`, `10:00:00.123`, `2026-08-17T10:00:00Z`, and `PT90S`,
// so those are what the emitters write now. The spelling stays exact both
// ways: the fraction is printed at the unit's full width, so the number of
// digits *is* the unit, and a zoned instant carries its offset plus the zone
// name in brackets when the name says more than the offset does.
//
// Formatting answers `None` for a reading with no classic spelling - a date
// beyond four-digit years, a time of day outside its day, an interval-layout
// duration - and the caller keeps its structural spelling instead. Parsing is
// strict about shape and total about meaning: `2026-02-30` is an error, not a
// guess.
//
// An hour past the end of the day is a reading rather than an error, and what
// it means is whose day it is: a time of day folds it modulo the day, so
// `25:30:00` is `01:30:00`; a datetime carries it into the following date, so
// `2026-08-17T25:30:00` is the 18th at `01:30`; and a duration keeps it plain,
// because `25:30:00` of elapsed time is twenty-five and a half hours and never
// half past one.
//
// A missing clock is a reading too: a date is a datetime at that day's
// midnight, `2026-08-17` and the compact `20260817` alike, which is what lets
// a FIX settlement date sit in the same column as a transact time instead of
// a second temporal type to cast through. It is read and never written - a
// datetime spells its clock - and a zoned reading still states its zone, so
// `20260818Z` is that day in UTC and `20260818` is a naive reading only.
// ------------------------------------------------------------------------

/// Seconds in one day, the modulus a datetime splits on.
const DAY: i64 = 86_400;

/// The subdivisions of one second a resolution unit holds.
///
/// An interval layout has no fixed width, so it has no classic spelling.
pub(crate) const fn per_second(unit: TimeUnit) -> Option<i64> {
    match unit {
        TimeUnit::Second => Some(1),
        TimeUnit::Millisecond => Some(1_000),
        TimeUnit::Microsecond => Some(1_000_000),
        TimeUnit::Nanosecond => Some(1_000_000_000),
        TimeUnit::Day | TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => None,
    }
}

/// The fraction digits one resolution unit prints, which is how the unit
/// round-trips: three digits are milliseconds, six are microseconds.
const fn fraction_digits(unit: TimeUnit) -> usize {
    match unit {
        TimeUnit::Millisecond => 3,
        TimeUnit::Microsecond => 6,
        TimeUnit::Nanosecond => 9,
        _ => 0,
    }
}

/// The resolution unit one fraction width means.
const fn unit_of_fraction(digits: usize) -> TimeUnit {
    match digits {
        0 => TimeUnit::Second,
        1..=3 => TimeUnit::Millisecond,
        4..=6 => TimeUnit::Microsecond,
        _ => TimeUnit::Nanosecond,
    }
}

/// Write `seconds.fraction` for one in-day count, at the unit's full width.
fn push_clock(text: &mut String, count: i64, unit: TimeUnit) {
    let per = per_second(unit).expect("a resolution unit reaches the clock");
    let seconds = count.div_euclid(per);
    let fraction = count.rem_euclid(per);
    let (hours, minutes, seconds) = (seconds / 3_600, (seconds / 60) % 60, seconds % 60);
    text.push_str(&format!("{hours:02}:{minutes:02}:{seconds:02}"));
    let digits = fraction_digits(unit);
    if digits > 0 {
        text.push_str(&format!(".{fraction:0digits$}"));
    }
}

/// Spell a day count as `YYYY-MM-DD`, when it has four-digit years.
pub(crate) fn format_date(days: i32) -> Option<SmolStr> {
    let (year, month, day) = civil_from_days(i64::from(days));
    if !(0..=9_999).contains(&year) {
        return None;
    }
    Some(format_smolstr!("{year:04}-{month:02}-{day:02}"))
}

/// Spell a time of day as `HH:MM:SS[.fraction]`, when it is inside its day.
pub(crate) fn format_time(count: i64, unit: TimeUnit) -> Option<SmolStr> {
    let per = per_second(unit)?;
    if count < 0 || count >= DAY.checked_mul(per)? {
        return None;
    }
    let mut text = String::with_capacity(18);
    push_clock(&mut text, count, unit);
    Some(SmolStr::from(text))
}

/// Spell a naive reading as `YYYY-MM-DDTHH:MM:SS[.fraction]`.
pub(crate) fn format_datetime(count: i64, unit: TimeUnit) -> Option<SmolStr> {
    let per = per_second(unit)?;
    let days = count.div_euclid(DAY * per);
    let in_day = count.rem_euclid(DAY * per);
    let (year, month, day) = civil_from_days(days);
    if !(0..=9_999).contains(&year) {
        return None;
    }
    let mut text = String::with_capacity(30);
    text.push_str(&format!("{year:04}-{month:02}-{day:02}T"));
    push_clock(&mut text, in_day, unit);
    Some(SmolStr::from(text))
}

/// Spell a zoned instant as its local reading plus its offset.
///
/// The count is UTC, as Arrow defines it; the spelling is the local wall
/// clock with the offset that recovers the instant - `Z` for UTC itself - and
/// the zone's name in brackets when the name is a place rather than an
/// offset, because `+02:00` cannot say `Europe/Paris`. A zone this build has
/// no rules for spells the UTC reading with `Z` and keeps its bracketed name.
pub(crate) fn format_timestamp(count: i64, unit: TimeUnit, zone: &Timezone) -> Option<SmolStr> {
    let per = per_second(unit)?;
    let offset = zone.offset_at(count.div_euclid(per));
    let local = match offset {
        Some(offset) => count.checked_add(i64::from(offset).checked_mul(per)?)?,
        None => count,
    };
    let mut text = String::from(format_datetime(local, unit)?.as_str());
    match offset {
        Some(0) if zone.is_utc() => text.push('Z'),
        None => text.push('Z'),
        Some(offset) => {
            let sign = if offset < 0 { '-' } else { '+' };
            let total = offset.unsigned_abs();
            text.push_str(&format!(
                "{sign}{:02}:{:02}",
                total / 3_600,
                (total % 3_600) / 60
            ));
        }
    }
    if !zone.is_utc() && !zone.is_fixed() {
        text.push('[');
        text.push_str(zone.as_str());
        text.push(']');
    }
    Some(SmolStr::from(text))
}

/// Spell an elapsed count as the ISO duration `PT<seconds>[.fraction]S`.
///
/// Seconds are the one component every unit restates exactly, so the spelling
/// never decomposes into hours a reader would have to multiply back. The sign
/// leads, as ISO 8601 puts it. An interval layout has no fixed width and
/// answers `None`.
pub(crate) fn format_duration(count: i64, unit: TimeUnit) -> Option<SmolStr> {
    let per = per_second(unit)?;
    let magnitude = count.unsigned_abs();
    let seconds = magnitude / per.unsigned_abs();
    let fraction = magnitude % per.unsigned_abs();
    let sign = if count < 0 { "-" } else { "" };
    let digits = fraction_digits(unit);
    Some(if digits == 0 {
        format_smolstr!("{sign}PT{seconds}S")
    } else {
        format_smolstr!("{sign}PT{seconds}.{fraction:0digits$}S")
    })
}

/// The error one malformed ISO spelling reports.
fn iso_error(target: &'static str, position: usize, reason: &'static str) -> Error {
    Error::Parse {
        target,
        position,
        reason: reason.to_smolstr(),
    }
}

/// Read exactly `width` ASCII digits at `position`.
///
/// Accumulated directly rather than sliced and re-parsed: every caller here
/// asks for two or four digits, and `str::parse` re-walks the bytes, re-checks
/// each one and builds a error value this never needs. The widths are bounded
/// by the callers, so the accumulator cannot overflow.
fn digits(text: &str, position: usize, width: usize, target: &'static str) -> Result<i64> {
    let bytes = text.as_bytes();
    let Some(slice) = bytes.get(position..position + width) else {
        return Err(iso_error(target, position, "expected digits"));
    };
    let mut held: i64 = 0;
    for byte in slice {
        if !byte.is_ascii_digit() {
            return Err(iso_error(target, position, "expected digits"));
        }
        held = held * 10 + i64::from(byte - b'0');
    }
    Ok(held)
}

/// Whether the byte at `position` is an ASCII digit.
///
/// What separates the two spellings of every field here: a compact reading
/// runs its digits together where an extended one puts a separator between
/// them, so one look decides which is being read and neither pays for the
/// other.
fn is_digit_at(text: &str, position: usize) -> bool {
    text.as_bytes()
        .get(position)
        .is_some_and(u8::is_ascii_digit)
}

/// Expect one literal byte at `position`.
fn literal(text: &str, position: usize, byte: u8, target: &'static str) -> Result<()> {
    if text.as_bytes().get(position) == Some(&byte) {
        return Ok(());
    }
    Err(iso_error(target, position, "unexpected separator"))
}

/// Parse `YYYY-MM-DD` or `YYYYMMDD` into a day count since the Unix epoch.
pub(crate) fn parse_date(text: &str) -> Result<i32> {
    let (days, end) = parse_date_at(text, 0)?;
    if text.len() != end {
        return Err(iso_error("date", end, "trailing text after the date"));
    }
    Ok(days)
}

/// Parse the date at `position`, answering it and the position after it.
///
/// Both spellings are read: the extended `YYYY-MM-DD` and the compact
/// `YYYYMMDD` a wire writes when every byte counts. FIX is the reason - it
/// spells every date and the date half of every timestamp compactly - and a
/// log line quoting one is the same value however it was written.
fn parse_date_at(text: &str, position: usize) -> Result<(i32, usize)> {
    let year = digits(text, position, 4, "date")?;
    let compact = is_digit_at(text, position + 4);
    let (month_at, day_at, end) = if compact {
        (position + 4, position + 6, position + 8)
    } else {
        literal(text, position + 4, b'-', "date")?;
        (position + 5, position + 8, position + 10)
    };
    let month = digits(text, month_at, 2, "date")?;
    if !compact {
        literal(text, position + 7, b'-', "date")?;
    }
    let day = digits(text, day_at, 2, "date")?;
    if !(1..=12).contains(&month) {
        return Err(iso_error("date", month_at, "month must be 01 to 12"));
    }
    let days = days_from_civil(year as i32, month as u32, day as u32);
    // Round-tripping the civil date rejects a day the month does not have,
    // such as February 30th, without a table of month lengths.
    if day < 1 || civil_from_days(days) != (year as i32, month as u32, day as u32) {
        return Err(iso_error("date", day_at, "no such day in this month"));
    }
    let days =
        i32::try_from(days).map_err(|_| iso_error("date", position, "date is out of range"))?;
    Ok((days, end))
}

/// Read the optional `.fraction` at `position`, returning its count at the
/// unit's full width, that unit, and the end position. A clock with no
/// fraction reads zero at second resolution and does not move the position.
///
/// Either decimal sign opens the fraction. ISO 8601 divides the fraction from
/// the integer part with the decimal sign of ISO 31-0 - the comma or the full
/// stop - and names the comma the preferred one, which is what log4j and the
/// European locales write, so `00:05:01,148` is a clock and not a clock
/// followed by a list. RFC 3339 allows only the full stop, and that is the one
/// this crate writes back: a comma is read here and spelled as a dot, so a
/// reading normalizes on the way out and no formatter needs a sign to choose.
///
/// The fraction accepts `_` digit-group separators - `.000_000` reads exactly
/// as `.000000` - because that is how a log emitter that groups microseconds
/// spells a clock. A separator is legal only *between* digits; the digit count
/// with separators removed keeps the 1-to-9 rule, so grouping never changes
/// the unit a width names.
fn parse_fraction_at(
    text: &str,
    position: usize,
    target: &'static str,
) -> Result<(i64, TimeUnit, usize)> {
    if !matches!(text.as_bytes().get(position), Some(b'.' | b',')) {
        return Ok((0, TimeUnit::Second, position));
    }
    let start = position + 1;
    let mut end = start;
    let mut width = 0_usize;
    let mut fraction: i64 = 0;
    loop {
        match text.as_bytes().get(end) {
            Some(byte) if byte.is_ascii_digit() => {
                width += 1;
                if width > 9 {
                    return Err(iso_error(target, start, "fraction must hold 1 to 9 digits"));
                }
                fraction = fraction * 10 + i64::from(byte - b'0');
                end += 1;
            }
            Some(b'_') => {
                // `_` groups digits, so one digit sits on each side of it:
                // `.5_` , `._5`, and `.5__5` are all malformed.
                let follows_digit =
                    end > start && text.as_bytes().get(end - 1).is_some_and(u8::is_ascii_digit);
                let precedes_digit = text.as_bytes().get(end + 1).is_some_and(u8::is_ascii_digit);
                if !follows_digit || !precedes_digit {
                    return Err(iso_error(
                        target,
                        end,
                        "a fraction separator sits between digits",
                    ));
                }
                end += 1;
            }
            _ => break,
        }
    }
    if width == 0 {
        return Err(iso_error(target, start, "fraction must hold 1 to 9 digits"));
    }
    let unit = unit_of_fraction(width);
    // A short fraction is right-padded to the unit's width: `.5` is `500`
    // milliseconds.
    for _ in width..fraction_digits(unit) {
        fraction *= 10;
    }
    Ok((fraction, unit, end))
}

/// Parse `HH:MM:SS[.fraction]`, returning the count, unit, and end position.
///
/// The hour field is two digits and may name an hour past the end of the day,
/// which the count states as it was written: the caller that owns the day
/// decides whether that folds or carries. Minutes and seconds are calendar
/// fields, so they stay under sixty.
fn parse_clock_at(
    text: &str,
    position: usize,
    target: &'static str,
) -> Result<(i64, TimeUnit, usize)> {
    let hours = digits(text, position, 2, target)?;
    // The compact `HHMMSS` beside the extended `HH:MM:SS`, decided by one
    // look, exactly as the date is.
    let compact = is_digit_at(text, position + 2);
    let (minutes_at, seconds_at, end) = if compact {
        (position + 2, position + 4, position + 6)
    } else {
        literal(text, position + 2, b':', target)?;
        (position + 3, position + 6, position + 8)
    };
    let minutes = digits(text, minutes_at, 2, target)?;
    if !compact {
        literal(text, position + 5, b':', target)?;
    }
    let seconds = digits(text, seconds_at, 2, target)?;
    if minutes >= 60 || seconds >= 60 {
        return Err(iso_error(target, position, "clock reading out of range"));
    }
    let whole = hours * 3_600 + minutes * 60 + seconds;
    let (fraction, unit, end) = parse_fraction_at(text, end, target)?;
    let per = per_second(unit).expect("a fraction width names a resolution unit");
    Ok((whole * per + fraction, unit, end))
}

/// Parse `HH:MM:SS[.fraction]` into a count of `unit` since midnight.
///
/// A clock past the end of its day folds into it, because a time of day is a
/// place on one dial and `24:00:00` is the midnight that closes a shift as
/// much as the one that opens the next: hours run to `99`, and the reading is
/// taken modulo the day.
pub(crate) fn parse_time(text: &str) -> Result<(i64, TimeUnit)> {
    let (count, unit, end) = parse_clock_at(text, 0, "time")?;
    if end != text.len() {
        return Err(iso_error("time", end, "trailing text after the time"));
    }
    let per = per_second(unit).expect("the clock parsed at a resolution unit");
    Ok((count.rem_euclid(DAY * per), unit))
}

/// Parse a naive datetime, returning the position after it.
///
/// Every spelling of one instant: the extended
/// `YYYY-MM-DDTHH:MM:SS[.fraction]`, the compact `YYYYMMDDHHMMSS[.fraction]`
/// a wire writes, and the FIX form `YYYYMMDD-HH:MM:SS[.fraction]` that mixes
/// them. The separator between the halves is required only where the clock
/// would otherwise run into the date: a compact date is eight digits and a
/// compact clock six, so `YYYYMMDDHHMMSS` reads without one, while the
/// extended date must still be closed before the clock opens.
///
/// A date states no clock and reads as that day at midnight, extended and
/// compact alike, because a settlement date sent as `20260818` is the same
/// instant a bridge spells `2026-08-18T00:00:00`. What ends the date says so
/// by itself - the text ends, or the byte a zone opens with begins, and the
/// caller that owns the zone reads it next - so no reading is attempted twice
/// and a clock that breaks still names the byte that broke it. `-` is not one
/// of those bytes: after a date it is FIX's clock separator, so
/// `20260818-10:15:00` is a clock and `20260818-08:00` is one that stops
/// halfway, never an offset on a bare date.
fn parse_datetime_at(text: &str, target: &'static str) -> Result<(i64, TimeUnit, usize)> {
    let (days, after) = parse_date_at(text, 0)?;
    let clock_at = match text.as_bytes().get(after) {
        Some(b'T' | b't' | b' ' | b'-') => after + 1,
        // A compact date runs straight into its clock, which is the whole
        // point of writing it that way.
        Some(byte) if byte.is_ascii_digit() && after == 8 => after,
        // Nothing, `Z`, an offset sign, or a bracketed zone name: the date
        // states the whole reading and the rest is the caller's. Midnight is
        // a clock with no fraction, so it reads at the resolution one does,
        // and an `i32` day count times a day of seconds is nowhere near what
        // 64 bits hold, so the multiplication a clock has to guard cannot
        // overflow without one.
        None | Some(b'Z' | b'z' | b'+' | b'[') => {
            return Ok((i64::from(days) * DAY, TimeUnit::Second, after));
        }
        _ => {
            return Err(iso_error(target, after, "expected T between date and time"));
        }
    };
    let (in_day, unit, end) = parse_clock_at(text, clock_at, target)?;
    let per = per_second(unit).expect("the clock parsed at a resolution unit");
    let count = i64::from(days)
        .checked_mul(DAY * per)
        .and_then(|days| days.checked_add(in_day))
        .ok_or_else(|| iso_error(target, 0, "datetime is out of range"))?;
    Ok((count, unit, end))
}

/// Parse a naive datetime into a count of `unit` since the Unix epoch.
///
/// A clock past the end of its day carries into the following date, because a
/// datetime names one point on the line rather than a place on the dial:
/// `2026-08-17T24:00:00` is the 18th at midnight. A reading that states no
/// clock is that day's own midnight, at second resolution.
pub(crate) fn parse_datetime(text: &str) -> Result<(i64, TimeUnit)> {
    let (count, unit, end) = parse_datetime_at(text, "datetime")?;
    if end != text.len() {
        return Err(iso_error(
            "datetime",
            end,
            "a naive datetime carries no zone",
        ));
    }
    Ok((count, unit))
}

/// Parse a zoned instant: a local reading, an offset or `Z`, and optionally
/// the zone's bracketed name, which wins over the offset when both appear.
///
/// The local reading carries an hour past the end of its day into the
/// following date, and states no clock where it is a bare date, as a naive
/// datetime does: `20260818Z` is the 18th at midnight in UTC. The zone stays
/// required either way, because an instant a column stores must never carry a
/// zone the text did not state.
pub(crate) fn parse_timestamp(text: &str) -> Result<(i64, TimeUnit, Timezone)> {
    let (local, unit, mut end) = parse_datetime_at(text, "timestamp")?;
    let per = per_second(unit).expect("the reading parsed at a resolution unit");

    let offset_start = end;
    let offset = match text.as_bytes().get(end) {
        Some(b'Z' | b'z') => {
            end += 1;
            0
        }
        Some(b'+' | b'-') => {
            let rest = &text[end..];
            let bracket = rest.find('[').unwrap_or(rest.len());
            let spelled = &rest[..bracket];
            let zone = Timezone::from_str(spelled)?;
            end += spelled.len();
            zone.offset_at(0)
                .ok_or_else(|| iso_error("timestamp", offset_start, "expected a fixed offset"))?
        }
        _ => {
            return Err(iso_error(
                "timestamp",
                end,
                "a timestamp needs Z or a zone offset",
            ));
        }
    };

    let count = local
        .checked_sub(
            i64::from(offset)
                .checked_mul(per)
                .ok_or_else(|| iso_error("timestamp", offset_start, "timestamp is out of range"))?,
        )
        .ok_or_else(|| iso_error("timestamp", offset_start, "timestamp is out of range"))?;

    let zone = match text[end..].strip_prefix('[') {
        Some(rest) => {
            let name = rest
                .strip_suffix(']')
                .ok_or_else(|| iso_error("timestamp", end, "unclosed zone bracket"))?;
            Timezone::from_str(name)?
        }
        None if end == text.len() => match offset {
            0 => Timezone::UTC,
            offset => Timezone::from_offset(offset)?,
        },
        None => {
            return Err(iso_error("timestamp", end, "trailing text after the zone"));
        }
    };
    Ok((count, unit, zone))
}

/// Parse an elapsed duration into a count of `unit`.
///
/// Two spellings read the same count. The ISO general form is accepted -
/// `-P1DT2H3M4.5S` - and every component is restated in seconds, so the
/// writer's seconds-only spelling and a reader's decomposed one meet at the
/// same number. Beside it, a plain clock - `-25:30:00.500` - reads the elapsed
/// hours as they were written: hours take as many digits as the count needs
/// and never fold, because twenty-five hours of work is not one o'clock.
/// Either way the sign leads, as ISO 8601 puts it, and the unit is the
/// fraction's width.
pub(crate) fn parse_duration(text: &str) -> Result<(i64, TimeUnit)> {
    let (sign, position) = match text.as_bytes().first() {
        Some(b'-') => (-1_i64, 1),
        Some(b'+') => (1, 1),
        _ => (1, 0),
    };
    let (magnitude, unit) = match text.as_bytes().get(position) {
        Some(b'P' | b'p') => parse_duration_components(text, position + 1)?,
        _ => parse_elapsed_clock(text, position)?,
    };
    let count = magnitude
        .checked_mul(sign)
        .ok_or_else(|| iso_error("duration", 0, "duration is out of range"))?;
    Ok((count, unit))
}

/// Parse the `nDTnHnMnS` components after `P`, at their own byte positions.
fn parse_duration_components(text: &str, mut position: usize) -> Result<(i64, TimeUnit)> {
    let bytes = text.as_bytes();
    let mut seconds: i64 = 0;
    let mut fraction: i64 = 0;
    let mut unit = TimeUnit::Second;
    let mut in_time = false;
    let mut saw_component = false;

    while position < bytes.len() {
        if bytes[position] == b'T' || bytes[position] == b't' {
            in_time = true;
            position += 1;
            continue;
        }
        let start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        let number: i64 = text[start..position]
            .parse()
            .map_err(|_| iso_error("duration", start, "expected a component count"))?;

        // A fraction is only classic on the final seconds component.
        let (value, parsed_unit, end) = parse_fraction_at(text, position, "duration")?;
        let with_fraction = end != position;
        position = end;

        let label = *bytes
            .get(position)
            .ok_or_else(|| iso_error("duration", position, "component is missing its label"))?;
        let weight = match (label.to_ascii_uppercase(), in_time) {
            (b'D', false) => DAY,
            (b'H', true) => 3_600,
            (b'M', true) => 60,
            (b'S', true) => 1,
            _ => {
                return Err(iso_error(
                    "duration",
                    position,
                    "expected D, or T then H, M, S",
                ));
            }
        };
        if with_fraction {
            if weight != 1 {
                return Err(iso_error(
                    "duration",
                    position,
                    "only the seconds component takes a fraction",
                ));
            }
            fraction = value;
            unit = parsed_unit;
        }
        position += 1;
        seconds = number
            .checked_mul(weight)
            .and_then(|component| seconds.checked_add(component))
            .ok_or_else(|| iso_error("duration", start, "duration is out of range"))?;
        saw_component = true;
    }

    if !saw_component {
        return Err(iso_error(
            "duration",
            position,
            "a duration names a component",
        ));
    }
    Ok((elapsed(seconds, fraction, unit)?, unit))
}

/// Parse `H+:MM:SS[.fraction]`, the plain clock an elapsed count also spells.
///
/// The hours are a count, not a calendar field, so they take any width and
/// stay whole; minutes and seconds remain the two-digit fields under sixty
/// that a clock has.
fn parse_elapsed_clock(text: &str, position: usize) -> Result<(i64, TimeUnit)> {
    let bytes = text.as_bytes();
    let mut end = position;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if end == position {
        return Err(iso_error(
            "duration",
            position,
            "a duration spells P components or a plain clock",
        ));
    }
    let hours: i64 = text[position..end]
        .parse()
        .map_err(|_| iso_error("duration", position, "duration is out of range"))?;
    literal(text, end, b':', "duration")?;
    let minutes = digits(text, end + 1, 2, "duration")?;
    literal(text, end + 3, b':', "duration")?;
    let seconds = digits(text, end + 4, 2, "duration")?;
    if minutes >= 60 || seconds >= 60 {
        return Err(iso_error("duration", end + 1, "clock reading out of range"));
    }
    let (fraction, unit, end) = parse_fraction_at(text, end + 6, "duration")?;
    if end != text.len() {
        return Err(iso_error(
            "duration",
            end,
            "trailing text after the duration",
        ));
    }
    let seconds = hours
        .checked_mul(3_600)
        .and_then(|hours| hours.checked_add(minutes * 60 + seconds))
        .ok_or_else(|| iso_error("duration", position, "duration is out of range"))?;
    Ok((elapsed(seconds, fraction, unit)?, unit))
}

/// Restate whole seconds plus a sub-second count as one count of `unit`.
fn elapsed(seconds: i64, fraction: i64, unit: TimeUnit) -> Result<i64> {
    let per = per_second(unit).expect("the fraction width names a resolution unit");
    seconds
        .checked_mul(per)
        .and_then(|whole| whole.checked_add(fraction))
        .ok_or_else(|| iso_error("duration", 0, "duration is out of range"))
}

#[cfg(test)]
mod iso_tests {
    use super::*;

    #[test]
    fn dates_round_trip_and_reject_days_that_do_not_exist() {
        assert_eq!(format_date(0).as_deref(), Some("1970-01-01"));
        assert_eq!(format_date(20_682).as_deref(), Some("2026-08-17"));
        assert_eq!(format_date(-719_162).as_deref(), Some("0001-01-01"));

        assert_eq!(parse_date("2026-08-17").unwrap(), 20_682);
        assert_eq!(parse_date("1970-01-01").unwrap(), 0);
        // A leap day parses in a leap year and errors in any other.
        assert!(parse_date("2024-02-29").is_ok());
        assert!(parse_date("2026-02-29").is_err());
        assert!(parse_date("2026-13-01").is_err());
        assert!(parse_date("2026-08-17T").is_err());

        // A five-digit year has no classic spelling, so it keeps its structure.
        assert_eq!(format_date(i32::MAX), None);
    }

    #[test]
    fn times_print_the_fraction_at_the_unit_width() {
        assert_eq!(
            format_time(0, TimeUnit::Second).as_deref(),
            Some("00:00:00")
        );
        assert_eq!(
            format_time(36_000 + 23 * 60 + 45, TimeUnit::Second).as_deref(),
            Some("10:23:45")
        );
        assert_eq!(
            format_time(36_001_500, TimeUnit::Millisecond).as_deref(),
            Some("10:00:01.500")
        );
        assert_eq!(
            format_time(1, TimeUnit::Nanosecond).as_deref(),
            Some("00:00:00.000000001")
        );

        // The digit count is the unit on the way back.
        assert_eq!(
            parse_time("10:00:01.500").unwrap(),
            (36_001_500, TimeUnit::Millisecond)
        );
        assert_eq!(parse_time("10:23:45").unwrap(), (37_425, TimeUnit::Second));
        assert_eq!(
            parse_time("00:00:00.5").unwrap(),
            (500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_time("00:00:00.000001").unwrap(),
            (1, TimeUnit::Microsecond)
        );

        // A reading outside its day has no clock spelling.
        assert_eq!(format_time(-1, TimeUnit::Second), None);
        assert_eq!(format_time(86_400, TimeUnit::Second), None);
    }

    #[test]
    fn a_time_of_day_folds_an_hour_past_the_end_of_its_day() {
        // Midnight closing a shift is the midnight that opens the next day.
        assert_eq!(parse_time("24:00:00").unwrap(), (0, TimeUnit::Second));
        assert_eq!(parse_time("25:30:00").unwrap(), (5_400, TimeUnit::Second));
        // Hours run to the two digits the field holds: 99:59:59 is 03:59:59.
        assert_eq!(parse_time("99:59:59").unwrap(), (14_399, TimeUnit::Second));
        assert!(parse_time("100:00:00").is_err());

        // The fold is on the count, so it keeps the fraction and its unit.
        assert_eq!(
            parse_time("24:00:00.500").unwrap(),
            (500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_time("48:00:00.000_001").unwrap(),
            (1, TimeUnit::Microsecond)
        );

        // Minutes and seconds are calendar fields and stay under sixty.
        assert!(parse_time("10:60:00").is_err());
        assert!(parse_time("10:00:60").is_err());

        // Folding is not round-tripping: the spelling comes back inside the day.
        assert_eq!(
            format_time(parse_time("25:30:00").unwrap().0, TimeUnit::Second).as_deref(),
            Some("01:30:00")
        );
    }

    #[test]
    fn a_datetime_carries_an_hour_past_the_end_of_its_day() {
        // A datetime is a point on the line, so the hour carries into the date.
        assert_eq!(
            parse_datetime("2026-08-17T24:00:00").unwrap(),
            parse_datetime("2026-08-18T00:00:00").unwrap()
        );
        assert_eq!(
            parse_datetime("2026-08-17T25:30:00.250").unwrap(),
            parse_datetime("2026-08-18T01:30:00.250").unwrap()
        );
        // Across a month end, the carry uses the calendar rather than a guess.
        assert_eq!(
            parse_datetime("2026-02-28T30:00:00").unwrap(),
            parse_datetime("2026-03-01T06:00:00").unwrap()
        );
        assert_eq!(
            parse_timestamp("2026-08-17T24:00:00Z").unwrap(),
            parse_timestamp("2026-08-18T00:00:00Z").unwrap()
        );
    }

    #[test]
    fn a_date_with_no_clock_reads_as_that_day_at_midnight() {
        // A wire that spells a settlement date spells the day and stops, so the
        // day is the instant it opens - the compact spelling FIX writes and the
        // extended one a bridge quotes it with are one reading.
        let midnight = parse_datetime("2026-08-18T00:00:00").unwrap();
        assert_eq!(midnight, (1_787_011_200, TimeUnit::Second));
        assert_eq!(parse_datetime("2026-08-18").unwrap(), midnight);
        assert_eq!(parse_datetime("20260818").unwrap(), midnight);
        assert_eq!(parse_datetime("19700101").unwrap(), (0, TimeUnit::Second));
        assert_eq!(
            parse_datetime("19691231").unwrap(),
            (-86_400, TimeUnit::Second)
        );

        // The day is still the calendar's, and the date reader is the one that
        // says so: February 30th is no date and therefore no datetime.
        assert!(parse_datetime("20260230").is_err());
        assert!(parse_datetime("2026-02-30").is_err());
        assert!(parse_datetime("20261301").is_err());

        // A zoned reading states its zone and then reads the same midnight, in
        // either case of the `Z` that states UTC.
        let (count, unit, zone) = parse_timestamp("20260818Z").unwrap();
        assert_eq!((count, unit), midnight);
        assert!(zone.is_utc());
        assert_eq!(parse_timestamp("20260818z").unwrap().0, midnight.0);
        let (count, _, zone) = parse_timestamp("2026-08-18+02:00[Europe/Paris]").unwrap();
        assert_eq!(count, midnight.0 - 2 * 3_600);
        assert_eq!(zone, Timezone::from_str("Europe/Paris").unwrap());
        assert_eq!(
            parse_timestamp("2026-08-18+02:00").unwrap().0,
            midnight.0 - 2 * 3_600
        );

        // The zone is required in the text and nowhere else: a bare date is a
        // naive reading, and a naive reading carries no zone.
        assert!(parse_timestamp("20260818").is_err());
        assert!(parse_timestamp("2026-08-18[Europe/Paris]").is_err());
        assert!(parse_datetime("20260818Z").is_err());

        // A date is read as a datetime and never written as one: the count spells
        // its clock back.
        assert_eq!(
            format_datetime(midnight.0, TimeUnit::Second).as_deref(),
            Some("2026-08-18T00:00:00")
        );

        // The other readers keep their own shapes. A date is a date, a clock is
        // not a date, and neither reads the other's trailing text.
        assert_eq!(parse_date("20260818").unwrap(), 20_683);
        assert!(parse_date("2026-08-18T00:00:00").is_err());
        assert!(parse_time("20260818").is_err());
        assert!(parse_duration("20260818").is_err());
    }

    #[test]
    fn a_date_that_opens_a_clock_still_has_to_finish_it() {
        let position = |text: &str| match parse_datetime(text).unwrap_err() {
            Error::Parse {
                target, position, ..
            } => {
                assert_eq!(target, "datetime");
                position
            }
            other => panic!("expected a parse error, got {other}"),
        };

        // A separator opens a clock, so it is followed by one or by nothing at
        // all: the byte the refusal names is the one the clock was owed at.
        assert_eq!(position("2026-08-18T"), 11);
        assert_eq!(position("20260818T"), 9);
        assert_eq!(position("2026-08-18 "), 11);
        // Trailing text that opens nothing names where the date ended instead.
        assert_eq!(position("2026-08-18X"), 10);

        // `-` after a date is FIX's clock separator, which is why a bare date
        // takes `Z` or a `+` offset and never a `-` one: a clock that stops at
        // its minutes is a broken clock rather than a western offset.
        assert_eq!(
            parse_datetime("20260818-10:15:00").unwrap(),
            parse_datetime("2026-08-18T10:15:00").unwrap()
        );
        assert!(parse_timestamp("20260818-08:00").is_err());
        assert!(parse_timestamp("2026-08-18-08:00").is_err());

        // A digit run is a date at eight and a date and a clock at fourteen, and
        // nothing at the widths between, where a field would be half written.
        for held in [
            "2026",
            "202608",
            "2026081",
            "202608181",
            "2026081810",
            "202608181015",
            "2026081810153",
            "202608181015300",
        ] {
            assert!(parse_datetime(held).is_err(), "{held}");
        }
        assert_eq!(
            parse_datetime("20260818101530").unwrap(),
            parse_datetime("2026-08-18T10:15:30").unwrap()
        );
    }

    #[test]
    fn fraction_separators_group_digits_without_changing_the_count() {
        // `_` groups digits; the count and the unit are exactly the ungrouped ones.
        assert_eq!(
            parse_time("10:00:00.000_000").unwrap(),
            parse_time("10:00:00.000000").unwrap()
        );
        assert_eq!(
            parse_time("10:00:00.000_001").unwrap(),
            (36_000_000_001, TimeUnit::Microsecond)
        );
        assert_eq!(
            parse_time("00:00:00.1_2_3").unwrap(),
            (123, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_time("00:00:00.123_456_789").unwrap(),
            (123_456_789, TimeUnit::Nanosecond)
        );
        assert_eq!(
            parse_datetime("2024-02-01 10:00:00.000_000").unwrap(),
            parse_datetime("2024-02-01T10:00:00.000000").unwrap()
        );
        let (count, unit, zone) = parse_timestamp("2024-02-01T10:00:00.500_000Z").unwrap();
        assert_eq!(
            (count, unit),
            (1_706_781_600_500_000, TimeUnit::Microsecond)
        );
        assert!(zone.is_utc());

        // The grouped digits still count toward the 1-to-9 budget.
        assert!(parse_time("00:00:00.000_000_000_1").is_err());
    }

    #[test]
    fn either_decimal_sign_opens_the_same_fraction() {
        // ISO 8601 names the comma and the full stop alike and prefers the comma,
        // which is what a log4j line and a European locale write; the count, the
        // unit and the grouping rule are the ones the full stop already had.
        assert_eq!(
            parse_time("10:00:01,500").unwrap(),
            parse_time("10:00:01.500").unwrap()
        );
        assert_eq!(
            parse_time("00:00:00,5").unwrap(),
            (500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_time("00:00:00,000_001").unwrap(),
            (1, TimeUnit::Microsecond)
        );
        assert_eq!(
            parse_datetime("2026-08-14 00:05:01,148").unwrap(),
            parse_datetime("2026-08-14T00:05:01.148").unwrap()
        );
        let (count, unit, zone) = parse_timestamp("2024-02-01T10:00:00,500_000Z").unwrap();
        assert_eq!(
            (count, unit),
            (1_706_781_600_500_000, TimeUnit::Microsecond)
        );
        assert!(zone.is_utc());

        // The sign is read, never written: a comma comes back as the full stop
        // RFC 3339 allows, so a spelling normalizes on the way out.
        assert_eq!(
            format_time(parse_time("00:00:00,5").unwrap().0, TimeUnit::Millisecond).as_deref(),
            Some("00:00:00.500")
        );

        // One sign, not two: the second is trailing text, not more fraction.
        assert!(parse_time("00:00:00,5.5").is_err());
        assert!(parse_time("00:00:00.5,5").is_err());
    }

    #[test]
    fn malformed_fraction_separators_are_rejected_with_their_byte_position() {
        let position = |text: &str| match parse_time(text).unwrap_err() {
            Error::Parse {
                target, position, ..
            } => {
                assert_eq!(target, "time");
                position
            }
            other => panic!("expected a parse error, got {other}"),
        };

        // Leading, trailing, doubled, and lone separators each name the byte of
        // the `_` that breaks the digit-grouping rule.
        assert_eq!(position("00:00:00._5"), 9);
        assert_eq!(position("00:00:00.5_"), 10);
        // In a doubled pair the first `_` is already not between digits.
        assert_eq!(position("00:00:00.5__5"), 10);
        assert_eq!(position("00:00:00._"), 9);
        assert_eq!(position("00:00:00.1_2_"), 12);

        // The comma opens a fraction exactly where the full stop does, so a
        // malformed one names the same byte.
        assert_eq!(position("00:00:00,"), 9);
        assert_eq!(position("00:00:00,_5"), 9);
        assert_eq!(position("00:00:00,1_2_"), 12);

        // The same clock feeds the datetime and timestamp parsers.
        assert!(parse_datetime("2024-02-01T10:00:00._5").is_err());
        assert!(parse_timestamp("2024-02-01T10:00:00.5_Z").is_err());
    }

    #[test]
    fn naive_datetimes_split_the_epoch_count_exactly() {
        assert_eq!(
            format_datetime(1_700_000_000, TimeUnit::Second).as_deref(),
            Some("2023-11-14T22:13:20")
        );
        assert_eq!(
            format_datetime(-1, TimeUnit::Second).as_deref(),
            Some("1969-12-31T23:59:59")
        );
        assert_eq!(
            parse_datetime("2023-11-14T22:13:20").unwrap(),
            (1_700_000_000, TimeUnit::Second)
        );
        assert_eq!(
            parse_datetime("2023-11-14 22:13:20.250").unwrap(),
            (1_700_000_000_250, TimeUnit::Millisecond)
        );
        // A naive reading carries no zone, and saying one is an error.
        assert!(parse_datetime("2023-11-14T22:13:20Z").is_err());
    }

    #[test]
    fn zoned_instants_spell_the_local_reading_and_recover_the_instant() {
        let utc = Timezone::UTC;
        assert_eq!(
            format_timestamp(1_700_000_000, TimeUnit::Second, &utc).as_deref(),
            Some("2023-11-14T22:13:20Z")
        );

        let kolkata = Timezone::from_str("Asia/Kolkata").unwrap();
        assert_eq!(
            format_timestamp(0, TimeUnit::Second, &kolkata).as_deref(),
            Some("1970-01-01T05:30:00+05:30[Asia/Kolkata]")
        );

        let fixed = Timezone::from_str("-08:00").unwrap();
        assert_eq!(
            format_timestamp(3_600_000, TimeUnit::Millisecond, &fixed).as_deref(),
            Some("1969-12-31T17:00:00.000-08:00")
        );

        // The offset recovers the instant; the bracket recovers the name.
        let (count, unit, zone) = parse_timestamp("1970-01-01T05:30:00+05:30[Asia/Kolkata]").unwrap();
        assert_eq!((count, unit), (0, TimeUnit::Second));
        assert_eq!(zone, kolkata);

        let (count, unit, zone) = parse_timestamp("2023-11-14T22:13:20Z").unwrap();
        assert_eq!((count, unit), (1_700_000_000, TimeUnit::Second));
        assert!(zone.is_utc());

        let (count, _, zone) = parse_timestamp("1969-12-31T17:00:00.000-08:00").unwrap();
        assert_eq!(count, 3_600_000);
        assert_eq!(zone, fixed);

        // A zone with no rules in this build keeps the instant in UTC and the
        // name in the bracket.
        let unknown = Timezone::from_str("Mars/Olympus").unwrap();
        let spelled = format_timestamp(60, TimeUnit::Second, &unknown).unwrap();
        assert_eq!(spelled, "1970-01-01T00:01:00Z[Mars/Olympus]");
        let (count, _, zone) = parse_timestamp(&spelled).unwrap();
        assert_eq!(count, 60);
        assert_eq!(zone, unknown);

        assert!(parse_timestamp("2023-11-14T22:13:20").is_err());
    }

    #[test]
    fn durations_spell_seconds_and_read_any_decomposition() {
        assert_eq!(
            format_duration(90, TimeUnit::Second).as_deref(),
            Some("PT90S")
        );
        assert_eq!(
            format_duration(1_500, TimeUnit::Millisecond).as_deref(),
            Some("PT1.500S")
        );
        assert_eq!(
            format_duration(-1_500, TimeUnit::Millisecond).as_deref(),
            Some("-PT1.500S")
        );
        assert_eq!(format_duration(1, TimeUnit::YearMonth), None);

        assert_eq!(parse_duration("PT90S").unwrap(), (90, TimeUnit::Second));
        assert_eq!(
            parse_duration("PT1.500S").unwrap(),
            (1_500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_duration("-PT1.5S").unwrap(),
            (-1_500, TimeUnit::Millisecond)
        );
        // The decomposed general form restates in seconds.
        assert_eq!(
            parse_duration("P1DT2H3M4S").unwrap(),
            (86_400 + 2 * 3_600 + 3 * 60 + 4, TimeUnit::Second)
        );
        assert!(parse_duration("P").is_err());
        assert!(parse_duration("PT1.5M").is_err());

        // The shared fraction reader means the duration spellings take the comma
        // too, as ISO 8601 says they do.
        assert_eq!(
            parse_duration("PT1,5S").unwrap(),
            parse_duration("PT1.5S").unwrap()
        );
        assert!(parse_duration("PT1,5M").is_err());
    }

    #[test]
    fn durations_also_read_a_plain_clock_whose_hours_never_fold() {
        // The hours are elapsed hours, so they stay whole where a time of day
        // would fold: 25:00:00 is twenty-five hours, not one o'clock.
        assert_eq!(
            parse_duration("01:30:00").unwrap(),
            (5_400, TimeUnit::Second)
        );
        assert_eq!(
            parse_duration("25:00:00").unwrap(),
            (90_000, TimeUnit::Second)
        );
        assert_eq!(parse_duration("00:00:00").unwrap(), (0, TimeUnit::Second));
        // A count takes the width it needs, and the sign leads it.
        assert_eq!(
            parse_duration("1:00:00").unwrap(),
            (3_600, TimeUnit::Second)
        );
        assert_eq!(
            parse_duration("100:00:00").unwrap(),
            (360_000, TimeUnit::Second)
        );
        assert_eq!(
            parse_duration("-01:30:00").unwrap(),
            (-5_400, TimeUnit::Second)
        );
        assert_eq!(parse_duration("+00:00:01").unwrap(), (1, TimeUnit::Second));

        // The fraction rules are the clock's own, grouping separators included.
        assert_eq!(
            parse_duration("00:00:01.500").unwrap(),
            (1_500, TimeUnit::Millisecond)
        );
        assert_eq!(
            parse_duration("-00:01:00.000_001").unwrap(),
            (-60_000_001, TimeUnit::Microsecond)
        );
        assert_eq!(
            parse_duration("00:00:00.5").unwrap(),
            parse_duration("PT0.5S").unwrap()
        );
        assert_eq!(
            parse_duration("25:30:00,5").unwrap(),
            parse_duration("25:30:00.5").unwrap()
        );
        // Both spellings of the same elapsed time read the same count.
        assert_eq!(
            parse_duration("26:03:04").unwrap(),
            parse_duration("P1DT2H3M4S").unwrap()
        );

        // Minutes and seconds keep the clock's two-digit fields under sixty.
        assert!(parse_duration("01:60:00").is_err());
        assert!(parse_duration("01:00:60").is_err());
        assert!(parse_duration("01:0:00").is_err());
        assert!(parse_duration("01:30").is_err());
        assert!(parse_duration(":30:00").is_err());
        assert!(parse_duration("01:30:00Z").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("later").is_err());
        // An hour count beyond the unit's reach is out of range, not a wrap.
        assert!(parse_duration("9999999999:00:00.000000001").is_err());
        assert!(parse_duration("99999999999999999999:00:00").is_err());
    }

    #[test]
    fn malformed_durations_name_the_byte_that_breaks_them() {
        let position = |text: &str| match parse_duration(text).unwrap_err() {
            Error::Parse {
                target, position, ..
            } => {
                assert_eq!(target, "duration");
                position
            }
            other => panic!("expected a parse error, got {other}"),
        };

        // Positions are byte offsets into the text as written, sign included.
        assert_eq!(position("-PT1.5M"), 6);
        assert_eq!(position("PT1.5M"), 5);
        // The comma reaches the component labels through the same fraction, so
        // the byte a fraction on the wrong component names moves with it.
        assert_eq!(position("PT1,5M"), 5);
        assert_eq!(position("P1,5D"), 4);
        assert_eq!(position("P1Y"), 2);
        assert_eq!(position("PT1"), 3);
        assert_eq!(position("-01:60:00"), 4);
        assert_eq!(position("-x"), 1);
    }
}

// ------------------------------------------------------------------------
// Temporal datatype grammar.
// ------------------------------------------------------------------------

impl Parser<'_> {
    pub(crate) fn parse_datetime64(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        let mut unit = TimeUnit::Microsecond;
        let mut timezone = if keyword == "timestampltz" || keyword == "timestampwithtimezone" {
            Timezone::UTC
        } else {
            Timezone::NAIVE
        };

        if let Some(close) = self.consume_opening() {
            if self.peek_symbol() != Some(close) {
                if let Some(precision) = self.peek_integer() {
                    let precision_start = self.current_position();
                    self.index += 1;
                    unit = precision_to_unit(precision, precision_start)?;
                } else if self.peek_word_is("none") {
                    self.index += 1;
                    timezone = Timezone::NAIVE;
                } else if self.peek_word_is("some") {
                    self.index += 1;
                    let inner_close = self
                        .consume_opening()
                        .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
                    timezone = self.parse_timezone()?;
                    self.expect_symbol(inner_close)?;
                } else {
                    let (parsed, unit_start) =
                        self.parse_time_unit_span(Some(close), "datetime64 unit")?;
                    unit = parsed;
                    if !unit.is_arrow_time() {
                        return Err(self.error_at(
                            unit_start,
                            "datetime64 requires a temporal resolution unit",
                        ));
                    }
                }

                if self.consume_separator() {
                    self.consume_label("timezone");
                    if self.peek_word_is("none") {
                        self.index += 1;
                        timezone = Timezone::NAIVE;
                    } else if self.peek_word_is("some") {
                        self.index += 1;
                        let inner_close = self
                            .consume_opening()
                            .ok_or_else(|| self.error_here("expected Some(timezone)"))?;
                        timezone = self.parse_timezone()?;
                        self.expect_symbol(inner_close)?;
                    } else {
                        timezone = self.parse_timezone()?;
                    }
                }
            }
            self.expect_symbol(close)?;
        }

        if self.consume_word("with") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            timezone = Timezone::UTC;
        } else if self.consume_word("without") {
            self.expect_word("time")?;
            self.expect_word("zone")?;
            timezone = Timezone::NAIVE;
        }

        DataType::datetime64(unit, timezone)
    }

    pub(crate) fn parse_sql_time(&mut self, depth: usize) -> Result<DataType> {
        self.check_depth(depth)?;
        if self.peek_opening().is_none() {
            return DataType::time(TimeUnit::Microsecond);
        }
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected time unit"))?;
        let (unit, unit_start) = if let Some(precision) = self.peek_integer() {
            let start = self.current_position();
            self.index += 1;
            (precision_to_unit(precision, start)?, start)
        } else {
            self.parse_time_unit_span(Some(close), "time unit")?
        };
        self.expect_symbol(close)?;
        DataType::time(unit).map_err(|error| self.error_at(unit_start, format_smolstr!("{error}")))
    }

    pub(crate) fn parse_required_time_unit(&mut self, depth: usize) -> Result<(TimeUnit, usize)> {
        self.check_depth(depth)?;
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected a temporal unit parameter"))?;
        let (unit, unit_start) = self.parse_time_unit_span(Some(close), "temporal unit")?;
        if !unit.is_arrow_time() {
            return Err(self.error_at(unit_start, "expected a temporal resolution"));
        }
        self.expect_symbol(close)?;
        Ok((unit, unit_start))
    }

    pub(crate) fn parse_interval_unit(&mut self, depth: usize) -> Result<TimeUnit> {
        self.check_depth(depth)?;
        let (unit, unit_start, sql_style) = if self
            .tokens
            .get(self.index)
            .is_none_or(|_| self.is_time_unit_boundary(self.index, None))
        {
            (TimeUnit::MonthDayNano, self.current_position(), false)
        } else if let Some(close) = self.consume_opening() {
            let (unit, unit_start) = self.parse_time_unit_span(Some(close), "interval unit")?;
            self.expect_symbol(close)?;
            (unit, unit_start, false)
        } else {
            let (unit, unit_start) = self.parse_time_unit_span(None, "interval unit")?;
            (unit, unit_start, true)
        };
        // In SQL, bare `INTERVAL DAY` names the day-time interval family;
        // parenthesized `interval(day)` remains the scalar Date32 unit and is
        // rejected below rather than contextually reinterpreted.
        let unit = if sql_style && unit == TimeUnit::Day {
            TimeUnit::DayTime
        } else {
            unit
        };
        if unit.is_interval() {
            Ok(unit)
        } else {
            Err(self.error_at(unit_start, "interval requires an interval layout"))
        }
    }

    pub(crate) fn parse_time_unit_span(
        &mut self,
        close: Option<char>,
        label: &str,
    ) -> Result<(TimeUnit, usize)> {
        let first = self.index;
        let mut end = first;
        while self.tokens.get(end).is_some() {
            if self.is_time_unit_boundary(end, close) {
                break;
            }
            end += 1;
        }
        if first == end {
            return Err(self.error_here(format_smolstr!("expected {label}")));
        }

        let first_token = &self.tokens[first];
        let last_token = &self.tokens[end - 1];
        let (value, source_start, quoted_token) = match &first_token.kind {
            TokenKind::Quoted(value) if end == first + 1 => {
                (value.as_str(), first_token.start + 1, Some(first_token))
            }
            _ => (
                &self.source[first_token.start..last_token.end],
                first_token.start,
                None,
            ),
        };
        let parsed = TimeUnit::from_str(value).map_err(|error| match error {
            Error::Parse {
                position, reason, ..
            } => {
                let position = quoted_token.map_or_else(
                    || source_start.saturating_add(position),
                    |token| self.quoted_source_position(token, position),
                );
                self.error_at(position, reason)
            }
            error => self.error_at(source_start, format_smolstr!("{error}")),
        });
        self.index = end;
        parsed.map(|unit| (unit, source_start))
    }

    pub(crate) fn quoted_source_position(&self, token: &Token, decoded_position: usize) -> usize {
        let Some(quote) = self.source[token.start..].chars().next() else {
            return token.start;
        };
        let mut source_position = token.start.saturating_add(quote.len_utf8());
        let body_end = token.end.saturating_sub(quote.len_utf8());
        let mut decoded_offset = 0_usize;

        while source_position < body_end {
            if decoded_position <= decoded_offset {
                return source_position;
            }
            let logical_start = source_position;
            let Some(character) = self.source[source_position..].chars().next() else {
                return logical_start;
            };
            source_position = source_position.saturating_add(character.len_utf8());

            let decoded_len =
                if character == quote && self.source[source_position..].starts_with(quote) {
                    source_position = source_position.saturating_add(quote.len_utf8());
                    quote.len_utf8()
                } else if character == '\\' {
                    let Some(escaped) = self.source[source_position..].chars().next() else {
                        return logical_start;
                    };
                    source_position = source_position.saturating_add(escaped.len_utf8());
                    if escaped == 'u' {
                        let digits_end = source_position.saturating_add(4);
                        let decoded_len = self
                            .source
                            .get(source_position..digits_end)
                            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                            .and_then(char::from_u32)
                            .map_or(1, char::len_utf8);
                        source_position = digits_end.min(body_end);
                        decoded_len
                    } else {
                        1
                    }
                } else {
                    character.len_utf8()
                };

            if decoded_position < decoded_offset.saturating_add(decoded_len) {
                return logical_start;
            }
            decoded_offset = decoded_offset.saturating_add(decoded_len);
        }
        body_end
    }

    pub(crate) fn is_time_unit_boundary(&self, index: usize, close: Option<char>) -> bool {
        match self.tokens.get(index).map(|token| &token.kind) {
            Some(TokenKind::Symbol(symbol)) => {
                close == Some(*symbol)
                    || matches!(*symbol, ',' | ';')
                    || (close.is_none()
                        && (is_closing_or_separator(*symbol)
                            || matches!(*symbol, '?' | '!')
                            || (*symbol == '['
                                && self
                                    .tokens
                                    .get(index + 1)
                                    .is_some_and(|next| next.kind == TokenKind::Symbol(']')))))
            }
            Some(TokenKind::Word(value)) if close.is_none() => {
                ["not", "required", "null", "nullable"]
                    .iter()
                    .any(|boundary| value.eq_ignore_ascii_case(boundary))
            }
            _ => false,
        }
    }
}
/// Width-accurate temporal values with explicit units and zones.
///
/// ```
/// use yggdryl::{Scalar, TemporalFamily, TimeUnit, Timezone};
///
/// let day = Scalar::from_date(20_000, TimeUnit::Day, Timezone::NAIVE)?;
/// let at = Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC)?;
/// assert_eq!(day.as_date32().map(|(count, ..)| count), Some(20_000));
/// assert_eq!(day.temporal_family(), Some(TemporalFamily::Date));
/// assert_eq!(at.temporal_family(), Some(TemporalFamily::DateTime));
/// assert_eq!(at.temporal_unit(), Some(TimeUnit::Microsecond));
/// assert_eq!(at.temporal_timezone(), Some(Timezone::UTC));
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub(crate) mod scalars {
    use std::fmt;

    use serde::{Deserialize, Serialize};
    use smol_str::{SmolStr, format_smolstr};
    use crate::types::arithmetic::{Arithmetic, invalid_binary};
    use crate::types::decimal::exact_value_parts;
    use crate::types::value::{ValidationFailure, expected};
    use crate::{DataType, Error, Result, Scalar, TimeUnit, Timezone, Value, i256};

    /// Operations shared by every temporal representation.
    pub trait TemporalValue: crate::Value {
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

    fn invalid_temporal_leaf(reason: &'static str) -> Error {
        Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static(reason),
        }
    }

    macro_rules! temporal_leaf {
        ($name:ident, $count:ty, $valid:expr, $reason:literal) => {
            #[doc = concat!("One exact `", stringify!($name), "` count, unit, and timezone.")]
            #[derive(
                Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
            )]
            pub struct $name {
                count: $count,
                unit: TimeUnit,
                timezone: Timezone,
            }

            impl $name {
                /// Validate and construct this exact temporal representation.
                pub fn new(count: $count, unit: TimeUnit, timezone: Timezone) -> Result<Self> {
                    if !$valid(unit, timezone) {
                        return Err(invalid_temporal_leaf($reason));
                    }
                    Ok(Self {
                        count,
                        unit,
                        timezone,
                    })
                }

                /// Return the stored count.
                pub const fn count(&self) -> $count {
                    self.count
                }

                /// Return the count's unit.
                pub const fn unit(&self) -> TimeUnit {
                    self.unit
                }

                /// Return the explicit timezone marker.
                pub const fn timezone(&self) -> Timezone {
                    self.timezone
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    write!(formatter, "{}@{}[{}]", self.count, self.unit, self.timezone)
                }
            }
        };
    }

    temporal_leaf!(
        Date32,
        i32,
        |unit: TimeUnit, timezone: Timezone| unit == TimeUnit::Day && timezone.is_naive(),
        "Date32 requires day units and the NAIVE timezone"
    );
    temporal_leaf!(
        Date64,
        i64,
        |unit: TimeUnit, timezone: Timezone| unit == TimeUnit::Millisecond && timezone.is_naive(),
        "Date64 requires millisecond units and the NAIVE timezone"
    );
    temporal_leaf!(
        Time32,
        i32,
        |unit: TimeUnit, timezone: Timezone| matches!(unit, TimeUnit::Second | TimeUnit::Millisecond)
            && timezone.is_naive(),
        "Time32 requires second or millisecond units and the NAIVE timezone"
    );
    temporal_leaf!(
        Time64,
        i64,
        |unit: TimeUnit, timezone: Timezone| matches!(
            unit,
            TimeUnit::Microsecond | TimeUnit::Nanosecond
        ) && timezone.is_naive(),
        "Time64 requires microsecond or nanosecond units and the NAIVE timezone"
    );
    temporal_leaf!(
        DateTime64,
        i64,
        |unit: TimeUnit, _timezone: Timezone| unit.is_arrow_time(),
        "DateTime64 requires an Arrow clock resolution"
    );
    temporal_leaf!(
        Duration32,
        i32,
        |unit: TimeUnit, timezone: Timezone| unit.is_temporal() && timezone.is_naive(),
        "Duration32 requires a fixed temporal unit and the NAIVE timezone"
    );
    temporal_leaf!(
        Duration64,
        i64,
        |unit: TimeUnit, timezone: Timezone| unit.is_temporal() && timezone.is_naive(),
        "Duration64 requires a fixed temporal unit and the NAIVE timezone"
    );

    const _: () = assert!(std::mem::size_of::<DateTime64>() == 16);

    /// One Arrow interval represented without losing any of its three layouts.
    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
    pub struct Interval {
        months: i32,
        days: i32,
        nanoseconds: i64,
        unit: TimeUnit,
    }

    impl<'de> Deserialize<'de> for Interval {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            #[derive(Deserialize)]
            struct Wire {
                months: i32,
                days: i32,
                nanoseconds: i64,
                unit: TimeUnit,
            }

            let value = Wire::deserialize(deserializer)?;
            Self::new(value.months, value.days, value.nanoseconds, value.unit)
                .map_err(serde::de::Error::custom)
        }
    }

    impl Interval {
        /// Construct an interval, rejecting fields the selected layout cannot hold.
        pub fn new(months: i32, days: i32, nanoseconds: i64, unit: TimeUnit) -> Result<Self> {
            let valid = match unit {
                TimeUnit::YearMonth => days == 0 && nanoseconds == 0,
                TimeUnit::DayTime => {
                    months == 0
                        && nanoseconds % 1_000_000 == 0
                        && i32::try_from(nanoseconds / 1_000_000).is_ok()
                }
                TimeUnit::MonthDayNano => true,
                _ => false,
            };
            if !valid {
                return Err(invalid_temporal_leaf(
                    "Interval components do not fit the selected layout",
                ));
            }
            Ok(Self {
                months,
                days,
                nanoseconds,
                unit,
            })
        }

        /// Return the month component.
        pub const fn months(&self) -> i32 {
            self.months
        }

        /// Return the day component.
        pub const fn days(&self) -> i32 {
            self.days
        }

        /// Return the nanosecond component.
        pub const fn nanoseconds(&self) -> i64 {
            self.nanoseconds
        }

        /// Return the physical interval layout.
        pub const fn unit(&self) -> TimeUnit {
            self.unit
        }
    }

    impl fmt::Display for Interval {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "{}mo:{}d:{}ns@{}",
                self.months, self.days, self.nanoseconds, self.unit
            )
        }
    }

    macro_rules! temporal_value {
        ($leaf:ident, $id:ident) => {
            impl Value for $leaf {
                fn dtype(&self) -> Result<DataType> {
                    Scalar::$id(*self).dtype()
                }

                fn into_scalar(self) -> Scalar {
                    Scalar::$id(self)
                }

                fn from_scalar(value: &Scalar) -> Option<&Self> {
                    match value {
                        Scalar::$id(value) => Some(value),
                        _ => None,
                    }
                }
            }
        };
        ($leaf:ident, $family:ident, $bits:literal, $count:ty) => {
            temporal_value!($leaf, $leaf);

            impl TemporalValue for $leaf {
                const FAMILY: TemporalFamily = TemporalFamily::$family;
                const BIT_WIDTH: u8 = $bits;

                fn count(&self) -> i64 {
                    i64::from(self.count())
                }

                fn unit(&self) -> TimeUnit {
                    self.unit()
                }

                fn timezone(&self) -> Timezone {
                    self.timezone()
                }

                fn with_unit(self, unit: TimeUnit) -> Result<Self> {
                    let value = Scalar::$leaf(self);
                    let count = value.temporal_count_at(unit).ok_or_else(|| {
                        invalid_temporal_leaf("temporal unit conversion is not exact")
                    })?;
                    let count = <$count>::try_from(count).map_err(|_| {
                        invalid_temporal_leaf("converted temporal count exceeds its physical width")
                    })?;
                    Self::new(count, unit, self.timezone())
                }

                fn with_timezone(self, timezone: Timezone) -> Result<Self> {
                    Self::new(self.count(), self.unit(), timezone)
                }
            }
        };
    }

    // The leaf, its `Scalar` variant and its `DataTypeId` share one name.
    temporal_value!(Date32, Date, 32, i32);
    temporal_value!(Date64, Date, 64, i64);
    temporal_value!(Time32, Time, 32, i32);
    temporal_value!(Time64, Time, 64, i64);
    temporal_value!(DateTime64, DateTime, 64, i64);
    temporal_value!(Duration32, Duration, 32, i32);
    temporal_value!(Duration64, Duration, 64, i64);
    temporal_value!(Interval, Interval);

    impl TemporalValue for Interval {
        const FAMILY: TemporalFamily = TemporalFamily::Interval;
        const BIT_WIDTH: u8 = 128;

        fn count(&self) -> i64 {
            self.nanoseconds()
        }

        fn unit(&self) -> TimeUnit {
            self.unit()
        }

        fn timezone(&self) -> Timezone {
            Timezone::NAIVE
        }

        fn with_unit(self, unit: TimeUnit) -> Result<Self> {
            Self::new(self.months(), self.days(), self.nanoseconds(), unit)
        }

        fn with_timezone(self, timezone: Timezone) -> Result<Self> {
            if timezone.is_naive() {
                Ok(self)
            } else {
                Err(invalid_temporal_leaf(
                    "Interval requires the NAIVE timezone",
                ))
            }
        }
    }

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
        /// Calendar intervals.
        Interval,
    }

    impl TemporalFamily {
        /// Return the canonical family name.
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Date => "date",
                Self::Time => "time",
                Self::DateTime => "datetime",
                Self::Duration => "duration",
                Self::Interval => "interval",
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
            Self::Date32(Date32 {
                count: days,
                unit: TimeUnit::Day,
                timezone: Timezone::NAIVE,
            })
        }

        /// Build a Date32 after validating its unit and zone.
        pub fn date32_in(days: i32, unit: TimeUnit, zone: Timezone) -> Result<Self> {
            require(unit == TimeUnit::Day, "date32 unit must be day")?;
            require(zone.is_naive(), "date32 timezone must be NAIVE")?;
            Date32::new(days, unit, zone).map(Self::Date32)
        }

        /// Build a Date64 millisecond count.
        pub const fn date64(milliseconds: i64) -> Self {
            Self::Date64(Date64 {
                count: milliseconds,
                unit: TimeUnit::Millisecond,
                timezone: Timezone::NAIVE,
            })
        }

        /// Build a Date64 after validating its unit and zone.
        pub fn date64_in(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
            require(
                unit == TimeUnit::Millisecond,
                "date64 unit must be millisecond",
            )?;
            require(zone.is_naive(), "date64 timezone must be NAIVE")?;
            Date64::new(count, unit, zone).map(Self::Date64)
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
            Time32::new(count, unit, zone).map(Self::Time32)
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
            Time64::new(count, unit, zone).map(Self::Time64)
        }

        /// Build an instant or wall-clock datetime at 64-bit width.
        pub fn datetime64(count: i64, unit: TimeUnit, zone: Timezone) -> Result<Self> {
            require(
                unit.is_arrow_time(),
                "datetime64 requires an Arrow time unit",
            )?;
            DateTime64::new(count, unit, zone).map(Self::DateTime64)
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
            Duration32::new(count, unit, zone).map(Self::Duration32)
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
            Duration64::new(count, unit, zone).map(Self::Duration64)
        }

        /// Return Date32's count, unit, and zone.
        pub const fn as_date32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
            match self {
                Self::Date32(value) => Some((value.count(), value.unit(), &value.timezone)),
                _ => None,
            }
        }

        /// Return Date64's count, unit, and zone.
        pub const fn as_date64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
            match self {
                Self::Date64(value) => Some((value.count(), value.unit(), &value.timezone)),
                _ => None,
            }
        }

        /// Return Time32's count, unit, and zone.
        pub const fn as_time32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
            match self {
                Self::Time32(value) => Some((value.count(), value.unit(), &value.timezone)),
                _ => None,
            }
        }

        /// Return Time64's count, unit, and zone.
        pub const fn as_time64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
            match self {
                Self::Time64(value) => Some((value.count(), value.unit(), &value.timezone)),
                _ => None,
            }
        }

        /// Return DateTime64's count, unit, and zone.
        pub const fn as_datetime64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
            match self {
                Self::DateTime64(value) => Some((value.count(), value.unit(), &value.timezone)),
                _ => None,
            }
        }

        /// Return Duration32's count, unit, and zone.
        pub const fn as_duration32(&self) -> Option<(i32, TimeUnit, &Timezone)> {
            match self {
                Self::Duration32(value) => Some((value.count(), value.unit(), &value.timezone)),
                _ => None,
            }
        }

        /// Return Duration64's count, unit, and zone.
        pub const fn as_duration64(&self) -> Option<(i64, TimeUnit, &Timezone)> {
            match self {
                Self::Duration64(value) => Some((value.count(), value.unit(), &value.timezone)),
                _ => None,
            }
        }

        /// Return the logical family of any temporal, or `None` for a
        /// non-temporal.
        pub const fn temporal_family(&self) -> Option<TemporalFamily> {
            match self {
                Self::Date32(_) | Self::Date64(_) => Some(TemporalFamily::Date),
                Self::Time32(_) | Self::Time64(_) => Some(TemporalFamily::Time),
                Self::DateTime64(_) => Some(TemporalFamily::DateTime),
                Self::Duration32(_) | Self::Duration64(_) => Some(TemporalFamily::Duration),
                Self::Interval(_) => Some(TemporalFamily::Interval),
                _ => None,
            }
        }

        /// Return the physical unit of any temporal, or an interval's layout;
        /// `None` for a non-temporal.
        pub const fn temporal_unit(&self) -> Option<TimeUnit> {
            match self {
                Self::Date32(value) => Some(value.unit()),
                Self::Date64(value) => Some(value.unit()),
                Self::Time32(value) => Some(value.unit()),
                Self::Time64(value) => Some(value.unit()),
                Self::DateTime64(value) => Some(value.unit()),
                Self::Duration32(value) => Some(value.unit()),
                Self::Duration64(value) => Some(value.unit()),
                Self::Interval(value) => Some(value.unit()),
                _ => None,
            }
        }

        /// Return the non-optional timezone carried by any temporal, with an
        /// interval explicitly zone-free; `None` for a non-temporal.
        pub const fn temporal_timezone(&self) -> Option<Timezone> {
            match self {
                Self::Date32(value) => Some(value.timezone()),
                Self::Date64(value) => Some(value.timezone()),
                Self::Time32(value) => Some(value.timezone()),
                Self::Time64(value) => Some(value.timezone()),
                Self::DateTime64(value) => Some(value.timezone()),
                Self::Duration32(value) => Some(value.timezone()),
                Self::Duration64(value) => Some(value.timezone()),
                Self::Interval(_) => Some(Timezone::NAIVE),
                _ => None,
            }
        }

        /// Return the stored count of any temporal widened to 64 bits, or `None`
        /// for a non-temporal.
        ///
        /// For an interval this is its nanosecond component; callers that need all
        /// three interval components match [`Scalar::Interval`] directly.
        pub const fn temporal_count(&self) -> Option<i64> {
            match self {
                Self::Date32(value) => Some(value.count() as i64),
                Self::Date64(value) => Some(value.count()),
                Self::Time32(value) => Some(value.count() as i64),
                Self::Time64(value) => Some(value.count()),
                Self::DateTime64(value) => Some(value.count()),
                Self::Duration32(value) => Some(value.count() as i64),
                Self::Duration64(value) => Some(value.count()),
                Self::Interval(value) => Some(value.nanoseconds()),
                _ => None,
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
                DataType::Date32 => Ok(Self::date32(crate::types::temporal::parse_date(text)?)),
                DataType::Date64 => i64::from(crate::types::temporal::parse_date(text)?)
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
                    let count = restated(crate::types::temporal::parse_datetime(text)?, *unit, "datetime64")?;
                    Self::datetime64(count, *unit, Timezone::NAIVE)
                }
                DataType::DateTime64 { unit, timezone } => {
                    let (count, source, _) = crate::types::temporal::parse_timestamp(text)?;
                    let count = restated((count, source), *unit, "datetime64")?;
                    Self::datetime64(count, *unit, *timezone)
                }
                DataType::Duration32(unit) => {
                    let count = restated(crate::types::temporal::parse_duration(text)?, *unit, "duration32")?;
                    Self::duration32(narrow_i32(count, "duration32")?, *unit)
                }
                DataType::Duration64(unit) => {
                    let count = restated(crate::types::temporal::parse_duration(text)?, *unit, "duration64")?;
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
                Self::Date32(value) => crate::types::temporal::format_date(value.count()),
                Self::Date64(value) => i32::try_from(value.count().div_euclid(86_400_000))
                    .ok()
                    .and_then(crate::types::temporal::format_date),
                Self::Time32(value) => crate::types::temporal::format_time(i64::from(value.count()), value.unit()),
                Self::Time64(value) => crate::types::temporal::format_time(value.count(), value.unit()),
                Self::DateTime64(value) if value.timezone().is_naive() => {
                    crate::types::temporal::format_datetime(value.count(), value.unit())
                }
                Self::DateTime64(value) => {
                    crate::types::temporal::format_timestamp(value.count(), value.unit(), &value.timezone())
                }
                Self::Duration32(value) => crate::types::temporal::format_duration(i64::from(value.count()), value.unit()),
                Self::Duration64(value) => crate::types::temporal::format_duration(value.count(), value.unit()),
                _ => None,
            }
        }

        /// Return this temporal's count restated in `unit`, when exact.
        pub fn temporal_count_at(&self, unit: TimeUnit) -> Option<i64> {
            if matches!(self, Self::Interval(_)) {
                return None;
            }
            let (count, current) = (self.temporal_count()?, self.temporal_unit()?);
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
            self.temporal_family().is_some()
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
        crate::types::temporal::parse_time(text)
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

    /// Nanoseconds in one count of a fixed-length unit; an interval layout has none.
    pub(crate) const fn nanoseconds_per(unit: TimeUnit) -> Option<i128> {
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

    #[derive(Clone)]
    pub(crate) struct TemporalParts {
        pub(crate) family: TemporalFamily,
        pub(crate) unit: TimeUnit,
        pub(crate) zone: Timezone,
        pub(crate) dtype: DataType,
    }

    pub(crate) fn temporal_value_parts(value: &Scalar) -> Option<TemporalParts> {
        let family = value.temporal_family()?;
        let unit = value.temporal_unit()?;
        let zone = value.temporal_timezone()?;
        let dtype = match value {
            Scalar::Date32(_) => DataType::Date32,
            Scalar::Date64(_) => DataType::Date64,
            Scalar::Time32(_) => DataType::Time32(unit),
            Scalar::Time64(_) => DataType::Time64(unit),
            Scalar::DateTime64(_) => DataType::DateTime64 {
                unit,
                timezone: zone,
            },
            Scalar::Duration32(_) => DataType::Duration32(unit),
            Scalar::Duration64(_) => DataType::Duration64(unit),
            _ => return None,
        };
        Some(TemporalParts {
            family,
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
            && right.is_integer()
        {
            (left, right, true)
        } else if left.is_integer()
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
            .map(|value| i256::from_i128(i128::from(value)))
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
        .and_then(i256::as_i128)
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
}
