//! Typed entry: what the text a user types into a cell is, the way Excel's
//! en-US entry reads it.
//!
//! [`Entry::from_text`] is the one reading: a formula after `=`, text after
//! a `'`, a boolean, an error, a number - with the format its spelling
//! suggests, `12%` a percentage and `$1,234.50` currency - a date, a time,
//! a datetime, or text for anything else.
//! [`Workbook::set_entry`](super::Workbook::set_entry) puts what it reads in
//! a cell and [`Workbook::entry_text`](super::Workbook::entry_text) spells a
//! cell back as it would be typed, so typing a cell's entry text again
//! leaves the cell as it is.

use std::fmt::Write as _;

use smol_str::SmolStr;

use crate::timezone::{civil_from_days, days_from_civil};
use crate::{Error, Result, Scalar, Str, TimeUnit, Timezone};

use super::cell::{CellRef, DateSystem, ExcelError};
use super::format::{DecimalParts, Digits, FormatCode, entry_number};
use super::formula::Formula;

/// What the text typed into a cell is.
///
/// ```
/// use yggdryl::excel::{CellRef, DateSystem, Entry};
/// use yggdryl::Scalar;
///
/// let host = CellRef::new(0, 0);
/// let read = |text: &str| Entry::from_text(text, host, DateSystem::Year1900);
/// assert_eq!(
///     read("12%")?,
///     Entry::Value { value: Scalar::from(0.12), format: Some("0%".into()) }
/// );
/// assert_eq!(
///     read("$1,234.50")?,
///     Entry::Value { value: Scalar::from(1234.5), format: Some("\"$\"#,##0.00".into()) }
/// );
/// assert_eq!(
///     read("1/2/2024")?,
///     Entry::Value { value: Scalar::date32(19_724), format: Some("m/d/yyyy".into()) }
/// );
/// assert_eq!(read("'007")?, Entry::Quoted("007".into()));
/// assert_eq!(read("abc")?, Entry::Text("abc".into()));
/// assert!(matches!(read("=SUM(A2:A9)")?, Entry::Formula(_)));
/// assert!(read("=SUM(A2").is_err());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// Nothing: the cell's content is cleared.
    Blank,
    /// A number, a boolean or a temporal, with the format code its spelling
    /// suggests: `0%` for `12%`, `m/d/yyyy` for a date.
    Value {
        /// The value.
        value: Scalar,
        /// The format the spelling suggests, `None` for none.
        format: Option<SmolStr>,
    },
    /// Text, as typed.
    Text(Str),
    /// Text typed after a `'`, which keeps it text whatever it spells: the
    /// cell's style states `quotePrefix`.
    Quoted(Str),
    /// One of the errors Excel spells, typed as it spells it.
    Error(ExcelError),
    /// A formula typed after `=`, or after a `+` or `-` that starts no
    /// number.
    Formula(Formula),
}

