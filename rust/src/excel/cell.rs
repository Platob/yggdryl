//! One cell of a worksheet: where it is, what the file says about it, and
//! the one value that says.
//!
//! A `<c>` element states a reference (`r="B2"`), a type (`t`), a style
//! (`s`) and its content - a `<v>` value, an `<is>` inline string, an `<f>`
//! formula. Reading resolves those facts once into a [`Cell`]: the shared
//! string an `s` cell indexes is its text, the style is its [`StyleId`] and
//! the [`NumberFormat`] its `numFmtId` classifies as, the formula is one
//! shared [`Formula`], an error is its [`ExcelError`], and the number, the
//! text, the boolean or the date the content spells is the cell's one
//! [`Scalar`].
//! A cell built from a value states the facts that value spells, so a
//! [`Sheet`](super::Sheet) writes it back without deciding anything again.
//!
//! Excel stores a date, a time, a datetime and a duration as one number: the
//! days since the workbook's epoch, the clock as a fraction of a day. The
//! [`DateSystem`] is the one owner of that rule, both ways, including the
//! 1900 system's phantom 1900-02-29 (serial 60).

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, Result, Scalar, Str, TemporalKind, TimeUnit, Timezone};

use super::formula::Formula;
use super::style::StyleId;
use super::styles::NumberFormat;

/// The most rows a worksheet holds: `1_048_576`.
pub const MAX_ROWS: u32 = 1_048_576;

/// The most columns a worksheet holds: `16_384`, the last spelled `XFD`.
pub const MAX_COLUMNS: u32 = 16_384;

/// Excel's cell text limit, in characters.
pub const MAX_CELL_TEXT: usize = 32_767;

/// Milliseconds in one day: the resolution a serial's clock is read at,
/// which is Excel's own display resolution and openpyxl's answer. A double
/// carries about 0.6 microseconds at the 2020s' serials, so the digits
/// below the millisecond are float noise, not the value written.
const DAY_MILLIS: i64 = 86_400 * 1_000;

/// Days from the Unix epoch back to 1899-12-30, the 1900 system's day zero.
const EPOCH_1900_DAYS: i64 = -25_569;

/// Days from the Unix epoch back to 1904-01-01, the 1904 system's day zero.
const EPOCH_1904_DAYS: i64 = -24_107;

/// The serial of 1900-03-01 in the 1900 system: the first day past the
/// phantom 1900-02-29 that Lotus 1-2-3 counted and Excel kept.
const FIRST_UNSHIFTED_1900: i64 = 61;

/// The serial of 9999-12-31, the last day either system spells.
const LAST_SERIAL_1900: i64 = 2_958_465;
const LAST_SERIAL_1904: i64 = LAST_SERIAL_1900 - (EPOCH_1904_DAYS - EPOCH_1900_DAYS);

/// Which day a workbook counts its serial dates from.
///
/// The 1900 system counts from 1899-12-30 and includes a day that never
/// was, 1900-02-29, as serial 60: serials 0 to 59 are one day later than
/// the count says - 0 is 1899-12-31, the day Excel displays as 1900-01-00,
/// and 1 is 1900-01-01 - 60 reads as 1900-02-28 (what openpyxl and
/// LibreOffice answer), and 61 onward is exact. Reading is the inverse of
/// writing at every serial, so 1899-12-31 written as 0 reads back as
/// itself; openpyxl alone reads 0 as 1899-12-30. The 1904 system counts
/// from 1904-01-01 with no such gap. A workbook states its system in
/// `workbookPr date1904`.
///
/// | serial | 1900 system | 1904 system |
/// | --- | --- | --- |
/// | 0 | 1899-12-31 (displays as 1900-01-00) | 1904-01-01 |
/// | 1 | 1900-01-01 | 1904-01-02 |
/// | 59 | 1900-02-28 | 1904-02-29 |
/// | 60 | 1900-02-28, the phantom day | 1904-03-01 |
/// | 61 | 1900-03-01 | 1904-03-02 |
/// | 25569 | 1970-01-01 | 1974-01-02 |
/// | 2958465 | 9999-12-31 | past the last day |
///
/// Writing runs the table backwards: a day from 1899-12-31 to 1900-02-28 is
/// one less than its count from 1899-12-30, a day before 1899-12-31 or past
/// 9999-12-31 has no serial and is refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DateSystem {
    /// Serial 1 is 1900-01-01; the default, and what Excel for Windows writes.
    #[default]
    Year1900,
    /// Serial 0 is 1904-01-01; what a workbook saved with `date1904` declares.
    Year1904,
}

/// Phantom-aware civil fields of one validated Excel serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExcelCalendar {
    pub(crate) year: i64,
    pub(crate) month: i64,
    pub(crate) day: i64,
    pub(crate) weekday: i64,
}

impl DateSystem {
    /// The whole serial day, validated before any formula date arithmetic.
    pub(crate) fn serial_day(self, serial: f64) -> Option<i64> {
        if !serial.is_finite() || serial < 0.0 || serial >= (self.last_serial() + 1) as f64 {
            return None;
        }
        Some(serial.floor() as i64)
    }

    /// Construct DATE's serial from its integer year, month and day parts.
    /// Month/day overflow uses the civil calendar while the final serial
    /// retains the 1900 phantom day and the selected workbook epoch.
    pub(crate) fn date_serial(self, year: f64, month: f64, day: f64) -> Option<i64> {
        if !year.is_finite() || !(0.0..10_000.0).contains(&year) {
            return None;
        }
        let year = year.trunc() as i64;
        let year = if year <= 1899 { year + 1900 } else { year };
        // Bound integer conversion before casting; f64-to-integer saturation
        // would silently turn a huge argument into a plausible date.
        const I64_CEILING: f64 = 9_223_372_036_854_775_808.0;
        if !month.is_finite() || !day.is_finite()
            || month < i64::MIN as f64 || month >= I64_CEILING
            || day < i64::MIN as f64 || day >= I64_CEILING
        {
            return None;
        }
        let month = month.trunc() as i64;
        let day = day.trunc() as i64;
        let month_index = year.checked_mul(12)?.checked_add(month)?.checked_sub(1)?;
        let civil_year = i32::try_from(month_index.div_euclid(12)).ok()?;
        let civil_month = (month_index.rem_euclid(12) + 1) as u32;
        // days_from_civil subtracts one from January/February's year.
        if civil_year == i32::MIN && civil_month <= 2 { return None; }
        let first = crate::timezone::days_from_civil(civil_year, civil_month, 1);
        let base = first - self.epoch_days();
        let base = if self == Self::Year1900
            && first < crate::timezone::days_from_civil(1900, 3, 1)
        {
            base - 1
        } else {
            base
        };
        let serial = base.checked_add(day)?.checked_sub(1)?;
        (0..=self.last_serial()).contains(&serial).then_some(serial)
    }

    /// Rounded whole second within a validated serial day. The clock can
    /// round through midnight; HOUR/MINUTE/SECOND then read 00:00:00.
    pub(crate) fn clock_second(self, serial: f64) -> Option<i64> {
        let day = self.serial_day(serial)?;
        let seconds = ((serial - day as f64) * 86_400.0).round() as i64;
        Some(seconds.rem_euclid(86_400))
    }

    /// Move a serial by whole civil months. EDATE retains/clamps the source
    /// day; EOMONTH lands on the real last day of the destination month.
    /// The 1900 phantom day can be a source but is not a civil month end.
    pub(crate) fn month_serial(self, serial: f64, months: f64, end: bool) -> Option<i64> {
        let source = self.calendar(serial)?;
        const I64_CEILING: f64 = 9_223_372_036_854_775_808.0;
        if !months.is_finite() || months < i64::MIN as f64 || months >= I64_CEILING {
            return None;
        }
        let index = source.year.checked_mul(12)?
            .checked_add(source.month)?.checked_sub(1)?
            .checked_add(months.trunc() as i64)?;
        let year = i32::try_from(index.div_euclid(12)).ok()?;
        let first_year = if self == Self::Year1900 { 1900 } else { 1904 };
        if !(first_year..=9999).contains(&year) { return None; }
        let month = (index.rem_euclid(12) + 1) as u32;
        let first = crate::timezone::days_from_civil(year, month, 1);
        let (next_year, next_month) = if month == 12 {
            (year.checked_add(1)?, 1)
        } else {
            (year, month + 1)
        };
        let last = crate::timezone::days_from_civil(next_year, next_month, 1) - first;
        let day = if end { last } else { source.day.min(last) };
        self.date_serial(f64::from(year), f64::from(month), day as f64)
    }
    /// Formula extraction retains day zero and phantom 60 as raw serial facts.
    pub(crate) fn calendar(self, serial: f64) -> Option<ExcelCalendar> {
        Some(self.civil_day(self.serial_day(serial)?))
    }

