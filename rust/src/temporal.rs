//! What the five temporal families share, and nothing any one of them owns.
//!
//! A date, a time of day, a datetime, a duration and an interval are five
//! families - [`crate::date`], [`crate::time`], [`crate::datetime`],
//! [`crate::duration`] and [`crate::interval`] - each holding its own datatype
//! payload, its `DataType` constructors and its values, with its `Field` leaf
//! declared beside the other families' in `field.rs`. Which one a value, a
//! datatype or a column belongs to is its identifier's
//! [`temporal_family`](crate::DataTypeId::temporal_family). This file holds
//! what several of them read: the `temporal_leaf!` macro the family files
//! build their count-unit-zone values with; the unit validators the
//! constructors call; the classic ISO 8601 spellings every reader and writer
//! of temporal text goes through; and the `Scalar` accessors that read across
//! the families - [`crate::Scalar::temporal_unit`],
//! [`crate::Scalar::temporal_count`] - with the arithmetic that uses them.
//! The contract every temporal value answers,
//! [`TemporalValue`](crate::TemporalValue), lives with the other value
//! contracts in `value/`.
//!
//! ```
//! use yggdryl::{DataType, DataTypeId, DataTypeKind, Scalar, TimeUnit, Timezone};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // A value knows its family and its unit whichever width holds it.
//! let day = Scalar::from_date(20_000, TimeUnit::Day, Timezone::NAIVE)?;
//! let at = Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC)?;
//! assert_eq!(day.id().temporal_family(), Some("date"));
//! assert!(DataTypeKind::Temporal.contains(at.id()));
//! assert!(matches!(at, Scalar::DateTime64(_)));
//! assert_eq!(at.temporal_unit(), Some(TimeUnit::Microsecond));
//! assert_eq!(at.temporal_timezone(), Some(Timezone::UTC));
//!
//! // So does a datatype: the family is its identifier's.
//! assert_eq!(DataType::date32().id().temporal_family(), Some("date"));
//! assert_eq!(DataTypeId::Interval.temporal_family(), Some("interval"));
//! assert_eq!(DataType::time(TimeUnit::Second)?, DataType::Time32(TimeUnit::Second));
//! # Ok(())
//! # }
//! ```

pub(crate) use scalars::TemporalKind;
pub(crate) use scalars::{validate_date64, validate_time};
use smol_str::{SmolStr, ToSmolStr, format_smolstr};

use crate::invalid;
use crate::parser::{Parser, Token, TokenKind, is_closing_or_separator};
use crate::timezone::{civil_from_days, days_from_civil};
use crate::{Error, Result, TimeUnit, Timezone};

/// Arrow casts every temporal family shares: they take any temporal.
pub(crate) mod casts {

    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BooleanArray, StringArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
    use arrow_cast::can_cast_types;
    use arrow_schema::DataType as ArrowDataType;
    use arrow_select::zip::zip;

    use crate::arrow::{Error, Result};
    use crate::budget::{MaterializationBudget, reserve_vec_bytes};
    use crate::cast::arrow_cast_exposed;
    use crate::cast::columns::is_exposed;
    use crate::cast::text::ingest_text_values;
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
        let source_type = DataType::from_arrow_datatype(array.data_type())?;
        let rows = array.len();
        budget.add_array(field.dtype(), rows)?;
        reserve_vec_bytes::<Option<smol_str::SmolStr>>(budget, rows)?;
        let mut spelled = Vec::with_capacity(rows);
        let mut ours = BooleanBufferBuilder::new(rows);
        let mut unspelled = false;
        for index in 0..rows {
            let text = if is_exposed(exposure, index) && array.is_valid(index) {
                crate::serie::value::value_from_array(&source_type, array.as_ref(), index)?
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
    ///
    /// An interval is temporal too and has no classic spelling, so it is not
    /// a target the text readers answer for.
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
// The unit rules the family constructors and their Arrow boundaries share.
// ------------------------------------------------------------------------

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

/// The refusal a temporal value answers for a count, unit or zone its
/// width cannot hold: a value problem, so an invalid record rather than an
/// invalid datatype. The reason is a literal, so nothing allocates.
pub(crate) fn invalid_record(reason: &'static str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: SmolStr::new_static(reason),
    }
}

/// [`invalid_record`] for a reason that names the value or the unit it
/// refused, and so is written at the refusal.
pub(crate) fn invalid_record_text(reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason,
    }
}

// ------------------------------------------------------------------------
// The count-unit-zone value every family but the interval builds.
// ------------------------------------------------------------------------

/// Emit one temporal value: a count at one width, its unit and its zone,
/// with its [`crate::Value`] and [`TemporalValue`] impls.
///
/// The family files invoke this, so every path inside is absolute: `$valid`
/// is the family's rule over the unit and the zone, `$reason` the refusal
/// it states, and `$dtype` reads the family datatype back off a value.
macro_rules! temporal_leaf {
    (
        $name:ident, $count:ty, $bits:literal,
        valid = $valid:expr,
        dtype = $dtype:expr,
        $reason:literal $(,)?
    ) => {
        #[doc = concat!("One exact `", stringify!($name), "` count, unit, and timezone.")]
        #[derive(
            Clone,
            Copy,
            Debug,
            ::serde::Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            ::serde::Serialize,
        )]
        pub struct $name {
            count: $count,
            unit: $crate::TimeUnit,
            timezone: $crate::Timezone,
        }