impl Entry {
    /// Read `text` typed into the cell at `host` of a workbook counting its
    /// dates from `system`, as en-US Excel reads it.
    ///
    /// | Text | Entry |
    /// | --- | --- |
    /// | empty | [`Entry::Blank`] |
    /// | `=…` | a formula |
    /// | `+` or `-` then no number | a formula, the sign its first operator |
    /// | `-`, `+`, `--`: signs and nothing else | [`Entry::Text`], as typed |
    /// | `'…` | [`Entry::Quoted`] |
    /// | `TRUE`, `FALSE`, any case | a boolean |
    /// | `#N/A` and the other seventeen errors, any case | [`Entry::Error`] |
    /// | `5`, `-5`, `+5`, `1,234.5`, `(12)` | a number, `(12)` being -12, fifteen significant digits kept and zeros past them |
    /// | `12%`, `12.5%` | a number over a hundred, `0%` or `0.00%` |
    /// | `$1,234`, `$1,234.50`, `-$5` | a number, `"$"#,##0` or `"$"#,##0.00` |
    /// | `1e3`, `1.5E-10` | a number, `0.00E+00` |
    /// | `1 1/2`, `0 3/4` | a number, `# ?/?` (`# ??/??` past nine) |
    /// | `1/2/2024`, `1-2-24`, `2024-01-02` | a date, `m/d/yyyy` |
    /// | `10:30`, `10:30:15` | a time, `h:mm` or `h:mm:ss` |
    /// | `10:30 PM`, `10:30:15 am` | a time, `h:mm AM/PM` or `h:mm:ss AM/PM` |
    /// | `25:30`, `36:00:00` | a duration past a day, `[h]:mm` or `[h]:mm:ss` |
    /// | `1/2/2024 10:30`, `1/2/2024 10:30:15 PM` | a datetime, `m/d/yyyy h:mm` |
    /// | anything else | [`Entry::Text`], as typed |
    ///
    /// Spaces around a number, a date or a time are ignored; a two-digit
    /// year is 2000 to 2029 below 30 and 1930 to 1999 from it; a date the
    /// system has no serial for is text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a formula that does not read, at the
    /// byte of `text` where it breaks.
    pub fn from_text(text: &str, host: CellRef, system: DateSystem) -> Result<Self> {
        if text.is_empty() {
            return Ok(Self::Blank);
        }
        if let Some(formula) = text.strip_prefix('=') {
            return Formula::from_entry(formula, host)
                .map(Self::Formula)
                .map_err(|error| match error {
                    Error::Parse {
                        target,
                        position,
                        reason,
                    } => Error::Parse {
                        target,
                        position: position + 1,
                        reason,
                    },
                    other => other,
                });
        }
        if let Some(quoted) = text.strip_prefix('\'') {
            return Ok(Self::Quoted(Str::new(quoted)));
        }
        let trimmed = text.trim();
        if trimmed.eq_ignore_ascii_case("TRUE") || trimmed.eq_ignore_ascii_case("FALSE") {
            return Ok(Self::Value {
                value: Scalar::from(trimmed.eq_ignore_ascii_case("TRUE")),
                format: None,
            });
        }
        if trimmed.starts_with('#') {
            let error = ExcelError::from_text(trimmed);
            if error != ExcelError::Unrecognized {
                return Ok(Self::Error(error));
            }
        }
        if let Some(parsed) = number(trimmed) {
            return Ok(Self::Value {
                value: Scalar::from(parsed.value),
                format: parsed.format.map(SmolStr::new),
            });
        }
        if let Some((value, format)) = temporal(trimmed, system) {
            return Ok(Self::Value {
                value,
                format: Some(SmolStr::new_static(format)),
            });
        }
        if trimmed.starts_with(['+', '-']) {
            // Signs and nothing else start no formula: the dash a ledger
            // writes for nothing is text.
            if trimmed
                .chars()
                .all(|character| matches!(character, '+' | '-') || character.is_whitespace())
            {
                return Ok(Self::Text(Str::new(text)));
            }
            return Formula::from_entry(text, host).map(Self::Formula);
        }
        Ok(Self::Text(Str::new(text)))
    }

    /// [`Self::from_text`] typed into a cell whose number format is
    /// `format`: under a text format (`@`) everything but nothing and a
    /// text after `'` is text as typed - a number, a date, a formula - as
    /// Excel keeps it.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::from_text`] returns.
    pub(crate) fn from_text_in(
        text: &str,
        host: CellRef,
        system: DateSystem,
        format: &FormatCode,
    ) -> Result<Self> {
        if format.is_text() && !text.is_empty() && !text.starts_with('\'') {
            return Ok(Self::Text(Str::new(text)));
        }
        Self::from_text(text, host, system)
    }