    /// The formatter and formulas share the existing phantom-aware civil rule.
    pub(crate) fn civil_day(self, day: i64) -> ExcelCalendar {
        let (days_from_epoch, weekday) = match self {
            Self::Year1900 => {
                let weekday = (day + 6).rem_euclid(7);
                match day {
                    0 => {
                        return ExcelCalendar { year: 1900, month: 1, day: 0, weekday };
                    }
                    60 => {
                        return ExcelCalendar { year: 1900, month: 2, day: 29, weekday };
                    }
                    1..=59 => (day - 25_568, weekday),
                    _ => (day - 25_569, weekday),
                }
            }
            Self::Year1904 => (day - 24_107, (day + 5).rem_euclid(7)),
        };
        let (year, month, day) = crate::timezone::civil_from_days(days_from_epoch);
        ExcelCalendar {
            year: i64::from(year),
            month: i64::from(month),
            day: i64::from(day),
            weekday,
        }
    }

    /// The system's name: `1900` or `1904`, the year its day zero falls in.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Year1900 => "1900",
            Self::Year1904 => "1904",
        }
    }

    /// Days from the Unix epoch to this system's day zero.
    const fn epoch_days(self) -> i64 {
        match self {
            Self::Year1900 => EPOCH_1900_DAYS,
            Self::Year1904 => EPOCH_1904_DAYS,
        }
    }

    /// The last serial this system spells, 9999-12-31.
    pub(crate) const fn last_serial(self) -> i64 {
        match self {
            Self::Year1900 => LAST_SERIAL_1900,
            Self::Year1904 => LAST_SERIAL_1904,
        }
    }

    /// Read a serial as milliseconds since the Unix epoch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a serial before day zero or past
    /// 9999-12-31, which no cell displays as a date.
    pub fn millis_from_serial(self, serial: f64) -> Result<i64> {
        let (mut day, clock) = self.serial_day_clock(serial)?;
        // Serials 0..59 of the 1900 system sit before the phantom day and
        // count one short; 60 is the phantom itself and reads as the 28th.
        if self == Self::Year1900 && day < 60 {
            day += 1;
        }
        Ok((day + self.epoch_days()) * DAY_MILLIS + clock)
    }

    fn serial_day_clock(self, serial: f64) -> Result<(i64, i64)> {
        if !serial.is_finite() || serial < 0.0 || serial >= (self.last_serial() + 1) as f64 {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected a serial date between 0 and {} in the {self} date system, got {serial}",
                    self.last_serial()
                ),
            });
        }
        let day = serial.floor();
        let fraction = serial - day;
        let clock = (fraction * DAY_MILLIS as f64).round() as i64;
        Ok((day as i64, clock))
    }

    /// Milliseconds for a PivotCache `<d>` item. Its 1900 day zero is OLE
    /// 1899-12-30 without the worksheet's phantom adjustment. Both forms
    /// share serial validation and clock rounding.
    pub(crate) fn pivot_cache_millis_from_serial(self, serial: f64) -> Result<i64> {
        let (day, clock) = self.serial_day_clock(serial)?;
        Ok((day + self.epoch_days()) * DAY_MILLIS + clock)
    }

    /// Spell a PivotCache `<d>` item once, after the source item is interned.
    pub(crate) fn pivot_cache_datetime(self, serial: f64) -> Result<SmolStr> {
        let millis = self.pivot_cache_millis_from_serial(serial)?;
        // Excel omits fractional seconds when the cache date is integral.
        let (value, unit) = if millis % 1000 == 0 {
            (millis / 1000, TimeUnit::Second)
        } else {
            (millis, TimeUnit::Millisecond)
        };
        crate::temporal::format_datetime(value, unit).ok_or_else(|| {
            Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!("expected a pivot cache date for serial {serial}"),
            }
        })
    }

    /// Write milliseconds since the Unix epoch as a serial.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an instant before the system's
    /// first day - 1899-12-31 in the 1900 system, 1904-01-01 in the 1904 one -
    /// or past 9999-12-31.
    pub fn serial_from_millis(self, millis: i64) -> Result<f64> {
        let days = millis.div_euclid(DAY_MILLIS) - self.epoch_days();
        let clock = millis.rem_euclid(DAY_MILLIS);
        let serial_day = if self == Self::Year1900 && days < FIRST_UNSHIFTED_1900 {
            days - 1
        } else {
            days
        };
        if serial_day < 0 || serial_day > self.last_serial() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected an instant the {self} date system spells, from its first day to \
                     9999-12-31, got {}",
                    crate::temporal::format_datetime(millis, TimeUnit::Millisecond)
                        .unwrap_or_else(|| format_smolstr!("{millis} ms since the epoch"))
                ),
            });
        }
        Ok(serial_day as f64 + clock as f64 / DAY_MILLIS as f64)
    }

    /// Read a serial as the days of a `date32`, refusing a clock.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a serial with a fraction, which
    /// names a clock a date cannot hold, or one outside the system.
    pub fn days_from_serial(self, serial: f64) -> Result<i32> {
        let millis = self.millis_from_serial(serial)?;
        if millis.rem_euclid(DAY_MILLIS) != 0 {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!("expected a whole day for a date, got serial {serial}"),
            });
        }
        i32::try_from(millis.div_euclid(DAY_MILLIS)).map_err(|_| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("the date at serial {serial} does not fit a date32"),
        })
    }

    /// Read a serial under `format` as the value it spells: a whole day under
    /// a date format is a `date32`, any other date, datetime or long time is
    /// a `datetime64(ms)`, a time below one day a `time32(ms)`, and an
    /// elapsed time a `duration64(ms)`; a General number is `float64`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a serial outside the system.
    pub fn scalar_from_serial(self, serial: f64, format: NumberFormat) -> Result<Scalar> {
        match format {
            NumberFormat::General => Ok(Scalar::from(serial)),
            NumberFormat::Date if serial.fract() == 0.0 => {
                Ok(Scalar::date32(self.days_from_serial(serial)?))
            }
            NumberFormat::Time if (0.0..1.0).contains(&serial) => {
                let millis = (serial * DAY_MILLIS as f64).round() as i64;
                Scalar::time32(
                    i32::try_from(millis).expect("a day of milliseconds fits i32"),
                    TimeUnit::Millisecond,
                    Timezone::NAIVE,
                )
            }
            NumberFormat::Duration => {
                let millis = (serial * DAY_MILLIS as f64).round() as i64;
                Scalar::duration64(millis, TimeUnit::Millisecond)
            }
            _ => Scalar::datetime64(
                self.millis_from_serial(serial)?,
                TimeUnit::Millisecond,
                Timezone::NAIVE,
            ),
        }
    }

    /// Read a serial as the temporal leaf `dtype` names, whatever the cell's
    /// own format said: what a numeric cell means under a declared temporal
    /// column. The serial is read at the millisecond in its family, then
    /// restated under the leaf by [`DataType::scalar`] - at the leaf's unit,
    /// and, for a zoned datetime, as the wall clock of that zone.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a serial the leaf cannot hold: a
    /// clock under a date, a day or more under a time, one outside the
    /// system, or one the leaf's own contract refuses.
    pub fn temporal_from_serial(self, serial: f64, dtype: &DataType) -> Result<Scalar> {
        let value = match dtype.id().temporal_kind() {
            Some(TemporalKind::Date) => Scalar::date32(self.days_from_serial(serial)?),
            Some(TemporalKind::Time) => {
                if !(0.0..1.0).contains(&serial) {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$"),
                        reason: format_smolstr!(
                            "expected a serial below one day for a time, got {serial}"
                        ),
                    });
                }
                self.scalar_from_serial(serial, NumberFormat::Time)?
            }
            Some(TemporalKind::Duration) => {
                self.scalar_from_serial(serial, NumberFormat::Duration)?
            }
            _ => self.scalar_from_serial(serial, NumberFormat::DateTime)?,
        };
        dtype.scalar(value)
    }

    /// The serial a temporal value is written as, and the format that reads
    /// it back: a date or a naive datetime as days since the epoch, a time or
    /// a duration as a fraction of a day. `None` for a value that is not a
    /// naive temporal, which a cell spells as text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an instant the system does not
    /// spell.
    pub fn serial_of(self, value: &Scalar) -> Result<Option<(f64, NumberFormat)>> {
        // The value's count at its own unit, as whole milliseconds and the
        // fraction of one below them: a serial carries what a double can.
        let millis = |value: &Scalar| -> Result<(i64, f64)> {
            let (count, unit) = value
                .temporal_count()
                .zip(value.temporal_unit())
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a temporal value, got {}", value.kind()),
                })?;
            let per_tick = crate::temporal::scalars::nanoseconds_per(unit).ok_or_else(|| {
                Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a clock unit, got {unit}"),
                }
            })?;
            let nanos = i128::from(count) * per_tick;
            let whole =
                i64::try_from(nanos.div_euclid(1_000_000)).map_err(|_| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!(
                        "expected an instant within the millisecond range, got {}",
                        value.kind()
                    ),
                })?;
            Ok((whole, nanos.rem_euclid(1_000_000) as f64 / 1_000_000.0))
        };
        let Some(format) = value
            .temporal_unit()
            .zip(value.temporal_timezone())
            .and_then(|(unit, zone)| NumberFormat::of_temporal(value.id(), unit, zone.is_naive()))
        else {
            return Ok(None);
        };
        let (whole, rest) = millis(value)?;
        let serial = match format {
            NumberFormat::Date => self.serial_from_millis(whole)?,
            NumberFormat::DateTime | NumberFormat::DateTimeFraction => {
                self.serial_from_millis(whole)? + rest / DAY_MILLIS as f64
            }
            NumberFormat::Time | NumberFormat::Duration => {
                (whole as f64 + rest) / DAY_MILLIS as f64
            }
            NumberFormat::General => return Ok(None),
        };
        Ok(Some((serial, format)))
    }
}