        impl $name {
            /// Validate and construct this exact temporal representation.
            ///
            /// # Errors
            ///
            /// Returns [`crate::Error::InvalidRecord`] for a unit or a zone
            /// this width does not carry.
            pub fn new(
                count: $count,
                unit: $crate::TimeUnit,
                timezone: $crate::Timezone,
            ) -> $crate::Result<Self> {
                if !$valid(unit, timezone) {
                    return Err($crate::temporal::invalid_record($reason));
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
            pub const fn unit(&self) -> $crate::TimeUnit {
                self.unit
            }

            /// Return the explicit timezone marker.
            pub const fn timezone(&self) -> $crate::Timezone {
                self.timezone
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(formatter, "{}@{}[{}]", self.count, self.unit, self.timezone)
            }
        }

        // The leaf, its `Scalar` variant and its `DataTypeId` share one name.
        impl $crate::Value for $name {
            fn dtype(&self) -> $crate::Result<$crate::DataType> {
                $dtype(self)
            }

            fn into_scalar(self) -> $crate::Scalar {
                $crate::Scalar::$name(self)
            }

            fn from_scalar(value: &$crate::Scalar) -> Option<&Self> {
                match value {
                    $crate::Scalar::$name(value) => Some(value),
                    _ => None,
                }
            }
        }

        impl $crate::value::TemporalValue for $name {
            const BIT_WIDTH: u8 = $bits;

            fn count(&self) -> i64 {
                i64::from(self.count())
            }

            fn unit(&self) -> $crate::TimeUnit {
                self.unit()
            }

            fn timezone(&self) -> $crate::Timezone {
                self.timezone()
            }

            fn with_unit(self, unit: $crate::TimeUnit) -> $crate::Result<Self> {
                let value = $crate::Scalar::$name(self);
                let count = value.temporal_count_at(unit).ok_or_else(|| {
                    $crate::temporal::invalid_record("temporal unit conversion is not exact")
                })?;
                let count = <$count>::try_from(count).map_err(|_| {
                    $crate::temporal::invalid_record(
                        "converted temporal count exceeds its physical width",
                    )
                })?;
                Self::new(count, unit, self.timezone())
            }

            fn with_timezone(self, timezone: $crate::Timezone) -> $crate::Result<Self> {
                Self::new(self.count(), self.unit(), timezone)
            }
        }
    };
}

pub(crate) use temporal_leaf;

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
///
/// `_` groups digits and does not open a fraction: `00:05:01_147` is a clock
/// of seconds followed by text, not a clock of milliseconds. The two signs
/// that open one are the two ISO 31-0 names for the decimal sign, and a
/// group separator is not a third - one character cannot mean "the fraction
/// starts here" and "these digits are grouped" in one token. An emitter that
/// divides its fraction with `_` is writing no ISO clock, and a reader of it
/// states the shape in its own row header rather than here.
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

// ------------------------------------------------------------------------
// The unit grammar every temporal spelling reads through; each family's
// own keyword arm lives in its file.
// ------------------------------------------------------------------------

impl Parser<'_> {
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

/// What every temporal value answers, and the readings that cross the
/// families: the family, the unit, the zone and the count of any temporal,
/// its classic text both ways, and the arithmetic over two of them.
pub(crate) mod scalars {
    use crate::arithmetic::{Arithmetic, invalid_binary};

    use crate::decimal::exact_value_parts;