    /// What a cell at `host` holding `value` under the number format
    /// `format` is typed as - see
    /// [`Workbook::entry_text`](super::Workbook::entry_text) - so that
    /// [`Self::from_text_in`] reads it back as `value`: text after a `'`
    /// where the style states `quotePrefix` (`quoted`) or where it would
    /// read as anything but itself; a day before 1900 - serial 0 in the
    /// 1900 system, the 0th of January 1900 no date spells - as its serial
    /// under `system`.
    pub(crate) fn spell(
        value: &Scalar,
        format: &FormatCode,
        quoted: bool,
        host: CellRef,
        system: DateSystem,
    ) -> String {
        use crate::temporal::TemporalKind;

        let clock = |out: &mut String, millis: i64, meridiem: bool| {
            let (hours, rest) = (millis / 3_600_000, millis % 3_600_000);
            let (minutes, rest) = (rest / 60_000, rest % 60_000);
            let (seconds, below) = (rest / 1_000, rest % 1_000);
            let shown = if meridiem {
                match hours % 12 {
                    0 => 12,
                    other => other,
                }
            } else {
                hours
            };
            let _ = write!(out, "{shown}:{minutes:02}:{seconds:02}");
            if below != 0 {
                let _ = write!(out, ".{below:03}");
                while out.ends_with('0') {
                    out.pop();
                }
            }
            if meridiem {
                out.push_str(if hours < 12 { " AM" } else { " PM" });
            }
        };
        let day = |out: &mut String, days: i64| {
            let (year, month, day) = civil_from_days(days);
            let _ = write!(out, "{month}/{day}/{year}");
        };
        let percent = format.is_percent();
        match value {
            Scalar::Null => String::new(),
            Scalar::Boolean(flag) => String::from(if flag.get() { "TRUE" } else { "FALSE" }),
            crate::string_scalars!(text) => {
                let text = text.as_str();
                let itself = !quoted
                    && matches!(
                        Self::from_text_in(text, host, system, format),
                        Ok(Self::Text(read)) if read.as_str() == text
                    );
                if itself {
                    text.to_owned()
                } else {
                    format!("'{text}")
                }
            }
            _ => {
                if let Some(number) = super::cell::number_of(value) {
                    return entry_number(number, percent).to_string();
                }
                let millis =
                    value
                        .temporal_count()
                        .zip(value.temporal_unit())
                        .and_then(|(count, unit)| {
                            let per = crate::temporal::scalars::nanoseconds_per(unit)?;
                            i64::try_from((i128::from(count) * per).div_euclid(1_000_000)).ok()
                        });
                let naive = value
                    .temporal_timezone()
                    .is_some_and(|zone| zone.is_naive());
                let mut out = String::new();
                if millis.is_some_and(|millis| millis < -2_208_988_800_000) {
                    if let Ok(Some((serial, _))) = system.serial_of(value) {
                        return entry_number(serial, false).to_string();
                    }
                }
                match (value.id().temporal_kind(), millis) {
                    (Some(TemporalKind::Date), Some(millis)) => {
                        day(&mut out, millis.div_euclid(86_400_000));
                    }
                    (Some(TemporalKind::DateTime), Some(millis)) if naive => {
                        day(&mut out, millis.div_euclid(86_400_000));
                        out.push(' ');
                        clock(&mut out, millis.rem_euclid(86_400_000), true);
                    }
                    (Some(TemporalKind::Time), Some(millis)) => clock(&mut out, millis, true),
                    // An elapsed time is typed in at most four digits of hours.
                    (Some(TemporalKind::Duration), Some(millis))
                        if (0..36_000_000_000).contains(&millis) =>
                    {
                        clock(&mut out, millis, false);
                    }
                    (Some(TemporalKind::Duration), Some(millis)) => {
                        return entry_number(millis as f64 / 86_400_000.0, percent).to_string();
                    }
                    _ => return super::cell::cell_text(value).into_owned(),
                }
                out
            }
        }
    }
}

/// A number resolved once at the entry boundary. Currency is a
/// parser-proved spelling, independent of its display format.
pub(crate) struct ParsedNumber {
    pub(crate) value: f64,
    pub(crate) format: Option<&'static str>,
    pub(crate) currency: bool,
}