impl fmt::Display for DateSystem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One cell's position, zero-based: row 0 column 0 is `A1`.
///
/// The A1 spelling is the file's, so [`FromStr`] reads it - `$` markers
/// dropped, a sheet qualifier refused - and [`Display`](fmt::Display) writes it; a pair
/// `(row, column)` converts. Every index is zero-based like every other index
/// in the crate, and the bounds are the worksheet's, [`MAX_ROWS`] by
/// [`MAX_COLUMNS`].
///
/// ```
/// use yggdryl::excel::CellRef;
///
/// let cell: CellRef = "B2".parse()?;
/// assert_eq!((cell.row(), cell.column()), (1, 1));
/// assert_eq!(CellRef::from((0, 27)).to_string(), "AB1");
/// assert_eq!(CellRef::column_name(16_383), "XFD");
/// assert_eq!("$C$3".parse::<CellRef>()?, CellRef::from((2, 2)));
/// assert!("XFE1".parse::<CellRef>().is_err());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRef {
    row: u32,
    column: u32,
}

impl CellRef {
    /// The cell at zero-based `row` and `column`.
    #[must_use]
    pub const fn new(row: u32, column: u32) -> Self {
        Self { row, column }
    }

    /// The zero-based row.
    #[must_use]
    pub const fn row(self) -> u32 {
        self.row
    }

    /// The zero-based column.
    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }

    /// Whether the cell lies inside the worksheet's grid.
    #[must_use]
    pub const fn is_in_grid(self) -> bool {
        self.row < MAX_ROWS && self.column < MAX_COLUMNS
    }

    /// Refuse a cell outside the grid, naming it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the cell and the limit.
    pub fn require_in_grid(self) -> Result<Self> {
        if self.is_in_grid() {
            return Ok(self);
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "expected a cell within {MAX_ROWS} rows and {MAX_COLUMNS} columns (A1 to \
                 XFD{MAX_ROWS}), got row {} column {}",
                u64::from(self.row) + 1,
                u64::from(self.column) + 1
            ),
        })
    }

    /// The letters of a zero-based column: `A` for 0, `Z` for 25, `AA` for 26.
    #[must_use]
    pub fn column_name(column: u32) -> SmolStr {
        let mut text = String::with_capacity(3);
        push_column_name(&mut text, column);
        SmolStr::new(text)
    }

    /// The zero-based column the letters spell, `None` for anything else.
    #[must_use]
    pub fn column_index(letters: &str) -> Option<u32> {
        if letters.is_empty() || letters.len() > 3 {
            return None;
        }
        let mut column: u32 = 0;
        for byte in letters.bytes() {
            let letter = byte.to_ascii_uppercase();
            if !letter.is_ascii_uppercase() {
                return None;
            }
            column = column * 26 + u32::from(letter - b'A') + 1;
        }
        (column <= MAX_COLUMNS).then(|| column - 1)
    }

    /// Write the A1 spelling into `target`.
    pub(crate) fn write_a1(self, target: &mut String) {
        use std::fmt::Write as _;

        push_column_name(target, self.column);
        let _ = write!(target, "{}", self.row + 1);
    }
}

/// Push the letters of a zero-based column.
fn push_column_name(target: &mut String, column: u32) {
    let mut letters = [0_u8; 8];
    let mut at = letters.len();
    let mut rest = i64::from(column);
    loop {
        at -= 1;
        letters[at] = b'A' + (rest % 26) as u8;
        rest = rest / 26 - 1;
        if rest < 0 {
            break;
        }
    }
    target.push_str(std::str::from_utf8(&letters[at..]).expect("ASCII letters"));
}

impl From<(u32, u32)> for CellRef {
    fn from((row, column): (u32, u32)) -> Self {
        Self::new(row, column)
    }
}

impl FromStr for DateSystem {
    type Err = Error;

    /// Read `1900` or `1904`, or the `date1904` attribute's `0`, `1`, `false`
    /// and `true`.
    fn from_str(text: &str) -> Result<Self> {
        match text.trim() {
            "1900" | "0" | "false" => Ok(Self::Year1900),
            "1904" | "1" | "true" => Ok(Self::Year1904),
            other => Err(Error::Parse {
                target: "date system",
                position: 0,
                reason: format_smolstr!("expected 1900 or 1904 for a date system, got {other:?}"),
            }),
        }
    }
}

impl FromStr for CellRef {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let (column, row) = parse_reference(text, 0)?;
        let (Some(column), Some(row)) = (column, row) else {
            return Err(refusal(
                text,
                format_smolstr!("expected a column and a row such as B2, got {text:?}"),
            ));
        };
        Ok(Self::new(row, column))
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = String::with_capacity(10);
        self.write_a1(&mut text);
        formatter.write_str(&text)
    }
}

/// A rectangle of cells, `A1:C10`, both corners included.
///
/// A range may leave one side open, as a whole column `A:C`, a whole row
/// `3:3` or rows from a row on, `A3:F`; the open side runs to the edge of
/// the grid. The corners may come in either order.
///
/// ```
/// use yggdryl::excel::{CellRange, CellRef, MAX_ROWS};
///
/// let range: CellRange = "B2:C3".parse()?;
/// assert_eq!(range.row_size(), 2);
/// assert_eq!(range.column_size(), 2);
/// assert!(range.contains(CellRef::from((2, 2))));
/// assert_eq!(range.cells().count(), 4);
/// assert_eq!("B2".parse::<CellRange>()?.to_string(), "B2");
/// assert_eq!("C3:A1".parse::<CellRange>()?.to_string(), "A1:C3");
///
/// let open: CellRange = "A3:F".parse()?;
/// assert_eq!(open.start(), CellRef::from((2, 0)));
/// assert_eq!(open.end().row(), MAX_ROWS - 1);
/// assert_eq!(open.to_string(), "A3:F");
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRange {
    start: CellRef,
    end: CellRef,
}

impl CellRange {
    /// The rectangle between two corners, whichever order they are given in.
    #[must_use]
    pub fn new(first: CellRef, second: CellRef) -> Self {
        Self {
            start: CellRef::new(first.row.min(second.row), first.column.min(second.column)),
            end: CellRef::new(first.row.max(second.row), first.column.max(second.column)),
        }
    }

