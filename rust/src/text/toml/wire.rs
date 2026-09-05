//! Natural TOML projection for structured values.

use std::io::Write;

use base64::Engine as _;

use crate::timezone::{civil_from_days, days_from_civil};
use crate::types::{Nested, Temporal};
use crate::{Error, Result, Scalar, TimeUnit, Timezone};

const SECONDS_PER_DAY: i64 = 86_400;
const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;

/// Convert a TOML table into the natural named-record value.
pub(super) fn decode_table(entries: Vec<(String, Scalar)>) -> Result<Scalar> {
    Scalar::from_record(entries)
}

/// Convert one native TOML date-time into its exact temporal value.
pub(super) fn datetime_value(datetime: toml::value::Datetime) -> Result<Scalar> {
    match (datetime.date, datetime.time, datetime.offset) {
        (Some(date), Some(time), offset) => datetime_from_parts(date, time, offset),
        (Some(date), None, None) => date_value(date),
        (None, Some(time), None) => time_value(time),
        _ => Err(codec_error("TOML date-time is not one of the four forms")),
    }
}

fn date_value(date: toml::value::Date) -> Result<Scalar> {
    i32::try_from(days_from_civil(
        i32::from(date.year),
        u32::from(date.month),
        u32::from(date.day),
    ))
    .map(Scalar::date32)
    .map_err(|_| codec_error("TOML date is outside the Date32 range"))
}

fn time_value(time: toml::value::Time) -> Result<Scalar> {
    let (seconds, nanosecond) = seconds_of_day(time);
    let (count, unit) = scaled_count(seconds, nanosecond)?;
    match unit {
        TimeUnit::Second | TimeUnit::Millisecond => Scalar::time32(
            i32::try_from(count).map_err(|_| codec_error("TOML time exceeds 32 bits"))?,
            unit,
            Timezone::NAIVE,
        ),
        TimeUnit::Microsecond | TimeUnit::Nanosecond => {
            Scalar::time64(count, unit, Timezone::NAIVE)
        }
        _ => unreachable!("a TOML fraction names a scalar unit"),
    }
}

fn datetime_from_parts(
    date: toml::value::Date,
    time: toml::value::Time,
    offset: Option<toml::value::Offset>,
) -> Result<Scalar> {
    let (within_day, nanosecond) = seconds_of_day(time);
    let local = days_from_civil(
        i32::from(date.year),
        u32::from(date.month),
        u32::from(date.day),
    )
    .checked_mul(SECONDS_PER_DAY)
    .and_then(|days| days.checked_add(within_day))
    .ok_or_else(|| codec_error("TOML datetime is out of range"))?;
    let (seconds, zone) = match offset {
        Some(offset) => {
            let seconds = offset_seconds(offset);
            (
                local
                    .checked_sub(i64::from(seconds))
                    .ok_or_else(|| codec_error("TOML datetime is out of range"))?,
                Timezone::from_offset(seconds)
                    .map_err(|_| codec_error("TOML offset is not a timezone"))?,
            )
        }
        None => (local, Timezone::NAIVE),
    };
    let (count, unit) = scaled_count(seconds, nanosecond)?;
    Scalar::datetime64(count, unit, zone)
}

const fn seconds_of_day(time: toml::value::Time) -> (i64, u32) {
    (
        time.hour as i64 * 3_600
            + time.minute as i64 * 60
            + match time.second {
                Some(second) => second as i64,
                None => 0,
            },
        match time.nanosecond {
            Some(nanosecond) => nanosecond,
            None => 0,
        },
    )
}

fn scaled_count(seconds: i64, nanosecond: u32) -> Result<(i64, TimeUnit)> {
    let (unit, per_second) = match nanosecond {
        0 => (TimeUnit::Second, 1),
        value if value % 1_000_000 == 0 => (TimeUnit::Millisecond, 1_000),
        value if value % 1_000 == 0 => (TimeUnit::Microsecond, 1_000_000),
        _ => (TimeUnit::Nanosecond, NANOSECONDS_PER_SECOND),
    };
    let fraction = i64::from(nanosecond) / (NANOSECONDS_PER_SECOND / per_second);
    seconds
        .checked_mul(per_second)
        .and_then(|count| count.checked_add(fraction))
        .map(|count| (count, unit))
        .ok_or_else(|| codec_error("TOML datetime exceeds its selected precision"))
}