/// A number as en-US entry spells one, and the format its spelling
/// suggests.
pub(crate) fn number(text: &str) -> Option<ParsedNumber> {
    let mut rest = text;
    let mut negative = false;
    if let Some(inner) = rest
        .strip_prefix('(')
        .and_then(|inner| inner.strip_suffix(')'))
    {
        negative = true;
        rest = inner.trim();
    }
    let mut sign = |rest: &mut &str| -> Option<()> {
        if let Some(after) = rest.strip_prefix('-') {
            if negative {
                return None;
            }
            negative = true;
            *rest = after.trim_start();
        } else if let Some(after) = rest.strip_prefix('+') {
            *rest = after.trim_start();
        }
        Some(())
    };
    sign(&mut rest)?;
    let currency = if let Some(after) = rest.strip_prefix('$') {
        rest = after.trim_start();
        sign(&mut rest)?;
        true
    } else {
        false
    };
    let percent = if let Some(before) = rest.strip_suffix('%') {
        rest = before.trim_end();
        true
    } else {
        false
    };
    if currency && percent || rest.is_empty() {
        return None;
    }
    let signed = |value: f64| if negative { -value } else { value };
    // A mixed fraction: a whole number, a space, a numerator over a larger
    // denominator.
    if let Some((whole, fraction)) = rest.split_once(' ') {
        if currency || percent {
            return None;
        }
        let (top, bottom) = fraction.trim_start().split_once('/')?;
        let digits =
            |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
        if !(digits(whole) && digits(top) && digits(bottom)) || bottom.len() > 9 {
            return None;
        }
        let (whole, top, bottom) = (
            whole.parse::<f64>().ok()?,
            top.parse::<u64>().ok()?,
            bottom.parse::<u64>().ok()?,
        );
        if bottom == 0 || top >= bottom {
            return None;
        }
        let format = if bottom < 10 { "# ?/?" } else { "# ??/??" };
        return Some(ParsedNumber {
            value: signed(whole + top as f64 / bottom as f64),
            format: Some(format),
            currency: false,
        });
    }
    // Digits, thousands separators in the integer part only, a point, and
    // an exponent.
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(at) => (&rest[..at], Some(&rest[at + 1..])),
        None => (rest, None),
    };
    let (integer, decimals) = match mantissa.split_once('.') {
        Some((integer, decimals)) => (integer, Some(decimals)),
        None => (mantissa, None),
    };
    let all_digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
    if !decimals.is_none_or(all_digits) || integer.is_empty() && decimals.is_none_or(str::is_empty)
    {
        return None;
    }
    let grouped = integer.contains(',');
    if grouped {
        let mut groups = integer.split(',');
        let first = groups.next()?;
        if first.is_empty() || first.len() > 3 || !all_digits(first) {
            return None;
        }
        if !groups.all(|group| group.len() == 3 && all_digits(group)) {
            return None;
        }
    } else if !all_digits(integer) {
        return None;
    }
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits.is_empty() || !all_digits(digits) || currency || percent || grouped {
            return None;
        }
    }
    // Entry owns grouping and exponent grammar; Digits owns the same
    // fifteen-digit decimal intake used by formula literals.
    let shift = exponent
        .map_or(Some(0), |exponent| exponent.parse::<i32>().ok())?
        .checked_sub(if percent { 2 } else { 0 })?;
    let value = Digits::from_parts(DecimalParts {
        integer,
        fraction: decimals,
        exponent: i64::from(shift),
    })
    .as_f64()?;
    if !value.is_finite() {
        return None;
    }
    let decimal = decimals.is_some_and(|decimals| !decimals.is_empty());
    let format = if exponent.is_some() {
        Some("0.00E+00")
    } else if percent {
        Some(if decimal { "0.00%" } else { "0%" })
    } else if currency {
        Some(if decimal {
            "\"$\"#,##0.00"
        } else {
            "\"$\"#,##0"
        })
    } else {
        None
    };
    Some(ParsedNumber {
        value: signed(value),
        format,
        currency,
    })
}

/// Formula VALUE and typed entry share one temporal parse, then choose a
/// numeric serial or a public temporal scalar without losing phantom day60.
enum ParsedTemporal {
    Date(ParsedDate),
    Clock(i64, &'static str),
    DateTime(ParsedDate, i64),
}

impl ParsedTemporal {
    fn read(text: &str) -> Option<Self> {
        if let Some(date) = date(text) {
            return Some(Self::Date(date));
        }
        if let Some((millis, format)) = clock(text) {
            return Some(Self::Clock(millis, format));
        }
        let (day, time) = text.split_once(' ')?;
        let day = date(day)?;
        let (millis, _) = clock(time.trim_start())?;
        (millis < 86_400_000).then_some(Self::DateTime(day, millis))
    }