    /// The whole grid.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            start: CellRef::new(0, 0),
            end: CellRef::new(MAX_ROWS - 1, MAX_COLUMNS - 1),
        }
    }

    /// The top-left cell.
    #[must_use]
    pub const fn start(self) -> CellRef {
        self.start
    }

    /// The bottom-right cell.
    #[must_use]
    pub const fn end(self) -> CellRef {
        self.end
    }

    /// Rows spanned.
    #[must_use]
    pub const fn row_size(self) -> u32 {
        self.end.row - self.start.row + 1
    }

    /// Columns spanned.
    #[must_use]
    pub const fn column_size(self) -> u32 {
        self.end.column - self.start.column + 1
    }

    /// Whether the rows run to the edge of the grid.
    #[must_use]
    pub const fn is_row_open(self) -> bool {
        self.end.row == MAX_ROWS - 1
    }

    /// Whether the columns run to the edge of the grid.
    #[must_use]
    pub const fn is_column_open(self) -> bool {
        self.end.column == MAX_COLUMNS - 1
    }

    /// Whether `cell` lies inside the rectangle.
    #[must_use]
    pub const fn contains(self, cell: CellRef) -> bool {
        cell.row >= self.start.row
            && cell.row <= self.end.row
            && cell.column >= self.start.column
            && cell.column <= self.end.column
    }

    /// Whether zero-based `row` lies inside the rectangle's rows.
    #[must_use]
    pub const fn contains_row(self, row: u32) -> bool {
        row >= self.start.row && row <= self.end.row
    }

    /// Whether zero-based `column` lies inside the rectangle's columns.
    #[must_use]
    pub const fn contains_column(self, column: u32) -> bool {
        column >= self.start.column && column <= self.end.column
    }

    /// Every cell of the rectangle, row by row.
    pub fn cells(self) -> impl Iterator<Item = CellRef> {
        (self.start.row..=self.end.row).flat_map(move |row| {
            (self.start.column..=self.end.column).map(move |column| CellRef::new(row, column))
        })
    }

    /// How many cells the rectangle holds.
    #[must_use]
    pub(crate) const fn cell_count(self) -> u64 {
        self.row_size() as u64 * self.column_size() as u64
    }

    /// Whether the two rectangles share a cell.
    #[must_use]
    pub(crate) const fn intersects(self, other: Self) -> bool {
        self.start.row <= other.end.row
            && other.start.row <= self.end.row
            && self.start.column <= other.end.column
            && other.start.column <= self.end.column
    }

    /// Whether `other` lies wholly inside the rectangle.
    #[must_use]
    pub(crate) const fn encloses(self, other: Self) -> bool {
        self.contains(other.start) && self.contains(other.end)
    }

    /// The whole rows `rows`, which name one row at least.
    #[must_use]
    pub(crate) const fn of_rows(rows: std::ops::Range<u32>) -> Self {
        Self {
            start: CellRef::new(rows.start, 0),
            end: CellRef::new(rows.end - 1, MAX_COLUMNS - 1),
        }
    }

    /// The whole columns `columns`, which name one column at least.
    #[must_use]
    pub(crate) const fn of_columns(columns: std::ops::Range<u32>) -> Self {
        Self {
            start: CellRef::new(0, columns.start),
            end: CellRef::new(MAX_ROWS - 1, columns.end - 1),
        }
    }

    /// The rectangle of the same size with `target` its top-left cell; the
    /// caller keeps it on the grid.
    #[must_use]
    pub(crate) const fn moved_to(self, target: CellRef) -> Self {
        Self {
            start: target,
            end: CellRef::new(
                target.row + (self.end.row - self.start.row),
                target.column + (self.end.column - self.start.column),
            ),
        }
    }
}

impl From<(CellRef, CellRef)> for CellRange {
    fn from((first, second): (CellRef, CellRef)) -> Self {
        Self::new(first, second)
    }
}

impl FromStr for CellRange {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let Some((first, second)) = text.split_once(':') else {
            let cell: CellRef = text.parse()?;
            return Ok(Self::new(cell, cell));
        };
        let (column, row) = parse_reference(first, 0)?;
        let (end_column, end_row) = parse_reference(second, first.len() + 1)?;
        // `A:C` names whole columns, `3:5` whole rows, `A3:F` rows from 3 on
        // in columns A to F; any other pairing of a whole corner, a column
        // alone and a row alone is refused rather than read one way.
        let shape = (
            column.is_some(),
            row.is_some(),
            end_column.is_some(),
            end_row.is_some(),
        );
        let (start, end) = match shape {
            (true, true, true, true) => (
                CellRef::new(row.unwrap_or(0), column.unwrap_or(0)),
                CellRef::new(end_row.unwrap_or(0), end_column.unwrap_or(0)),
            ),
            (true, false, true, false) => (
                CellRef::new(0, column.unwrap_or(0)),
                CellRef::new(MAX_ROWS - 1, end_column.unwrap_or(0)),
            ),
            (false, true, false, true) => (
                CellRef::new(row.unwrap_or(0), 0),
                CellRef::new(end_row.unwrap_or(0), MAX_COLUMNS - 1),
            ),
            (true, true, true, false) => (
                CellRef::new(row.unwrap_or(0), column.unwrap_or(0)),
                CellRef::new(MAX_ROWS - 1, end_column.unwrap_or(0)),
            ),
            _ => {
                return Err(refusal(
                    text,
                    format_smolstr!(
                        "expected a cell range such as A1:C10, A:C, 3:5 or A3:F, got {text:?}"
                    ),
                ));
            }
        };
        Ok(Self::new(start, end))
    }
}

impl fmt::Display for CellRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            return write!(formatter, "{}", self.start);
        }
        let mut text = String::with_capacity(20);
        if self.is_row_open() && self.start.row == 0 {
            // Whole columns: `A:C`.
            push_column_name(&mut text, self.start.column);
            text.push(':');
            push_column_name(&mut text, self.end.column);
        } else if self.is_column_open() && self.start.column == 0 {
            // Whole rows: `3:5`.
            text = format!("{}:{}", self.start.row + 1, self.end.row + 1);
        } else {
            self.start.write_a1(&mut text);
            text.push(':');
            if self.is_row_open() {
                push_column_name(&mut text, self.end.column);
            } else {
                self.end.write_a1(&mut text);
            }
        }
        formatter.write_str(&text)
    }
}

/// Read one corner: its column letters and its row, either optional, `$`
/// dropped, a sheet qualifier refused. `offset` is where the corner starts
/// in the text the caller gave, so a refusal is located in that text.
fn parse_reference(text: &str, offset: usize) -> Result<(Option<u32>, Option<u32>)> {
    if let Some(at) = text.find('!') {
        return Err(Error::Parse {
            target: "cell reference",
            position: offset + at,
            reason: format_smolstr!(
                "expected a reference within the sheet, got the sheet-qualified {text:?}"
            ),
        });
    }
    // An absolute reference marks its column and its row with `$`, so the
    // marks are stripped where they may stand and nowhere else; nothing is
    // built, because a reference is parsed per cell of a part.
    let plain = text.trim_start_matches('$');
    let letters_at = offset + (text.len() - plain.len());
    let split = plain
        .bytes()
        .position(|byte| byte.is_ascii_digit())
        .unwrap_or(plain.len());
    let (letters, digits) = plain.split_at(split);
    let letters = letters.strip_suffix('$').unwrap_or(letters);
    let digits_at = offset + text.len() - digits.len();
    let column = if letters.is_empty() {
        None
    } else {
        Some(CellRef::column_index(letters).ok_or_else(|| Error::Parse {
            target: "cell reference",
            position: letters_at,
            reason: format_smolstr!(
                "expected column letters A to XFD in {text:?}, got {letters:?}"
            ),
        })?)
    };
    let row = if digits.is_empty() {
        None
    } else {
        let row: u32 = digits
            .parse()
            .ok()
            .filter(|row| (1..=MAX_ROWS).contains(row))
            .ok_or_else(|| Error::Parse {
                target: "cell reference",
                position: digits_at,
                reason: format_smolstr!(
                    "expected a row number from 1 to {MAX_ROWS} in {text:?}, got {digits:?}"
                ),
            })?;
        Some(row - 1)
    };
    Ok((column, row))
}

fn refusal(text: &str, reason: SmolStr) -> Error {
    let position =
        text.find(|character: char| !character.is_ascii_alphanumeric() && character != '$');
    Error::Parse {
        target: "cell reference",
        position: position.unwrap_or(0),
        reason,
    }
}