    use crate::value::{ValidationFailure, expected};
    use crate::{DataType, Error, Result, Scalar, TimeUnit, Timezone, i256};
    use smol_str::format_smolstr;

    use super::{invalid_record, invalid_record_text};

    /// One logical temporal family, independent of its physical width: the
    /// crate's own five-way tag, which the arithmetic, the digests and the
    /// canonicalization branch on. A caller reads the name off
    /// [`DataTypeId::temporal_family`](crate::DataTypeId::temporal_family);
    /// there is no public tag.
    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub(crate) enum TemporalKind {
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

    impl TemporalKind {
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
        /// The crate's five-way tag of any temporal, or `None` for a
        /// non-temporal.
        pub(crate) const fn temporal_kind(&self) -> Option<TemporalKind> {
            self.id().temporal_kind()
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
        /// `NAIVE` datetime refuses one. An interval has no classic spelling, so
        /// it is no target here.
        ///
        /// # Errors
        ///
        /// Returns the parse error the spelling raised, or an invalid-value error
        /// when the count does not fit the declared unit and width.
        pub(crate) fn from_temporal_text(dtype: &DataType, text: &str) -> Result<Self> {
            match dtype {
                DataType::Date32 => Ok(Self::date32(super::parse_date(text)?)),
                DataType::Date64 => i64::from(super::parse_date(text)?)
                    .checked_mul(86_400_000)
                    .map(Self::date64)
                    .ok_or_else(|| invalid_record("date64 count must fit signed 64 bits")),
                DataType::Time32(unit) => {
                    let count = restated(clock_of_day(text)?, *unit, "time32")?;
                    Self::time32(narrow_i32(count, "time32")?, *unit, Timezone::NAIVE)
                }
                DataType::Time64(unit) => {
                    let count = restated(clock_of_day(text)?, *unit, "time64")?;
                    Self::time64(count, *unit, Timezone::NAIVE)
                }
                DataType::DateTime64 { unit, timezone } if timezone.is_naive() => {
                    let count = restated(super::parse_datetime(text)?, *unit, "datetime64")?;
                    Self::datetime64(count, *unit, Timezone::NAIVE)
                }
                DataType::DateTime64 { unit, timezone } => {
                    let (count, source, _) = super::parse_timestamp(text)?;
                    let count = restated((count, source), *unit, "datetime64")?;
                    Self::datetime64(count, *unit, *timezone)
                }
                DataType::Duration32(unit) => {
                    let count = restated(super::parse_duration(text)?, *unit, "duration32")?;
                    Self::duration32(narrow_i32(count, "duration32")?, *unit)
                }
                DataType::Duration64(unit) => {
                    let count = restated(super::parse_duration(text)?, *unit, "duration64")?;
                    Self::duration64(count, *unit)
                }
                other => Err(invalid_record_text(format_smolstr!(
                    "{other} holds no temporal text"
                ))),
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
                Self::Date32(value) => crate::temporal::format_date(value.count()),
                Self::Date64(value) => i32::try_from(value.count().div_euclid(86_400_000))
                    .ok()
                    .and_then(crate::temporal::format_date),
                Self::Time32(value) => {
                    crate::temporal::format_time(i64::from(value.count()), value.unit())
                }
                Self::Time64(value) => crate::temporal::format_time(value.count(), value.unit()),
                Self::DateTime64(value) if value.timezone().is_naive() => {
                    crate::temporal::format_datetime(value.count(), value.unit())
                }
                Self::DateTime64(value) => crate::temporal::format_timestamp(
                    value.count(),
                    value.unit(),
                    &value.timezone(),
                ),
                Self::Duration32(value) => {
                    crate::temporal::format_duration(i64::from(value.count()), value.unit())
                }
                Self::Duration64(value) => {
                    crate::temporal::format_duration(value.count(), value.unit())
                }
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
            crate::DataTypeKind::Temporal.contains(self.id())
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
            return Err(invalid_record(
                "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
            ));
        }
        crate::temporal::parse_time(text)
    }

    /// Restate a parsed count in the unit its datatype declares, when exact.
    fn restated(
        (count, source): (i64, TimeUnit),
        unit: TimeUnit,
        kind: &'static str,
    ) -> Result<i64> {
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
        restate(i128::from(count))
            .ok_or_else(|| invalid_record_text(format_smolstr!("{kind} count is no exact {unit}")))
    }

    /// Narrow a count to the 32-bit widths, naming the width that refused it.
    pub(crate) fn narrow_i32(count: i64, kind: &'static str) -> Result<i32> {
        i32::try_from(count).map_err(|_| {
            invalid_record_text(format_smolstr!("{kind} count must fit signed 32 bits"))
        })
    }

    /// The one refusal a value constructor states for a unit or zone its
    /// width does not carry.
    pub(crate) fn require(valid: bool, reason: &'static str) -> Result<()> {
        if valid {
            Ok(())
        } else {
            Err(invalid_record(reason))
        }
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
        pub(crate) family: TemporalKind,
        pub(crate) unit: TimeUnit,
        pub(crate) zone: Timezone,
        pub(crate) dtype: DataType,
    }

    /// The family, unit, zone and datatype of one temporal value; an interval
    /// takes no arithmetic and answers `None`.
    pub(crate) fn temporal_value_parts(value: &Scalar) -> Option<TemporalParts> {
        let family = value.temporal_kind()?;
        let unit = value.temporal_unit()?;
        let zone = value.temporal_timezone()?;
        // The value already proved its unit and zone, so the datatype is
        // written as it is rather than checked again.
        let dtype = match value {
            Scalar::Date32(_) => DataType::date32(),
            Scalar::Date64(_) => DataType::date64(),
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

    pub(crate) fn temporal_target(dtype: &DataType) -> Option<(TemporalKind, TimeUnit)> {
        match dtype {
            leaf_dtype @ (DataType::Date32 | DataType::Date64) => {
                let leaf = &leaf_dtype
                    .date_type()
                    .expect("the variant was just matched");
                Some((TemporalKind::Date, leaf.unit()))
            }
            leaf_dtype @ (DataType::Time32(_) | DataType::Time64(_)) => {
                let leaf = &leaf_dtype
                    .time_type()
                    .expect("the variant was just matched");
                Some((TemporalKind::Time, leaf.unit()))
            }
            leaf_dtype @ DataType::DateTime64 { .. } => {
                let leaf = &leaf_dtype
                    .datetime_type()
                    .expect("the variant was just matched");
                Some((TemporalKind::DateTime, leaf.unit()))
            }
            leaf_dtype @ (DataType::Duration32(_) | DataType::Duration64(_)) => {
                let leaf = &leaf_dtype
                    .duration_type()
                    .expect("the variant was just matched");
                Some((TemporalKind::Duration, leaf.unit()))
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
            (family, TemporalKind::Duration, Arithmetic::Add | Arithmetic::Sub)
                if family != TemporalKind::Duration =>
            {
                Ok(left_parts.dtype)
            }
            (TemporalKind::Duration, family, Arithmetic::Add)
                if family != TemporalKind::Duration =>
            {
                Ok(right_parts.dtype)
            }
            (family, other, Arithmetic::Sub)
                if family == other && family != TemporalKind::Duration =>
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
            (TemporalKind::Duration, TemporalKind::Duration, Arithmetic::Add | Arithmetic::Sub) => {
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
        let left_parts = temporal_value_parts(left).ok_or_else(|| {
            invalid_binary(operation, left, right, "left operand is not temporal")
        })?;
        let right_parts = temporal_value_parts(right).ok_or_else(|| {
            invalid_binary(operation, left, right, "right operand is not temporal")
        })?;
        let (target_family, unit) = temporal_target(target).ok_or_else(|| {
            invalid_binary(
                operation,
                left,
                right,
                "the promoted datatype is not temporal",
            )
        })?;
        let (left_count, right_count) = match (left_parts.family, right_parts.family, operation) {
            (family, TemporalKind::Duration, Arithmetic::Add | Arithmetic::Sub)
                if family == target_family =>
            {
                (
                    temporal_at(left, unit, operation, right)?,
                    temporal_at(right, unit, operation, left)?,
                )
            }
            (TemporalKind::Duration, family, Arithmetic::Add) if family == target_family => (
                temporal_at(right, unit, operation, left)?,
                temporal_at(left, unit, operation, right)?,
            ),
            (family, other, Arithmetic::Sub)
                if family == other
                    && target_family == TemporalKind::Duration
                    && family != TemporalKind::Duration =>
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
            (TemporalKind::Duration, TemporalKind::Duration, Arithmetic::Add | Arithmetic::Sub)
                if target_family == TemporalKind::Duration =>
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
            .is_some_and(|parts| parts.family == TemporalKind::Duration)
            && right.is_integer()
        {
            (left, right, true)
        } else if left.is_integer()
            && temporal_value_parts(right)
                .is_some_and(|parts| parts.family == TemporalKind::Duration)
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
        let narrowed = |kind: &'static str| {
            i32::try_from(count).map_err(|_| Error::ArithmeticOverflow {
                operation: "temporal arithmetic",
                kind,
            })
        };
        match dtype {
            DataType::Date32 => Scalar::date32_in(narrowed("date32")?, unit, Timezone::NAIVE),
            DataType::Date64 => Scalar::date64_in(count, unit, Timezone::NAIVE),
            DataType::Time32(expected) if *expected == unit => {
                Scalar::time32(narrowed("time32")?, unit, Timezone::NAIVE)
            }
            DataType::Time64(expected) if *expected == unit => {
                Scalar::time64(count, unit, Timezone::NAIVE)
            }
            DataType::DateTime64 {
                unit: expected,
                timezone,
            } if *expected == unit => Scalar::datetime64(count, unit, *timezone),
            DataType::Duration32(expected) if *expected == unit => {
                Scalar::duration32(narrowed("duration32")?, unit)
            }
            DataType::Duration64(expected) if *expected == unit => Scalar::duration64(count, unit),
            _ => Err(Error::InvalidArithmetic {
                operation: "temporal arithmetic",
                left: temporal_kind_name(dtype),
                right: None,
                reason: "invalid result unit".into(),
            }),
        }
    }

    /// The name a refusal states a temporal result under: the leaf's own,
    /// or `temporal` for a datatype the arithmetic does not answer.
    const fn temporal_kind_name(dtype: &DataType) -> &'static str {
        match dtype {
            DataType::Date32
            | DataType::Date64
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::DateTime64 { .. }
            | DataType::Duration32(_)
            | DataType::Duration64(_) => dtype.name(),
            _ => "temporal",
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/temporal.rs` pins and a caller cannot reach.
    //!
    //! The ISO 8601 readers and writers are crate-private: a caller reaches
    //! them only through a datatype and a `Scalar`, so what each spelling
    //! reads to - the unit a fraction's digit count names, the hour that
    //! carries into the next day, the byte a refusal points at - is only
    //! observable from the reader itself. Each is behind a forwarder, so
    //! nothing here is more public than it was.

    use smol_str::SmolStr;

    use crate::{Result, TimeUnit, Timezone};

    /// Write a day count as a calendar date.
    #[must_use]
    pub fn format_date(days: i32) -> Option<SmolStr> {
        super::format_date(days)
    }

    /// Write a count of `unit` as a time of day.
    #[must_use]
    pub fn format_time(count: i64, unit: TimeUnit) -> Option<SmolStr> {
        super::format_time(count, unit)
    }

    /// Write a count of `unit` as a naive datetime.
    #[must_use]
    pub fn format_datetime(count: i64, unit: TimeUnit) -> Option<SmolStr> {
        super::format_datetime(count, unit)
    }

    /// Write a count of `unit` as a zoned timestamp.
    #[must_use]
    pub fn format_timestamp(count: i64, unit: TimeUnit, zone: &Timezone) -> Option<SmolStr> {
        super::format_timestamp(count, unit, zone)
    }

    /// Write a count of `unit` as a duration.
    #[must_use]
    pub fn format_duration(count: i64, unit: TimeUnit) -> Option<SmolStr> {
        super::format_duration(count, unit)
    }

    /// Read a calendar date as a day count.
    pub fn parse_date(text: &str) -> Result<i32> {
        super::parse_date(text)
    }

    /// Read a time of day as a count and the unit its digits name.
    pub fn parse_time(text: &str) -> Result<(i64, TimeUnit)> {
        super::parse_time(text)
    }

    /// Read a naive datetime as a count and the unit its digits name.
    pub fn parse_datetime(text: &str) -> Result<(i64, TimeUnit)> {
        super::parse_datetime(text)
    }

    /// Read a zoned timestamp as a count, a unit, and the zone it states.
    pub fn parse_timestamp(text: &str) -> Result<(i64, TimeUnit, Timezone)> {
        super::parse_timestamp(text)
    }

    /// Read a duration as a count and the unit its digits name.
    pub fn parse_duration(text: &str) -> Result<(i64, TimeUnit)> {
        super::parse_duration(text)
    }

    /// Read `text` as the temporal `dtype` declares.
    pub fn from_temporal_text(dtype: &crate::DataType, text: &str) -> Result<crate::Scalar> {
        crate::Scalar::from_temporal_text(dtype, text)
    }
}