    fn scalar(self, system: DateSystem) -> Option<(Scalar, &'static str)> {
        match self {
            Self::Date(ParsedDate::Civil(days)) => {
                system.serial_from_millis(days * 86_400_000).ok()?;
                Some((Scalar::date32(i32::try_from(days).ok()?), "m/d/yyyy"))
            }
            Self::Clock(millis, format) if millis >= 86_400_000 => {
                let format = if format.contains(":ss") {
                    "[h]:mm:ss"
                } else {
                    "[h]:mm"
                };
                Some((
                    Scalar::duration64(millis, TimeUnit::Millisecond).ok()?,
                    format,
                ))
            }
            Self::Clock(millis, format) => Some((
                Scalar::time32(
                    i32::try_from(millis).ok()?,
                    TimeUnit::Millisecond,
                    Timezone::NAIVE,
                )
                .ok()?,
                format,
            )),
            Self::DateTime(ParsedDate::Civil(days), millis) => {
                let instant = days.checked_mul(86_400_000)?.checked_add(millis)?;
                system.serial_from_millis(instant).ok()?;
                Some((
                    Scalar::datetime64(instant, TimeUnit::Millisecond, Timezone::NAIVE).ok()?,
                    "m/d/yyyy h:mm",
                ))
            }
            Self::Date(ParsedDate::Phantom1900) | Self::DateTime(ParsedDate::Phantom1900, _) => {
                None
            }
        }
    }