/// The `t` attribute of a `<c>`: what kind of content the cell states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CellKind {
    /// `n`, or no `t` at all: a number in `<v>`, a date under a date style.
    #[default]
    Number,
    /// `s`: `<v>` indexes the shared string table.
    SharedString,
    /// `str`: `<v>` is the text a formula produced.
    FormulaString,
    /// `inlineStr`: `<is>` holds the text itself.
    InlineString,
    /// `b`: `<v>` is `0` or `1`.
    Boolean,
    /// `d`: `<v>` is an ISO 8601 date, time or datetime.
    Date,
    /// `e`: `<v>` is an error such as `#N/A` or `#DIV/0!`.
    Error,
}

impl CellKind {
    /// The `t` attribute as the file spells it; a number states none.
    #[must_use]
    pub const fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Number => None,
            Self::SharedString => Some("s"),
            Self::FormulaString => Some("str"),
            Self::InlineString => Some("inlineStr"),
            Self::Boolean => Some("b"),
            Self::Date => Some("d"),
            Self::Error => Some("e"),
        }
    }

    /// The kind a `t` attribute spells.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a value the schema does not list.
    pub fn from_attribute(value: &str) -> Result<Self> {
        Ok(match value {
            "n" => Self::Number,
            "s" => Self::SharedString,
            "str" => Self::FormulaString,
            "inlineStr" => Self::InlineString,
            "b" => Self::Boolean,
            "d" => Self::Date,
            "e" => Self::Error,
            other => {
                return Err(Error::Parse {
                    target: "cell type",
                    position: 0,
                    reason: format_smolstr!(
                        "expected one of n, s, str, inlineStr, b, d, e for a cell's `t`, got {other:?}"
                    ),
                });
            }
        })
    }

    /// Whether the cell's content is text.
    #[must_use]
    pub const fn is_text(self) -> bool {
        matches!(
            self,
            Self::SharedString | Self::FormulaString | Self::InlineString
        )
    }
}

/// An error value a cell holds: what its `<v>` spells under `t="e"`.
///
/// The eighteen errors Excel spells are variants of their own, from the
/// classic `#NULL!` to `#DIV/0!`, `#N/A` and `#NAME?` through the ones a
/// dynamic array, a data type or a connected source answers - `#SPILL!`,
/// `#CALC!`, `#FIELD!`, `#BLOCKED!`, `#CONNECT!`, `#BUSY!`, `#UNKNOWN!`,
/// `#PYTHON!`, `#TIMEOUT!`, `#EXTERNAL!` and `#GETTING_DATA`. Any other
/// text is [`ExcelError::Unrecognized`]: the cell keeps the literal it read
/// and writes it back unchanged, so an error a later Excel adds survives a
/// round trip through this crate.
///
/// ```
/// use yggdryl::excel::ExcelError;
///
/// assert_eq!(ExcelError::from_text("#DIV/0!"), ExcelError::Div0);
/// assert_eq!(ExcelError::from_text("#n/a"), ExcelError::NA);
/// assert_eq!(ExcelError::Spill.as_str(), "#SPILL!");
/// assert_eq!(ExcelError::from_text("#WHAT?"), ExcelError::Unrecognized);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExcelError {
    /// `#NULL!`: two ranges that do not intersect.
    Null,
    /// `#DIV/0!`: a division by zero.
    Div0,
    /// `#VALUE!`: an operand of the wrong type.
    Value,
    /// `#REF!`: a reference to a cell that is gone.
    Ref,
    /// `#NAME?`: a name no function or defined name has.
    Name,
    /// `#NUM!`: a number no cell can hold.
    Num,
    /// `#N/A`: a value that is not available.
    NA,
    /// `#GETTING_DATA`: a value still being fetched.
    GettingData,
    /// `#SPILL!`: a dynamic array with no room to spill.
    Spill,
    /// `#CALC!`: a calculation the engine cannot perform.
    Calc,
    /// `#FIELD!`: a field a data type does not have.
    Field,
    /// `#BLOCKED!`: a feature the workbook's settings block.
    Blocked,
    /// `#CONNECT!`: a connected service that did not answer.
    Connect,
    /// `#BUSY!`: a value a service is still computing.
    Busy,
    /// `#UNKNOWN!`: a data type this Excel does not know.
    Unknown,
    /// `#PYTHON!`: a Python formula that failed.
    Python,
    /// `#TIMEOUT!`: a computation that ran out of time.
    Timeout,
    /// `#EXTERNAL!`: an external computation that failed.
    External,
    /// Any other text: the cell holds the literal it was read with.
    Unrecognized,
}

impl ExcelError {
    /// The error as Excel spells it; [`Self::Unrecognized`] answers
    /// `#UNRECOGNIZED`, which is never written - its cell writes the literal
    /// it holds.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "#NULL!",
            Self::Div0 => "#DIV/0!",
            Self::Value => "#VALUE!",
            Self::Ref => "#REF!",
            Self::Name => "#NAME?",
            Self::Num => "#NUM!",
            Self::NA => "#N/A",
            Self::GettingData => "#GETTING_DATA",
            Self::Spill => "#SPILL!",
            Self::Calc => "#CALC!",
            Self::Field => "#FIELD!",
            Self::Blocked => "#BLOCKED!",
            Self::Connect => "#CONNECT!",
            Self::Busy => "#BUSY!",
            Self::Unknown => "#UNKNOWN!",
            Self::Python => "#PYTHON!",
            Self::Timeout => "#TIMEOUT!",
            Self::External => "#EXTERNAL!",
            Self::Unrecognized => "#UNRECOGNIZED",
        }
    }

    /// The error `text` spells, compared without case as Excel reads an
    /// error typed in any case; any other text is [`Self::Unrecognized`].
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        const KNOWN: [ExcelError; 18] = [
            ExcelError::Null,
            ExcelError::Div0,
            ExcelError::Value,
            ExcelError::Ref,
            ExcelError::Name,
            ExcelError::Num,
            ExcelError::NA,
            ExcelError::GettingData,
            ExcelError::Spill,
            ExcelError::Calc,
            ExcelError::Field,
            ExcelError::Blocked,
            ExcelError::Connect,
            ExcelError::Busy,
            ExcelError::Unknown,
            ExcelError::Python,
            ExcelError::Timeout,
            ExcelError::External,
        ];
        KNOWN
            .into_iter()
            .find(|error| error.as_str().eq_ignore_ascii_case(text))
            .unwrap_or(Self::Unrecognized)
    }
}

impl fmt::Display for ExcelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One cell: its reference, what the file stated about it, and its value.
///
/// The value is the one the file's facts prove, derived once when the cell
/// is read: `float64` for a number, the `date32`, `datetime64(ms)`,
/// `time32(ms)` or `duration64(ms)` a serial spells under a temporal
/// format, `boolean` for `b`, `utf8` for text, the datetime a `d` cell's
/// ISO 8601 text spells, and null for an error or an empty cell. A cell
/// built from a value states the kind and format that value writes as.
///
/// A cell is at most 80 bytes: its value, its reference, one pointer to a
/// shared [`Formula`], its [`StyleId`], and one byte each for its kind, its
/// format and its error. What only a few cells state is held beside them by
/// their [`Sheet`](super::Sheet), never in every cell.
///
/// ```
/// use yggdryl::excel::{Cell, CellKind, CellRef, DateSystem, NumberFormat};
/// use yggdryl::Scalar;
///
/// let price = Cell::from_scalar(CellRef::from((1, 1)), Scalar::from(187.23), DateSystem::Year1900)?;
/// assert_eq!(price.kind(), CellKind::Number);
/// assert_eq!(price.value(), &Scalar::from(187.23));
/// assert_eq!(price.text(), "187.23");
///
/// let day = Cell::from_scalar(CellRef::from((1, 2)), Scalar::date32(19_723), DateSystem::Year1900)?;
/// assert_eq!(day.format(), NumberFormat::Date);
/// assert_eq!(day.text(), "2024-01-01");
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// The typed value, a formula's cached result, or the literal an
    /// [`ExcelError::Unrecognized`] was read with.
    value: Scalar,
    reference: CellRef,
    formula: Option<Formula>,
    style: StyleId,
    kind: CellKind,
    format: NumberFormat,
    error: Option<ExcelError>,
}

const _: () = assert!(std::mem::size_of::<Cell>() <= 80);

impl Cell {
    /// A cell stated fact by fact: the kind and format the file gave it, and
    /// the value they proved.
    #[must_use]
    pub const fn new(
        reference: CellRef,
        kind: CellKind,
        format: NumberFormat,
        value: Scalar,
    ) -> Self {
        Self {
            value,
            reference,
            formula: None,
            style: StyleId::DEFAULT,
            kind,
            format,
            error: None,
        }
    }