const fn offset_seconds(offset: toml::value::Offset) -> i32 {
    match offset {
        toml::value::Offset::Z => 0,
        toml::value::Offset::Custom { minutes } => minutes as i32 * 60,
    }
}

/// Preflight the natural TOML document.
pub(super) fn check_depth(value: &Scalar, maximum: usize) -> Result<()> {
    observe_depth(1, maximum)?;
    match value {
        Scalar::Nested(Nested::Record(entries)) => {
            for value in entries.as_map().values() {
                check_value(value, 1, maximum)?;
            }
            Ok(())
        }
        Scalar::Nested(Nested::Mapping(entries))
            if entries
                .as_slice()
                .iter()
                .all(|(key, _)| key.as_str().is_some()) =>
        {
            for (_, value) in entries.as_slice() {
                check_value(value, 1, maximum)?;
            }
            Ok(())
        }
        _ => Err(codec_error("TOML document root must be a record")),
    }
}

fn check_value(value: &Scalar, parent: usize, maximum: usize) -> Result<()> {
    match value {
        Scalar::Null => Err(codec_error("TOML cannot represent null")),
        Scalar::Integer(value)
            if value
                .as_i128()
                .and_then(|value| i64::try_from(value).ok())
                .is_none() =>
        {
            Err(codec_error("TOML integer exceeds i64"))
        }
        Scalar::Nested(Nested::Sequence(values)) => {
            let depth = parent.saturating_add(1);
            observe_depth(depth, maximum)?;
            for value in values.as_slice() {
                check_value(value, depth, maximum)?;
            }
            Ok(())
        }
        Scalar::Nested(Nested::Record(entries)) => {
            let depth = parent.saturating_add(1);
            observe_depth(depth, maximum)?;
            for value in entries.as_map().values() {
                check_value(value, depth, maximum)?;
            }
            Ok(())
        }
        Scalar::Nested(Nested::Mapping(entries)) => {
            if !entries
                .as_slice()
                .iter()
                .all(|(key, _)| key.as_str().is_some())
            {
                return Err(codec_error("TOML table keys must be strings"));
            }
            let depth = parent.saturating_add(1);
            observe_depth(depth, maximum)?;
            for (_, value) in entries.as_slice() {
                check_value(value, depth, maximum)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn observe_depth(depth: usize, maximum: usize) -> Result<()> {
    if depth > super::MAX_PARSER_DEPTH {
        Err(codec_error("TOML nesting exceeds the parser limit"))
    } else if depth > maximum {
        Err(codec_error("nesting depth limit exceeded while encoding"))
    } else {
        Ok(())
    }
}

/// Write one natural TOML document.
pub(super) fn write_document<W: Write>(
    writer: &mut W,
    value: &Scalar,
    layout: Layout,
) -> Result<()> {
    match value {
        Scalar::Nested(Nested::Record(entries)) => {
            for (name, value) in entries.as_map() {
                write_quoted(writer, name)?;
                writer.write_all(b" = ")?;
                write_scalar(writer, value, layout, 0)?;
                writer.write_all(b"\n")?;
            }
            Ok(())
        }
        Scalar::Nested(Nested::Mapping(entries)) => {
            for (key, value) in entries.as_slice() {
                let key = key
                    .as_str()
                    .ok_or_else(|| codec_error("TOML table keys must be strings"))?;
                write_quoted(writer, key)?;
                writer.write_all(b" = ")?;
                write_scalar(writer, value, layout, 0)?;
                writer.write_all(b"\n")?;
            }
            Ok(())
        }
        _ => Err(codec_error("TOML document root must be a record")),
    }
}

#[derive(Clone, Copy)]
pub(super) struct Layout {
    unit: Option<&'static [u8]>,
}

impl From<crate::text::Formatting> for Layout {
    fn from(value: crate::text::Formatting) -> Self {
        Self {
            unit: value.indent().unit(),
        }
    }
}

fn write_scalar<W: Write>(
    writer: &mut W,
    value: &Scalar,
    layout: Layout,
    depth: usize,
) -> Result<()> {
    match value {
        Scalar::Null => return Err(codec_error("TOML cannot represent null")),
        Scalar::Boolean(value) => writer.write_all(if value.get() { b"true" } else { b"false" })?,
        Scalar::Integer(value) => write!(
            writer,
            "{}",
            value
                .as_i128()
                .and_then(|value| i64::try_from(value).ok())
                .ok_or_else(|| codec_error("TOML integer exceeds i64"))?
        )?,
        Scalar::Floating(value) => write_float(writer, value.as_f64())?,
        Scalar::Decimal(value) => {
            write_quoted(
                writer,
                &crate::types::decimal::scalars::decimal_text(value.coefficient(), value.scale()),
            )?;
        }
        Scalar::Text(value) => write_quoted(writer, value.as_str())?,
        Scalar::Ascii(value) => write_quoted(writer, value.as_str())?,
        Scalar::Uuid(value) => write_quoted(writer, &value.to_string())?,
        Scalar::Enum(value) => write_quoted(writer, value.as_str())?,
        Scalar::Bytes(value) => write_quoted(
            writer,
            &base64::engine::general_purpose::STANDARD.encode(value.as_bytes()),
        )?,
        Scalar::Geospatial(value) => write_quoted(
            writer,
            &base64::engine::general_purpose::STANDARD.encode(value.as_bytes()),
        )?,
        Scalar::Temporal(
            Temporal::Date32(_)
            | Temporal::Date64(_)
            | Temporal::Time32(_)
            | Temporal::Time64(_)
            | Temporal::DateTime64(_),
        ) => write_temporal(writer, value)?,
        Scalar::Temporal(Temporal::Duration32(value)) => {
            write_duration(
                writer,
                i64::from(value.count()),
                value.unit(),
                &value.timezone(),
            )?;
        }
        Scalar::Temporal(Temporal::Duration64(value)) => {
            write_duration(writer, value.count(), value.unit(), &value.timezone())?
        }
        Scalar::Temporal(Temporal::Interval(value)) => match value.unit() {
            TimeUnit::YearMonth => write!(writer, "{}", value.months())?,
            TimeUnit::DayTime => write!(
                writer,
                "[{}, {}]",
                value.days(),
                value.nanoseconds() / 1_000_000
            )?,
            TimeUnit::MonthDayNano => write!(
                writer,
                "[{}, {}, {}]",
                value.months(),
                value.days(),
                value.nanoseconds()
            )?,
            _ => return Err(codec_error("invalid interval layout")),
        },
        Scalar::Nested(Nested::Sequence(values)) => {
            write_sequence(writer, values.as_slice(), layout, depth)?
        }
        Scalar::Nested(Nested::Record(entries)) => {
            writer.write_all(b"{")?;
            for (index, (name, value)) in entries.as_map().iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                write_quoted(writer, name)?;
                writer.write_all(b" = ")?;
                write_scalar(writer, value, layout, depth)?;
            }
            writer.write_all(b"}")?;
        }
        Scalar::Nested(Nested::Mapping(entries)) => {
            writer.write_all(b"{")?;
            for (index, (key, value)) in entries.as_slice().iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                let key = key
                    .as_str()
                    .ok_or_else(|| codec_error("TOML table keys must be strings"))?;
                write_quoted(writer, key)?;
                writer.write_all(b" = ")?;
                write_scalar(writer, value, layout, depth)?;
            }
            writer.write_all(b"}")?;
        }
    }
    Ok(())
}

fn write_sequence<W: Write>(
    writer: &mut W,
    values: &[Scalar],
    layout: Layout,
    depth: usize,
) -> Result<()> {
    match layout.unit.filter(|_| !values.is_empty()) {
        None => {
            writer.write_all(b"[")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                write_scalar(writer, value, layout, depth)?;
            }
            writer.write_all(b"]")?;
        }
        Some(unit) => {
            writer.write_all(b"[\n")?;
            for value in values.iter() {
                write_units(writer, unit, depth + 1)?;
                write_scalar(writer, value, layout, depth + 1)?;
                writer.write_all(b",\n")?;
            }
            write_units(writer, unit, depth)?;
            writer.write_all(b"]")?;
        }
    }
    Ok(())
}

fn write_temporal<W: Write>(writer: &mut W, value: &Scalar) -> Result<()> {
    if let Some(datetime) = native_datetime(value) {
        write!(writer, "{datetime}")?;
        return Ok(());
    }
    match value {
        Scalar::Temporal(Temporal::Date32(value)) => {
            write!(writer, "{}", value.count())?;
        }
        Scalar::Temporal(Temporal::Date64(value)) => {
            write!(writer, "{}", value.count())?;
        }
        Scalar::Temporal(Temporal::Time32(value)) => {
            write_time(
                writer,
                i64::from(value.count()),
                value.unit(),
                &value.timezone(),
            )?;
        }
        Scalar::Temporal(Temporal::Time64(value)) => {
            write_time(writer, value.count(), value.unit(), &value.timezone())?
        }
        Scalar::Temporal(Temporal::DateTime64(value)) => {
            let text = if value.timezone().is_naive() {
                crate::types::ascii::iso::format_datetime(value.count(), value.unit())
            } else {
                crate::types::ascii::iso::format_timestamp(
                    value.count(),
                    value.unit(),
                    &value.timezone(),
                )
            };
            match text {
                Some(text) => write_quoted(writer, &text)?,
                None => write!(writer, "{}", value.count())?,
            }
        }
        _ => unreachable!("only datetime-like values reach this helper"),
    }
    Ok(())
}

fn write_duration<W: Write>(
    writer: &mut W,
    count: i64,
    unit: TimeUnit,
    zone: &Timezone,
) -> Result<()> {
    if zone.is_naive() {
        if let Some(text) = crate::types::ascii::iso::format_duration(count, unit) {
            return write_quoted(writer, &text);
        }
    } else {
        return Err(codec_error("duration cannot carry a timezone"));
    }
    write!(writer, "{count}")?;
    Ok(())
}

fn write_time<W: Write>(writer: &mut W, count: i64, unit: TimeUnit, zone: &Timezone) -> Result<()> {
    if !zone.is_naive() {
        return Err(codec_error(
            "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
        ));
    }
    let Some(text) = crate::types::ascii::iso::format_time(count, unit) else {
        write!(writer, "{count}")?;
        return Ok(());
    };
    write_quoted(writer, &text)
}

fn native_datetime(value: &Scalar) -> Option<toml::value::Datetime> {
    match value {
        Scalar::Temporal(Temporal::Date32(value)) => Some(toml::value::Datetime {
            date: Some(toml_date(i64::from(value.count()))?),
            time: None,
            offset: None,
        }),
        Scalar::Temporal(Temporal::Date64(value)) if value.count().rem_euclid(86_400_000) == 0 => {
            Some(toml::value::Datetime {
                date: Some(toml_date(value.count().div_euclid(86_400_000))?),
                time: None,
                offset: None,
            })
        }
        Scalar::Temporal(Temporal::Time32(value)) => {
            time_datetime(i64::from(value.count()), value.unit())
        }
        Scalar::Temporal(Temporal::Time64(value)) => time_datetime(value.count(), value.unit()),
        Scalar::Temporal(Temporal::DateTime64(value)) => {
            datetime_datetime(value.count(), value.unit(), &value.timezone())
        }
        _ => None,
    }
}

fn time_datetime(count: i64, unit: TimeUnit) -> Option<toml::value::Datetime> {
    let (seconds, nanosecond) = split_count(count, unit)?;
    (0..SECONDS_PER_DAY)
        .contains(&seconds)
        .then(|| toml::value::Datetime {
            date: None,
            time: Some(time_of_day(seconds, nanosecond)),
            offset: None,
        })
}

fn datetime_datetime(count: i64, unit: TimeUnit, zone: &Timezone) -> Option<toml::value::Datetime> {
    let (seconds, nanosecond) = split_count(count, unit)?;
    let (local, offset) = if zone.is_naive() {
        (seconds, None)
    } else {
        let offset = zone.is_fixed().then(|| zone.standard_offset()).flatten()?;
        if offset % 60 != 0 {
            return None;
        }
        (
            seconds.checked_add(i64::from(offset))?,
            Some(toml_offset(offset)?),
        )
    };
    Some(toml::value::Datetime {
        date: Some(toml_date(local.div_euclid(SECONDS_PER_DAY))?),
        time: Some(time_of_day(local.rem_euclid(SECONDS_PER_DAY), nanosecond)),
        offset,
    })
}

const fn split_count(count: i64, unit: TimeUnit) -> Option<(i64, u32)> {
    let per_second = match unit {
        TimeUnit::Second => 1,
        TimeUnit::Millisecond => 1_000,
        TimeUnit::Microsecond => 1_000_000,
        TimeUnit::Nanosecond => NANOSECONDS_PER_SECOND,
        _ => return None,
    };
    Some((
        count.div_euclid(per_second),
        (count.rem_euclid(per_second) * (NANOSECONDS_PER_SECOND / per_second)) as u32,
    ))
}

fn toml_date(days: i64) -> Option<toml::value::Date> {
    let (year, month, day) = civil_from_days(days);
    (0..=9_999).contains(&year).then_some(toml::value::Date {
        year: year as u16,
        month: month as u8,
        day: day as u8,
    })
}

const fn time_of_day(seconds: i64, nanosecond: u32) -> toml::value::Time {
    toml::value::Time {
        hour: (seconds / 3_600) as u8,
        minute: ((seconds % 3_600) / 60) as u8,
        second: Some((seconds % 60) as u8),
        nanosecond: if nanosecond == 0 {
            None
        } else {
            Some(nanosecond)
        },
    }
}

fn toml_offset(seconds: i32) -> Option<toml::value::Offset> {
    if seconds == 0 {
        Some(toml::value::Offset::Z)
    } else {
        i16::try_from(seconds / 60)
            .ok()
            .map(|minutes| toml::value::Offset::Custom { minutes })
    }
}

fn write_float<W: Write>(writer: &mut W, value: f64) -> Result<()> {
    if value.is_nan() {
        writer.write_all(b"nan")?;
    } else if value == f64::INFINITY {
        writer.write_all(b"inf")?;
    } else if value == f64::NEG_INFINITY {
        writer.write_all(b"-inf")?;
    } else {
        let spelling = serde_json::Number::from_f64(value)
            .ok_or_else(|| codec_error("float has no TOML spelling"))?
            .to_string();
        writer.write_all(spelling.as_bytes())?;
        if !spelling.contains(['.', 'e', 'E']) {
            writer.write_all(b".0")?;
        }
    }
    Ok(())
}

fn write_units<W: Write>(writer: &mut W, unit: &[u8], count: usize) -> Result<()> {
    for _ in 0..count {
        writer.write_all(unit)?;
    }
    Ok(())
}

fn write_quoted<W: Write>(writer: &mut W, value: &str) -> Result<()> {
    writer.write_all(b"\"")?;
    let mut start = 0;
    for (position, character) in value.char_indices() {
        let escaped = match character {
            '"' => Some("\\\""),
            '\\' => Some("\\\\"),
            '\u{0008}' => Some("\\b"),
            '\t' => Some("\\t"),
            '\n' => Some("\\n"),
            '\u{000c}' => Some("\\f"),
            '\r' => Some("\\r"),
            _ => None,
        };
        if let Some(escaped) = escaped {
            writer.write_all(&value.as_bytes()[start..position])?;
            writer.write_all(escaped.as_bytes())?;
            start = position + character.len_utf8();
        } else if character <= '\u{001f}' || character == '\u{007f}' {
            writer.write_all(&value.as_bytes()[start..position])?;
            write!(writer, "\\u{:04X}", u32::from(character))?;
            start = position + character.len_utf8();
        }
    }
    writer.write_all(&value.as_bytes()[start..])?;
    writer.write_all(b"\"")?;
    Ok(())
}

fn codec_error(reason: &'static str) -> Error {
    Error::Codec {
        format: "toml",
        position: 0,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests;