    fn serial(self, system: DateSystem) -> std::result::Result<f64, ExcelError> {
        match self {
            Self::Date(date) => date.serial(system),
            Self::Clock(millis, _) => Ok(millis as f64 / 86_400_000.0),
            Self::DateTime(date, millis) => Ok(date.serial(system)? + millis as f64 / 86_400_000.0),
        }
    }
}

/// A date, time, duration or datetime projected into the public scalar domain.
pub(crate) fn temporal(text: &str, system: DateSystem) -> Option<(Scalar, &'static str)> {
    ParsedTemporal::read(text)?.scalar(system)
}

/// VALUE's fixed en-US intake. Unsupported current-year and sub-millisecond
/// spelling keeps its cache; invalid represented syntax is a value error.
pub(crate) fn value_number(
    text: &str,
    system: DateSystem,
) -> Option<std::result::Result<f64, ExcelError>> {
    let text = text.trim_matches(' ');
    if text.is_empty() || text.chars().any(char::is_control) {
        return Some(Err(ExcelError::Value));
    }
    if let Some(number) = number(text) {
        return Some(Ok(number.value));
    }
    if let Some(temporal) = ParsedTemporal::read(text) {
        return Some(temporal.serial(system));
    }
    if text.contains(':')
        && text.rsplit_once('.').is_some_and(|(_, tail)| {
            tail.len() > 3 && tail.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    if text.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'/' | b'-'))
        && text
            .bytes()
            .filter(|byte| matches!(*byte, b'/' | b'-'))
            .count()
            == 1
    {
        return None;
    }
    Some(Err(ExcelError::Value))
}

impl ParsedDate {
    fn serial(self, system: DateSystem) -> std::result::Result<f64, ExcelError> {
        match self {
            Self::Phantom1900 if system == DateSystem::Year1900 => Ok(60.0),
            Self::Civil(days) => system
                .serial_from_millis(days * 86_400_000)
                .map_err(|_| ExcelError::Value),
            Self::Phantom1900 => Err(ExcelError::Value),
        }
    }
}

enum ParsedDate {
    Civil(i64),
    Phantom1900,
}

/// A date as days since 1970-01-01: `m/d/yyyy` or `m-d-yyyy` with a two-
/// or four-digit year, or `yyyy-mm-dd`.
fn date(text: &str) -> Option<ParsedDate> {
    let separator = if text.contains('/') { '/' } else { '-' };
    let mut parts = text.split(separator);
    let (first, second, third) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let number = |text: &str, most: usize| -> Option<i64> {
        (!text.is_empty() && text.len() <= most && text.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let (year, month, day) = if first.len() == 4 {
        (number(first, 4)?, number(second, 2)?, number(third, 2)?)
    } else {
        let year = match third.len() {
            1 | 2 => {
                let short = number(third, 2)?;
                if short < 30 {
                    2000 + short
                } else {
                    1900 + short
                }
            }
            4 => number(third, 4)?,
            _ => return None,
        };
        (year, number(first, 2)?, number(second, 2)?)
    };
    if !(1900..=9999).contains(&year) {
        return None;
    }
    // A day the calendar has reads back as itself.
    let civil = (
        i32::try_from(year).ok()?,
        u32::try_from(month).ok()?,
        u32::try_from(day).ok()?,
    );
    if civil == (1900, 2, 29) {
        return Some(ParsedDate::Phantom1900);
    }
    let days = days_from_civil(civil.0, civil.1, civil.2);
    (civil_from_days(days) == civil).then_some(ParsedDate::Civil(days))
}

/// Formula DATEVALUE accepts year-first text without choosing a workbook
/// locale for ambiguous day/month spellings. None keeps the cached result.
pub(crate) fn date_value(
    text: &str,
    system: DateSystem,
) -> Option<std::result::Result<f64, ExcelError>> {
    let text = text.trim();
    let first = text.split(|c| c == '-' || c == '/').next().unwrap_or("");
    if first.len() != 4 {
        return None;
    }
    Some(match date(text) {
        Some(date) => date.serial(system),
        _ => Err(ExcelError::Value),
    })
}

/// Formula TIMEVALUE uses the same clock grammar as a typed cell entry.
pub(crate) fn time_value(text: &str) -> std::result::Result<f64, ExcelError> {
    match clock(text.trim()) {
        Some((millis, _)) if millis < 86_400_000 => Ok(millis as f64 / 86_400_000.0),
        _ => Err(ExcelError::Value),
    }
}
/// A clock as milliseconds since midnight - past a day for an elapsed
/// time - and the format it suggests: `h:mm`, `h:mm:ss`, a fraction of a
/// second, and `AM`/`PM` (`A`/`P`, any case, a space before it or not).
fn clock(text: &str) -> Option<(i64, &'static str)> {
    let trimmed = text.trim_end();
    let (body, meridiem) = [("am", false), ("pm", true), ("a", false), ("p", true)]
        .into_iter()
        .find_map(|(suffix, pm)| {
            let cut = trimmed.len().checked_sub(suffix.len())?;
            trimmed
                .get(cut..)
                .filter(|tail| tail.eq_ignore_ascii_case(suffix))
                .map(|_| (trimmed[..cut].trim_end(), Some(pm)))
        })
        .unwrap_or((trimmed, None));
    let mut parts = body.split(':');
    let (hours, minutes) = (parts.next()?, parts.next()?);
    let seconds = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let digits = |text: &str, most: usize| -> Option<i64> {
        (!text.is_empty() && text.len() <= most && text.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let mut hours = digits(hours, 4)?;
    let minutes = digits(minutes, 2).filter(|minutes| *minutes < 60)?;
    let (seconds, millis) = match seconds {
        None => (0, 0),
        Some(seconds) => {
            let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
            let whole = digits(whole, 2).filter(|seconds| *seconds < 60)?;
            if !fraction.bytes().all(|byte| byte.is_ascii_digit()) || fraction.len() > 3 {
                return None;
            }
            let millis = match fraction.len() {
                0 => 0,
                1 => fraction.parse::<i64>().ok()? * 100,
                2 => fraction.parse::<i64>().ok()? * 10,
                3 => fraction.parse::<i64>().ok()?,
                _ => unreachable!("clock fraction length was validated"),
            };
            (whole, millis)
        }
    };
    if let Some(pm) = meridiem {
        if !(0..=12).contains(&hours) {
            return None;
        }
        hours = match (hours, pm) {
            (12, false) => 0,
            (12, true) => 12,
            (hour, true) => hour + 12,
            (hour, false) => hour,
        };
    }
    let format = match (meridiem.is_some(), seconds_stated(text)) {
        (true, true) => "h:mm:ss AM/PM",
        (true, false) => "h:mm AM/PM",
        (false, true) => "h:mm:ss",
        (false, false) => "h:mm",
    };
    Some((
        (hours * 3_600 + minutes * 60 + seconds) * 1_000 + millis,
        format,
    ))
}

/// Whether a clock states its seconds: two colons.
fn seconds_stated(text: &str) -> bool {
    text.matches(':').count() == 2
}