    /// The cell that spells `value` under `system`.
    ///
    /// A boolean is a `b` cell; a date, a time, a naive datetime and a
    /// duration are serials under the matching format; a number is its
    /// digits; every string leaf is a string cell, and so is every other
    /// leaf with a text spelling - a code, an enum, an identifier, a zoned
    /// datetime as ISO 8601 with its offset, bytes as base64 - and a nested
    /// value, spelled as its JSON text because a cell is flat. A float that
    /// is not a number is the `#NUM!` error. The value is kept as given, so
    /// the cell reads back what it was built from.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the cell for an instant the
    /// date system does not spell, or a text longer than [`MAX_CELL_TEXT`]
    /// characters.
    pub fn from_scalar(reference: CellRef, value: Scalar, system: DateSystem) -> Result<Self> {
        // A cell on its own is located by its reference alone; a sheet
        // holding it names itself in front.
        let located = |error: Error| Error::InvalidRecord {
            path: SmolStr::new(reference.to_string()),
            reason: wire_reason(&error),
        };
        let (kind, format) = match &value {
            Scalar::Null => (CellKind::Number, NumberFormat::General),
            Scalar::Boolean(_) => (CellKind::Boolean, NumberFormat::General),
            Scalar::Float16(_) | Scalar::Float32(_) | Scalar::Float64(_) => {
                if value.as_f64().is_some_and(f64::is_finite) {
                    (CellKind::Number, NumberFormat::General)
                } else {
                    return Ok(Self::new(
                        reference,
                        CellKind::Number,
                        NumberFormat::General,
                        Scalar::Null,
                    )
                    .with_error(ExcelError::Num));
                }
            }
            Scalar::Int8(_)
            | Scalar::Int16(_)
            | Scalar::Int32(_)
            | Scalar::Int64(_)
            | Scalar::UInt8(_)
            | Scalar::UInt16(_)
            | Scalar::UInt32(_)
            | Scalar::UInt64(_)
            | Scalar::Int128(_)
            | Scalar::UInt128(_)
            | Scalar::Decimal32(_)
            | Scalar::Decimal64(_)
            | Scalar::Decimal128(_)
            | Scalar::Decimal256(_)
            | Scalar::Decimal(_)
            | Scalar::BigDecimal(_) => (CellKind::Number, NumberFormat::General),
            _ => match system.serial_of(&value).map_err(located)? {
                Some((_, format)) => (CellKind::Number, format),
                None => (CellKind::SharedString, NumberFormat::General),
            },
        };
        let cell = Self::new(reference, kind, format, value);
        if kind.is_text() {
            let text = cell.text();
            let length = text.chars().count();
            if length > MAX_CELL_TEXT {
                return Err(located(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!(
                        "expected at most {MAX_CELL_TEXT} characters in a cell, got {length}"
                    ),
                }));
            }
        }
        Ok(cell)
    }

    /// Read a calculation operand from this cell and its already resolved
    /// source serial, without reparsing cell text.
    pub(crate) fn calculation_operand(
        &self,
        system: DateSystem,
        raw: Option<f64>,
    ) -> super::formula::value::Outcome {
        use super::formula::shape::Held;
        use super::formula::value::{Operand, Outcome, Unevaluated};
        if let Some(error) = self.error {
            return if error == ExcelError::Unrecognized {
                Outcome::Uncomputed(Unevaluated::Held(Held::Unrecognized))
            } else {
                Outcome::Computed(Operand::Error(error))
            };
        }
        if self.kind.is_text() {
            // A typed source value can be serialized as text without changing
            // its Scalar. The cell owns that wire/display spelling.
            let text = match &self.value {
                crate::string_scalars!(text) => text.clone(),
                _ => self.text().into(),
            };
            return Outcome::Computed(Operand::Text(text));
        }
        let value = match &self.value {
            Scalar::Null => Operand::Blank,
            Scalar::Boolean(value) => Operand::Boolean(value.get()),
            crate::string_scalars!(text) => Operand::Text(text.clone()),
            value if value.temporal_unit().is_some() => {
                let serial = raw.or_else(|| {
                    system
                        .serial_of(value)
                        .ok()
                        .flatten()
                        .map(|(serial, _)| serial)
                });
                return serial.map_or(Outcome::Uncomputed(Unevaluated::TemporalSerial), |serial| {
                    Outcome::Computed(Operand::Number(serial))
                });
            }
            value => match value.as_decimal_f64().or_else(|| number_of(value)) {
                Some(value) if value.is_finite() => Operand::Number(value),
                Some(_) => Operand::Error(ExcelError::Num),
                None => return Outcome::Uncomputed(Unevaluated::Coercion),
            },
        };
        Outcome::Computed(value)
    }

    /// A computed formula result staged without changing this cell's formula,
    /// style, reference, or adjacent metadata. Numeric caches are read under
    /// their format by the same date-system owner as wire caches.
    pub(crate) fn with_calculated(
        &self,
        result: super::formula::value::Operand,
        system: DateSystem,
        shown_format: NumberFormat,
    ) -> Result<Option<Self>> {
        use super::formula::value::Operand;

        debug_assert!(self.formula.is_some());
        let result = match result {
            Operand::Blank => Operand::Number(0.0),
            other => other,
        };
        let (kind, value, error, format) = match result {
            Operand::Blank => unreachable!("blank is normalized before cache publication"),
            Operand::Reference(_) => {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new(self.reference.to_string()),
                    reason: SmolStr::new_static(
                        "expected a scalar formula result, got an unresolved reference",
                    ),
                });
            }
            Operand::Number(value) => match system.scalar_from_serial(value, shown_format) {
                Ok(value) => (CellKind::Number, value, None, shown_format),
                // A temporal format cannot interpret this finite number. As
                // in restyle, retain its numeric meaning under General.
                Err(_) => (
                    CellKind::Number,
                    Scalar::from(value),
                    None,
                    NumberFormat::General,
                ),
            },
            Operand::Text(value) => {
                if value.as_str().chars().count() > MAX_CELL_TEXT {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new(self.reference.to_string()),
                        reason: format_smolstr!(
                            "expected at most {MAX_CELL_TEXT} text characters from the formula"
                        ),
                    });
                }
                (
                    CellKind::FormulaString,
                    Scalar::from(value),
                    None,
                    self.format,
                )
            }
            Operand::Boolean(value) => (CellKind::Boolean, Scalar::from(value), None, self.format),
            Operand::Error(error) => {
                if error == ExcelError::Unrecognized {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new(self.reference.to_string()),
                        reason: SmolStr::new_static(
                            "expected a recognized computed error, got an unrecognized one",
                        ),
                    });
                }
                (CellKind::Error, Scalar::Null, Some(error), self.format)
            }
        };
        if self.kind == kind && self.value == value && self.error == error && self.format == format
        {
            return Ok(None);
        }
        let mut changed = self.clone();
        changed.kind = kind;
        changed.value = value;
        changed.error = error;
        changed.format = format;
        Ok(Some(changed))
    }

    /// This cell with a formula, whose cached result the value is.
    #[must_use]
    pub fn with_formula(mut self, formula: Formula) -> Self {
        self.formula = Some(formula);
        self
    }

    /// Put `formula` in the cell, or take the one it holds out.
    pub(crate) fn set_formula(&mut self, formula: Option<Formula>) {
        self.formula = formula;
    }

    /// This cell holding the error `error`, its value null.
    ///
    /// An [`ExcelError::Unrecognized`] keeps the literal it was read with:
    /// the text the cell's value holds when the error is set, which is what
    /// [`Self::error_text`] answers and what the cell is written back as.
    /// An empty text is no literal.
    #[must_use]
    pub fn with_error(mut self, error: ExcelError) -> Self {
        let literal = error == ExcelError::Unrecognized
            && self.value.as_str().is_some_and(|text| !text.is_empty());
        if !literal {
            self.value = Scalar::Null;
        }
        self.kind = CellKind::Error;
        self.error = Some(error);
        self
    }

    /// This cell displayed with the style `style`.
    ///
    /// A style is an index into the cell formats of the workbook the sheet
    /// is written into ([`StyleSheet`](super::StyleSheet)): a save refuses
    /// an index those do not hold, naming the cell, and writes a temporal
    /// cell whose style does not read as its format under the style the
    /// workbook interns for that format.
    #[must_use]
    pub const fn with_style(mut self, style: StyleId) -> Self {
        self.style = style;
        self
    }

    /// Display the cell with `style`, nothing else changed.
    pub(crate) const fn set_style(&mut self, style: StyleId) {
        self.style = style;
    }

    /// Display the cell with the default style again.
    pub(crate) const fn clear_style(&mut self) {
        self.style = StyleId::DEFAULT;
    }

    /// Display the cell with `style`, whose number format says its number
    /// is `format`, reading the value again under it where that changes
    /// what the number is - `45292` under a date format is a date, a date
    /// under a number format its serial - through `system`. A number no
    /// date spells under a date format stays the number it is.
    /// Answers the source numeric serial for the sheet to retain when the
    /// new typed temporal value cannot reproduce it.
    pub(crate) fn restyle(
        &mut self,
        style: StyleId,
        format: NumberFormat,
        system: DateSystem,
        raw: Option<f64>,
    ) -> Option<f64> {
        self.style = style;
        if self.format == format {
            return raw;
        }
        if self.error.is_none() && self.kind == CellKind::Number && !self.value.is_null() {
            let serial = raw.or_else(|| match system.serial_of(&self.value) {
                Ok(Some((serial, _))) => Some(serial),
                _ => number_of(&self.value),
            });
            let Some(serial) = serial else {
                self.format = format;
                return None;
            };
            if !format.is_temporal() {
                if self.format.is_temporal() {
                    self.value = Scalar::from(serial);
                }
            } else if let Ok(value) = system.scalar_from_serial(serial, format) {
                self.value = value;
            } else {
                self.value = Scalar::from(serial);
                self.format = NumberFormat::General;
                return Some(serial);
            }
            self.format = format;
            return Some(serial);
        }
        self.format = format;
        None
    }

    /// This cell at another reference.
    #[must_use]
    pub const fn at(mut self, reference: CellRef) -> Self {
        self.move_to(reference);
        self
    }

    /// Where the cell is.
    #[must_use]
    pub const fn reference(&self) -> CellRef {
        self.reference
    }

    /// The zero-based row.
    #[must_use]
    pub const fn row(&self) -> u32 {
        self.reference.row
    }

    /// The zero-based column.
    #[must_use]
    pub const fn column(&self) -> u32 {
        self.reference.column
    }

    /// The `t` attribute's kind.
    #[must_use]
    pub const fn kind(&self) -> CellKind {
        self.kind
    }

    /// What the cell's style says about its number.
    #[must_use]
    pub const fn format(&self) -> NumberFormat {
        self.format
    }

    /// The `cellXfs` index the cell is displayed with, as the file stated
    /// it - the default for an index the file's styles do not hold;
    /// [`Self::with_style`] says how it is written back.
    #[must_use]
    pub const fn style(&self) -> StyleId {
        self.style
    }

    /// The value the cell holds: null for an error cell.
    #[must_use]
    pub const fn value(&self) -> &Scalar {
        match self.error {
            Some(_) => &Scalar::Null,
            None => &self.value,
        }
    }

    /// The value, taken: null for an error cell.
    #[must_use]
    pub fn into_scalar(self) -> Scalar {
        match self.error {
            Some(_) => Scalar::Null,
            None => self.value,
        }
    }

    /// The formula whose cached result the value is.
    #[must_use]
    pub const fn formula(&self) -> Option<&Formula> {
        self.formula.as_ref()
    }

    /// The error the cell holds, `#N/A`, `#DIV/0!` and the rest.
    #[must_use]
    pub const fn error(&self) -> Option<ExcelError> {
        self.error
    }

    /// The error as it is written: its spelling, or the literal an
    /// [`ExcelError::Unrecognized`] was read with - `#UNRECOGNIZED` for one
    /// holding no literal, which no part spells - and the empty text for a
    /// cell holding no error.
    #[must_use]
    pub fn error_text(&self) -> &str {
        match self.error {
            None => "",
            Some(error) => self.written_error().unwrap_or(error.as_str()),
        }
    }

    /// The text a part spells the cell's error with: `None` for no error,
    /// and for an unrecognized one holding no literal to write back.
    pub(crate) fn written_error(&self) -> Option<&str> {
        match self.error? {
            ExcelError::Unrecognized => self.value.as_str(),
            error => Some(error.as_str()),
        }
    }

    /// Move the cell to `reference` in place, which is what a sheet moving
    /// a row does to every cell in it; [`Self::at`] is its consuming form.
    pub(crate) const fn move_to(&mut self, reference: CellRef) {
        self.reference = reference;
    }

    /// Whether the cell holds no value: an empty cell or an error.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.error.is_some() || matches!(self.value, Scalar::Null)
    }

    /// Whether the cell holds something a user sees: a value, a formula or
    /// an error, as against a blank cell kept for its style.
    #[must_use]
    pub(crate) const fn has_content(&self) -> bool {
        self.formula.is_some() || self.error.is_some() || !matches!(self.value, Scalar::Null)
    }

    /// The cell spelled as text, which is what a declared `utf8` column
    /// reads and what the cell displays: a number by its shortest round
    /// trip, a date, time or datetime in ISO 8601, a boolean as `true` or
    /// `false`, text as stored, an error as its name, an empty cell as the
    /// empty text.
    #[must_use]
    pub fn text(&self) -> Cow<'_, str> {
        if self.error.is_some() {
            return Cow::Borrowed(self.error_text());
        }
        cell_text(&self.value)
    }
}

/// The number a numeric value is, `None` for anything else.
pub(crate) fn number_of(value: &Scalar) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        return Some(number);
    }
    if let Some(integer) = value.as_i128() {
        return Some(integer as f64);
    }
    if let Some(integer) = value.as_u128() {
        return Some(integer as f64);
    }
    if value.is_decimal() {
        return cell_text(value).parse::<f64>().ok();
    }
    None
}

/// A value spelled as a cell displays it.
///
/// Numbers by their shortest round trip, temporals in ISO 8601, booleans as
/// `true` and `false`, text as itself, null as the empty text, and every
/// other leaf as the XML codec spells it; a nested value as its JSON.
pub(crate) fn cell_text(value: &Scalar) -> Cow<'_, str> {
    match value {
        Scalar::Null => Cow::Borrowed(""),
        crate::string_scalars!(text) => Cow::Borrowed(text.as_str()),
        Scalar::Float16(_) | Scalar::Float32(_) | Scalar::Float64(_) => {
            let number = value.as_f64().expect("a float reads as f64");
            if number.is_finite() {
                let mut buffer = ryu::Buffer::new();
                Cow::Owned(trim_float(buffer.format(number)).to_owned())
            } else {
                Cow::Borrowed("#NUM!")
            }
        }
        Scalar::Int8(_)
        | Scalar::Int16(_)
        | Scalar::Int32(_)
        | Scalar::Int64(_)
        | Scalar::UInt8(_)
        | Scalar::UInt16(_)
        | Scalar::UInt32(_)
        | Scalar::UInt64(_)
        | Scalar::Int128(_)
        | Scalar::UInt128(_) => Cow::Owned(match (value.as_i128(), value.as_u128()) {
            (Some(signed), _) => signed.to_string(),
            (None, unsigned) => unsigned.unwrap_or_default().to_string(),
        }),
        Scalar::Serie(_)
        | Scalar::SerieView(_)
        | Scalar::FixedSizeSerie(_)
        | Scalar::LargeSerie(_)
        | Scalar::LargeSerieView(_)
        | Scalar::Map(_)
        | Scalar::SortedMap(_)
        | Scalar::Struct(_)
        | Scalar::Variant(_) => Cow::Owned(value.into_json().unwrap_or_default()),
        _ => {
            let mut text = Vec::new();
            if crate::xml::write_leaf_text(&mut text, value, "cell").is_err() {
                return Cow::Borrowed("");
            }
            Cow::Owned(unescape_content(
                &String::from_utf8(text).unwrap_or_default(),
            ))
        }
    }
}

/// Write what [`cell_text`] answers, building nothing for a number.
///
/// # Errors
///
/// Returns the writer's failure.
pub(crate) fn write_cell_text<W: std::io::Write>(writer: &mut W, value: &Scalar) -> Result<()> {
    match value {
        Scalar::Float16(_) | Scalar::Float32(_) | Scalar::Float64(_) => {
            let number = value.as_f64().expect("a float reads as f64");
            if number.is_finite() {
                let mut buffer = ryu::Buffer::new();
                writer.write_all(trim_float(buffer.format(number)).as_bytes())?;
            } else {
                writer.write_all(b"#NUM!")?;
            }
            Ok(())
        }
        Scalar::Int8(_)
        | Scalar::Int16(_)
        | Scalar::Int32(_)
        | Scalar::Int64(_)
        | Scalar::UInt8(_)
        | Scalar::UInt16(_)
        | Scalar::UInt32(_)
        | Scalar::UInt64(_)
        | Scalar::Int128(_)
        | Scalar::UInt128(_) => {
            match (value.as_i128(), value.as_u128()) {
                (Some(signed), _) => write!(writer, "{signed}")?,
                (None, unsigned) => write!(writer, "{}", unsigned.unwrap_or_default())?,
            }
            Ok(())
        }
        _ => crate::xml::write_element_text(writer, &cell_text(value)),
    }
}

/// `1.0` as `1`: what a cell shows for a whole number.
fn trim_float(text: &str) -> &str {
    text.strip_suffix(".0").unwrap_or(text)
}

/// Undo the five entity escapes the XML codec writes into element content.
fn unescape_content(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The shortest text that reads back as `serial`.
pub(crate) fn serial_text(serial: f64) -> SmolStr {
    if serial == 0.0 && serial.is_sign_negative() {
        return SmolStr::new_static("-0");
    }
    if serial.fract() == 0.0 && serial.abs() < 1e15 {
        return format_smolstr!("{}", serial as i64);
    }
    let mut buffer = ryu::Buffer::new();
    SmolStr::new(buffer.format(serial))
}

/// Read a numeric cell's text as Excel spells one: a decimal or scientific
/// literal, never `INF` or `NaN`.
pub(crate) fn parse_number(text: &str) -> Result<f64> {
    let trimmed = text.trim();
    let number: f64 = trimmed.parse().map_err(|_| Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!("expected a number in a numeric cell, got {text:?}"),
    })?;
    if !number.is_finite() {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("expected a finite number in a numeric cell, got {text:?}"),
        });
    }
    Ok(number)
}

/// The value a `d` cell's ISO 8601 text spells: a date alone a `date32`, a
/// time alone a `time`, else a datetime at the millisecond.
pub(crate) fn iso_scalar(text: &str) -> Result<Scalar> {
    let text = text.trim();
    if let Ok(days) = crate::temporal::parse_date(text) {
        return Ok(Scalar::date32(days));
    }
    // A datetime is read at the millisecond, as a serial is; digits below
    // it are refused by the millisecond leaf's own contract.
    if let Ok((count, unit, zone)) = crate::temporal::parse_timestamp(text) {
        return DataType::datetime64(TimeUnit::Millisecond, zone)?
            .scalar(Scalar::datetime64(count, unit, zone)?);
    }
    if let Ok((count, unit)) = crate::temporal::parse_datetime(text) {
        return DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE)?
            .scalar(Scalar::datetime64(count, unit, Timezone::NAIVE)?);
    }
    if let Ok((count, unit)) = crate::temporal::parse_time(text) {
        return match unit {
            TimeUnit::Second | TimeUnit::Millisecond => Scalar::time32(
                i32::try_from(count).map_err(|_| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a time within one day, got {text:?}"),
                })?,
                unit,
                Timezone::NAIVE,
            ),
            _ => Scalar::time64(count, unit, Timezone::NAIVE),
        };
    }
    Err(Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!(
            "expected an ISO 8601 date, time or datetime in a `d` cell, got {text:?}"
        ),
    })
}

/// A decoded cell scalar and its already-parsed numeric serial, or the
/// temporal interpretation that a held sheet alone may fall back from.
pub(crate) enum WireScalar {
    Decoded { scalar: Scalar, serial: Option<f64> },
    TemporalInvalid { serial: f64, error: Error },
}

impl WireScalar {
    pub(crate) fn into_scalar(self) -> Result<Scalar> {
        match self {
            Self::Decoded { scalar, .. } => Ok(scalar),
            Self::TemporalInvalid { error, .. } => Err(error),
        }
    }
}

/// The value the file's facts about one cell prove, before a field says
/// anything: what [`Cell::value`] holds and what inference reads.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] for content its kind refuses - a numeric
/// cell that is not a number, a serial outside the date system, a `d` cell
/// spelling no date.
pub(crate) fn wire_scalar(
    kind: CellKind,
    format: NumberFormat,
    system: DateSystem,
    content: &str,
) -> Result<WireScalar> {
    let decoded = |scalar| WireScalar::Decoded {
        scalar,
        serial: None,
    };
    Ok(match kind {
        CellKind::Number => {
            if content.trim().is_empty() {
                return Ok(decoded(Scalar::Null));
            }
            let serial = parse_number(content)?;
            match system.scalar_from_serial(serial, format) {
                Ok(scalar) => WireScalar::Decoded {
                    scalar,
                    serial: Some(serial),
                },
                Err(error) if format.is_temporal() => WireScalar::TemporalInvalid { serial, error },
                Err(error) => return Err(error),
            }
        }
        CellKind::SharedString | CellKind::FormulaString | CellKind::InlineString => {
            decoded(Scalar::from(Str::new(content)))
        }
        CellKind::Boolean => decoded(match content.trim() {
            "1" | "true" | "TRUE" => Scalar::from(true),
            "0" | "" | "false" | "FALSE" => Scalar::from(false),
            other => {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected 0 or 1 for a boolean cell, got {other:?}"),
                });
            }
        }),
        CellKind::Date => decoded(iso_scalar(content)?),
        CellKind::Error => decoded(Scalar::Null),
    })
}

/// The value one cell's facts prove under the column `field` declares.
///
/// The declared datatype is what the content is read as, through the value
/// contract every document leaf crosses: `12.50` under a decimal column keeps
/// its digits, `TRUE` under a boolean column is true, a number under a
/// temporal column is a serial in the date system, a `b` cell under a text
/// column is `true` or `false`, and any cell under a text column is its
/// displayed text. An error cell is a value no column holds. A null in a
/// required column is refused by the field's own contract.
///
/// # Errors
///
/// Returns the contract's refusal, unlocated: the caller names the cell.
pub(crate) fn field_scalar(
    field: &Field,
    kind: CellKind,
    format: NumberFormat,
    system: DateSystem,
    content: &str,
) -> Result<Scalar> {
    let dtype = field.dtype();
    let is_text_target = dtype.string_parameters().is_some();
    match kind {
        CellKind::Error => Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("expected a {dtype} value, got the error {content}"),
        }),
        CellKind::Number => {
            if content.trim().is_empty() {
                return field.scalar(Scalar::Null);
            }
            if is_text_target {
                let value = system.scalar_from_serial(parse_number(content)?, format)?;
                return field.scalar(Scalar::from(cell_text(&value).into_owned()));
            }
            match dtype.id().temporal_kind() {
                Some(
                    TemporalKind::Date
                    | TemporalKind::Time
                    | TemporalKind::DateTime
                    | TemporalKind::Duration,
                ) => field.scalar(system.temporal_from_serial(parse_number(content)?, dtype)?),
                _ if matches!(dtype, DataType::Boolean) => match content.trim() {
                    "1" => field.scalar(Scalar::from(true)),
                    "0" => field.scalar(Scalar::from(false)),
                    other => crate::text::prepare_text(Scalar::from(other), field),
                },
                _ => crate::text::prepare_text(Scalar::from(content), field),
            }
        }
        CellKind::SharedString | CellKind::FormulaString | CellKind::InlineString => {
            crate::text::prepare_text(Scalar::from(Str::new(content)), field)
        }
        CellKind::Boolean => {
            let held = wire_scalar(kind, format, system, content)?.into_scalar()?;
            if is_text_target {
                return field.scalar(Scalar::from(cell_text(&held).into_owned()));
            }
            field.scalar(held)
        }
        CellKind::Date => {
            if is_text_target || dtype.id().temporal_kind().is_some() {
                return crate::text::prepare_text(Scalar::from(content.trim()), field);
            }
            field.scalar(iso_scalar(content)?)
        }
    }
}

/// The reason of a refusal, without the location it carried: what a cell's
/// own refusal says once the cell is the location.
pub(crate) fn wire_reason(error: &Error) -> SmolStr {
    match error {
        Error::InvalidRecord { reason, .. } | Error::Parse { reason, .. } => reason.clone(),
        other => format_smolstr!("{other}"),
    }
}
