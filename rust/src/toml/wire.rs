//! TOML projection for the shared structured-text value.

use std::io::Write;

use base64::Engine as _;

use crate::{Error, Result, TimeUnit, Timezone, Value};

pub(super) const MARKER: &str = "$yggdryl";
const WIRE_VERSION: i64 = 1;

/// Seconds in one day, the modulus a calendar reading turns on.
const SECONDS_PER_DAY: i64 = 86_400;

/// Nanoseconds in one second, the modulus a fractional second turns on.
const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;

/// Convert one parsed TOML table, recognizing only exact Yggdryl envelopes.
pub(super) fn decode_table(entries: Vec<(String, Value)>) -> Result<Value> {
    if entries.len() == 1 && entries[0].0 == MARKER {
        if let Some(value) = decode_envelope(&entries[0].1) {
            return value;
        }
    }
    Value::from_mapping(
        entries
            .into_iter()
            .map(|(key, value)| (Value::from(key), value)),
    )
}

fn decode_envelope(body: &Value) -> Option<Result<Value>> {
    let fields = body.as_mapping()?;
    if field(fields, "version") != Some(&Value::I64(WIRE_VERSION)) {
        return None;
    }
    let kind = field(fields, "type").and_then(Value::as_str)?;
    // Every kind here names a `Value` variant, or - for `value` - says that the
    // payload beside it is one TOML already spells. There is no longer a kind
    // that names something outside the value model: the `tag` envelope named a
    // carrier that no longer exists, and rather than decode into a shape with no
    // home it is simply not an envelope any more. A document still spelling
    // `type = "tag"` therefore decodes as the ordinary mapping its syntax always
    // was, and, because `is_plain_mapping` refuses a lone `$yggdryl` key, that
    // mapping is written back through the `mapping` envelope unchanged.
    let names: &[&str] = match kind {
        "null" => &["version", "type"],
        "bytes" | "u64" | "i128" | "u128" | "decimal" | "date" | "time" | "timestamp"
        | "duration" | "mapping" | "value" => &["version", "type", "value"],
        _ => return None,
    };
    if !has_exact_fields(fields, names) {
        return None;
    }

    Some(match kind {
        "null" => Ok(Value::Null),
        "bytes" => decode_bytes(fields),
        "u64" => decode_integer(fields, "u64", str::parse::<u64>).map(Value::U64),
        "i128" => decode_integer(fields, "i128", str::parse::<i128>).map(Value::I128),
        "u128" => decode_integer(fields, "u128", str::parse::<u128>).map(Value::U128),
        "decimal" => decode_decimal(fields),
        "date" => decode_date(fields),
        "time" | "timestamp" | "duration" => decode_temporal(kind, fields),
        "mapping" => decode_mapping(fields),
        "value" => field(fields, "value")
            .cloned()
            .ok_or_else(|| codec_error("value envelope has no payload")),
        _ => return None,
    })
}

fn decode_bytes(fields: &[(Value, Value)]) -> Result<Value> {
    let encoded = field(fields, "value")
        .and_then(Value::as_str)
        .ok_or_else(|| codec_error("bytes envelope value must be base64 text"))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map(Value::from)
        .map_err(|_| codec_error("invalid base64 bytes envelope"))
}

fn decode_integer<T>(
    fields: &[(Value, Value)],
    name: &'static str,
    parse: impl FnOnce(&str) -> std::result::Result<T, std::num::ParseIntError>,
) -> Result<T> {
    let encoded = field(fields, "value")
        .and_then(Value::as_str)
        .ok_or_else(|| codec_error("integer envelope value must be decimal text"))?;
    parse(encoded).map_err(|_| {
        codec_error(match name {
            "u64" => "invalid u64 envelope",
            "i128" => "invalid i128 envelope",
            "u128" => "invalid u128 envelope",
            _ => "invalid integer envelope",
        })
    })
}

/// Read the `["<unscaled>", scale]` payload a decimal envelope carries.
///
/// The coefficient travels as text for the same reason `i128` does: it is wider
/// than the signed 64-bit integer TOML spells. The shape matches the JSON and
/// YAML decimal envelope exactly, so one document converts to another format
/// without the payload changing.
fn decode_decimal(fields: &[(Value, Value)]) -> Result<Value> {
    let parts = envelope_parts(fields)?;
    if parts.len() != 2 {
        return Err(codec_error(
            "decimal envelope value must be [unscaled, scale]",
        ));
    }
    let unscaled = parts[0]
        .as_str()
        .and_then(|unscaled| unscaled.parse::<i128>().ok())
        .ok_or_else(|| codec_error("decimal envelope has no unscaled integer"))?;
    let scale = parts[1]
        .as_i128()
        .and_then(|scale| i8::try_from(scale).ok())
        .ok_or_else(|| codec_error("decimal envelope has no scale"))?;
    Ok(Value::decimal(unscaled, scale))
}

/// Read the day count a date envelope carries.
fn decode_date(fields: &[(Value, Value)]) -> Result<Value> {
    field(fields, "value")
        .and_then(Value::as_i128)
        .and_then(|days| i32::try_from(days).ok())
        .map(Value::date)
        .ok_or_else(|| codec_error("date envelope value must be a day count"))
}

/// Read the `["<unit>", count]` payload a time, timestamp, or duration carries.
fn decode_temporal(kind: &str, fields: &[(Value, Value)]) -> Result<Value> {
    let parts = envelope_parts(fields)?;
    let unit = parts
        .first()
        .and_then(Value::as_str)
        .and_then(|unit| TimeUnit::from_str(unit).ok())
        .ok_or_else(|| codec_error("temporal envelope has no unit"))?;
    let count = parts
        .get(1)
        .and_then(Value::as_i128)
        .and_then(|count| i64::try_from(count).ok())
        .ok_or_else(|| codec_error("temporal envelope has no count"))?;
    match (kind, parts.len()) {
        ("time", 2) => Ok(Value::time(count, unit)),
        ("duration", 2) => Ok(Value::duration(count, unit)),
        ("timestamp", 2) => Ok(Value::timestamp_in(count, unit, None)),
        ("timestamp", 3) => {
            let zone = parts[2]
                .as_str()
                .ok_or_else(|| codec_error("timestamp envelope zone is not text"))?;
            Value::timestamp(count, unit, Some(zone))
        }
        _ => Err(codec_error("temporal envelope has the wrong shape")),
    }
}

/// Borrow the array a decimal or temporal envelope carries as its payload.
fn envelope_parts(fields: &[(Value, Value)]) -> Result<&[Value]> {
    field(fields, "value")
        .and_then(Value::as_sequence)
        .ok_or_else(|| codec_error("typed envelope value must be an array"))
}

fn decode_mapping(fields: &[(Value, Value)]) -> Result<Value> {
    let entries = field(fields, "value")
        .and_then(Value::as_sequence)
        .ok_or_else(|| codec_error("mapping envelope value must be an entry sequence"))?;
    let mut decoded = Vec::with_capacity(entries.len());
    for entry in entries {
        let pair = entry
            .as_sequence()
            .filter(|pair| pair.len() == 2)
            .ok_or_else(|| codec_error("mapping envelope entry must be a pair"))?;
        decoded.push((pair[0].clone(), pair[1].clone()));
    }
    Value::from_mapping(decoded).map_err(|_| codec_error("mapping envelope has duplicate keys"))
}

fn field<'a>(entries: &'a [(Value, Value)], name: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find_map(|(key, value)| (key.as_str() == Some(name)).then_some(value))
}

fn has_exact_fields(entries: &[(Value, Value)], names: &[&str]) -> bool {
    entries.len() == names.len()
        && entries
            .iter()
            .all(|(key, _)| key.as_str().is_some_and(|key| names.contains(&key)))
}

/// Convert one native TOML date-time into the temporal it names.
///
/// TOML's four forms are the four shapes the value already holds, so each one
/// converts into its own variant rather than into a name over a string. That is
/// why nothing here keeps the original spelling: the parts are the value, and
/// the writer below spells them again from the parts.
///
/// # Errors
///
/// Returns an error when the reading needs more precision than an `i64` count
/// of its own unit can hold, when the offset is not a usable zone, or when the
/// parts are not one of TOML's four forms.
pub(super) fn datetime_value(datetime: toml::value::Datetime) -> Result<Value> {
    match (datetime.date, datetime.time, datetime.offset) {
        (Some(date), Some(time), offset) => timestamp_value(date, time, offset),
        (Some(date), None, None) => date_value(date),
        (None, Some(time), None) => time_value(time),
        _ => Err(codec_error("TOML date-time is not one of the four forms")),
    }
}

/// Build the day count a TOML local date names.
fn date_value(date: toml::value::Date) -> Result<Value> {
    i32::try_from(days_from_civil(date))
        .map(Value::date)
        .map_err(|_| codec_error("TOML date is outside the representable day count"))
}

/// Build the count since midnight a TOML local time names.
fn time_value(time: toml::value::Time) -> Result<Value> {
    let (seconds, nanosecond) = seconds_of_day(time);
    let (count, unit) = scaled_count(seconds, nanosecond)?;
    Ok(Value::time(count, unit))
}

/// Build the instant a TOML offset or local date-time names.
///
/// The count is always relative to UTC, as Arrow and [`Value::timestamp`] both
/// define it, so an offset reading is moved back to UTC and the offset itself
/// is kept as the zone. A local date-time has no offset, so its count is the
/// wall-clock reading and its zone is absent.
fn timestamp_value(
    date: toml::value::Date,
    time: toml::value::Time,
    offset: Option<toml::value::Offset>,
) -> Result<Value> {
    let (within_day, nanosecond) = seconds_of_day(time);
    // A TOML year is four digits, so this product is nowhere near overflowing.
    let local = days_from_civil(date) * SECONDS_PER_DAY + within_day;
    let (seconds, zone) = match offset {
        Some(offset) => {
            let offset = offset_seconds(offset);
            let zone = Timezone::from_offset(offset)
                .map_err(|_| codec_error("TOML date-time offset is not a usable zone"))?;
            (local - i64::from(offset), Some(zone))
        }
        None => (local, None),
    };
    let (count, unit) = scaled_count(seconds, nanosecond)?;
    Ok(Value::timestamp_in(count, unit, zone))
}

/// Split a TOML time into whole seconds since midnight and its nanoseconds.
///
/// TOML 1.1 lets a reading stop at the minute, and a leap second is spelled as
/// the sixtieth second of its minute. Both are read as the plain arithmetic
/// they describe, so `23:59:60` is the first second of the next day.
const fn seconds_of_day(time: toml::value::Time) -> (i64, u32) {
    let second = match time.second {
        Some(second) => second as i64,
        None => 0,
    };
    let nanosecond = match time.nanosecond {
        Some(nanosecond) => nanosecond,
        None => 0,
    };
    (
        time.hour as i64 * 3_600 + time.minute as i64 * 60 + second,
        nanosecond,
    )
}

/// Scale whole seconds and a nanosecond remainder into one exact count.
///
/// The unit chosen is the coarsest one that drops no digit, which is also the
/// unit the spelling names: a whole second is seconds, `.123` is milliseconds,
/// and `.123456789` is nanoseconds. Keeping the coarsest unit is what lets a
/// year far from the epoch survive, because a nanosecond count only reaches
/// from 1677 to 2262 while TOML spells every year from 0000 to 9999.
///
/// # Errors
///
/// Returns an error when the count does not fit `i64` at that unit, which is a
/// distant year written to sub-second precision.
fn scaled_count(seconds: i64, nanosecond: u32) -> Result<(i64, TimeUnit)> {
    let (unit, per_second) = match nanosecond {
        0 => (TimeUnit::Second, 1),
        nanosecond if nanosecond % 1_000_000 == 0 => (TimeUnit::Millisecond, 1_000),
        nanosecond if nanosecond % 1_000 == 0 => (TimeUnit::Microsecond, 1_000_000),
        _ => (TimeUnit::Nanosecond, NANOSECONDS_PER_SECOND),
    };
    let fraction = i64::from(nanosecond) / (NANOSECONDS_PER_SECOND / per_second);
    seconds
        .checked_mul(per_second)
        .and_then(|count| count.checked_add(fraction))
        .map(|count| (count, unit))
        .ok_or_else(|| codec_error("TOML date-time is outside the range its precision can hold"))
}

/// Read a TOML offset as the seconds east of UTC it stands for.
const fn offset_seconds(offset: toml::value::Offset) -> i32 {
    match offset {
        toml::value::Offset::Z => 0,
        toml::value::Offset::Custom { minutes } => minutes as i32 * 60,
    }
}

/// Return the native TOML spelling of a temporal, when TOML has one.
///
/// TOML writes an instant as a calendar reading, so the temporals it can spell
/// are the ones whose year lands in its four digits, whose zone is a fixed
/// offset rather than a place, and whose unit is a resolution rather than a
/// calendar interval. Every other temporal takes the `$yggdryl` envelope, like
/// every other value TOML has no syntax for, so nothing is ever reinterpreted
/// to fit and no round trip changes the value.
fn native_datetime(value: &Value) -> Option<toml::value::Datetime> {
    match value {
        Value::Date(days) => Some(toml::value::Datetime {
            date: Some(civil_from_days(i64::from(*days))?),
            time: None,
            offset: None,
        }),
        Value::Time(count, unit) => {
            let (seconds, nanosecond) = split_count(*count, *unit)?;
            (0..SECONDS_PER_DAY)
                .contains(&seconds)
                .then(|| toml::value::Datetime {
                    date: None,
                    time: Some(time_of_day(seconds, nanosecond)),
                    offset: None,
                })
        }
        Value::Timestamp(count, unit, zone) => {
            let offset = fixed_offset(zone)?;
            let (seconds, nanosecond) = split_count(*count, *unit)?;
            let local = seconds.checked_add(i64::from(offset))?;
            Some(toml::value::Datetime {
                date: Some(civil_from_days(local.div_euclid(SECONDS_PER_DAY))?),
                time: Some(time_of_day(local.rem_euclid(SECONDS_PER_DAY), nanosecond)),
                offset: Some(toml_offset(offset)),
            })
        }
        // The naive reading is TOML's local date-time, offset and all absent.
        Value::DateTime(count, unit) => {
            let (seconds, nanosecond) = split_count(*count, *unit)?;
            Some(toml::value::Datetime {
                date: Some(civil_from_days(seconds.div_euclid(SECONDS_PER_DAY))?),
                time: Some(time_of_day(seconds.rem_euclid(SECONDS_PER_DAY), nanosecond)),
                offset: None,
            })
        }
        // A duration is elapsed time rather than a reading on a calendar, and no
        // other kind is a temporal at all, so neither has a date-time spelling.
        _ => None,
    }
}

/// Split a temporal count into whole seconds and the nanoseconds after them.
///
/// Restating the count in nanoseconds first would be shorter and wrong: an
/// `i64` nanosecond count only spans 1677 to 2262, so every older or later year
/// TOML can spell would be refused. Splitting at the second keeps the whole
/// range and still carries the fraction exactly.
const fn split_count(count: i64, unit: TimeUnit) -> Option<(i64, u32)> {
    let per_second = match unit {
        TimeUnit::Second => 1,
        TimeUnit::Millisecond => 1_000,
        TimeUnit::Microsecond => 1_000_000,
        TimeUnit::Nanosecond => NANOSECONDS_PER_SECOND,
        // A calendar interval is not a fixed number of seconds, so it has no
        // reading on a clock at all.
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => return None,
    };
    let fraction = count.rem_euclid(per_second) * (NANOSECONDS_PER_SECOND / per_second);
    Some((count.div_euclid(per_second), fraction as u32))
}

/// Build the TOML clock reading a count of seconds into a day names.
const fn time_of_day(seconds: i64, nanosecond: u32) -> toml::value::Time {
    toml::value::Time {
        hour: (seconds / 3_600) as u8,
        minute: ((seconds % 3_600) / 60) as u8,
        second: Some((seconds % 60) as u8),
        // A zero fraction is spelled as no fraction, which is what TOML's own
        // printer emits and what the parser reads back as zero.
        nanosecond: if nanosecond == 0 {
            None
        } else {
            Some(nanosecond)
        },
    }
}

/// Return the fixed offset a zone stands for, or `None` when it names a place.
///
/// TOML spells an offset and never a zone name, so `Europe/Paris` has no native
/// spelling. Writing the offset that zone happens to be at would throw the name
/// away silently; the envelope keeps it instead.
fn fixed_offset(zone: &Timezone) -> Option<i32> {
    zone.is_fixed().then(|| zone.standard_offset()).flatten()
}

/// Build the TOML offset a count of seconds east of UTC stands for.
const fn toml_offset(seconds: i32) -> toml::value::Offset {
    if seconds == 0 {
        // `Z` and `+00:00` name one zone, and `Z` is the spelling a zone that
        // canonicalized to UTC goes back out as.
        toml::value::Offset::Z
    } else {
        toml::value::Offset::Custom {
            minutes: (seconds / 60) as i16,
        }
    }
}

/// Return the civil date a day count since the Unix epoch names.
///
/// This is Howard Hinnant's `civil_from_days`, which is exact for every day in
/// range and needs no table. It answers `None` outside TOML's four-digit year,
/// because that is the only range TOML has syntax for.
const fn civil_from_days(days: i64) -> Option<toml::value::Date> {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_of_year = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_of_year + 2) / 5 + 1;
    let month = if month_of_year < 10 {
        month_of_year + 3
    } else {
        month_of_year - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    if year < 0 || year > 9_999 {
        return None;
    }
    Some(toml::value::Date {
        year: year as u16,
        month: month as u8,
        day: day as u8,
    })
}

/// Return the day count since the Unix epoch a civil date names.
///
/// This is Hinnant's `days_from_civil`, the exact inverse of the conversion
/// above. A TOML year is four digits, so nothing here can overflow.
const fn days_from_civil(date: toml::value::Date) -> i64 {
    let month = date.month as i64;
    let year = date.year as i64 - if month <= 2 { 1 } else { 0 };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + date.day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(super) fn write_document<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    // A record at the root is the plain table its field names spell.
    if let Value::Record(..) = value {
        return write_document(writer, &value.record_to_mapping());
    }
    if let Value::Mapping(entries) = value {
        if is_plain_mapping(entries) {
            for (key, value) in entries.iter() {
                let Some(key) = key.as_str() else {
                    return Err(codec_error("plain TOML mapping key is not text"));
                };
                write_quoted(writer, key)?;
                writer.write_all(b" = ")?;
                write_value(writer, value)?;
                writer.write_all(b"\n")?;
            }
            return Ok(());
        }
    }

    write_quoted(writer, MARKER)?;
    writer.write_all(b" = ")?;
    write_root_envelope(writer, value)?;
    writer.write_all(b"\n")?;
    Ok(())
}

/// Return whether a value reaches the wire inside a `$yggdryl` envelope.
///
/// This is the single question the writer and the depth preflight both ask, so
/// the two can never disagree about how many containers a value costs.
fn is_enveloped(value: &Value) -> bool {
    match value {
        Value::Bool(_)
        | Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::F32(_)
        | Value::F64(_)
        | Value::String(_)
        | Value::Sequence(_) => false,
        Value::Mapping(entries) => !is_plain_mapping(entries),
        // A record lowers to a string-keyed mapping, which is a plain table.
        Value::Record(..) => false,
        Value::Date(_) | Value::Time(..) | Value::DateTime(..) => native_datetime(value).is_none(),
        // A zoned instant TOML cannot spell natively still has its classic
        // string; only a reading with neither takes the envelope.
        Value::Timestamp(count, unit, zone) => {
            native_datetime(value).is_none()
                && crate::generic::iso::format_timestamp(*count, *unit, zone).is_none()
        }
        Value::Duration(count, unit) => {
            crate::generic::iso::format_duration(*count, *unit).is_none()
        }
        Value::Null
        | Value::U64(_)
        | Value::I128(_)
        | Value::U128(_)
        | Value::Bytes(_)
        | Value::Decimal(..) => true,
    }
}

/// The container levels an envelope payload adds beyond the envelope body.
///
/// A payload that is one scalar - base64 text, a decimal spelled as text, a day
/// count - adds nothing. The `["<unit>", count]` and `["<unscaled>", scale]`
/// payloads are TOML arrays, which the parser counts as one more container on
/// the way back, so the preflight has to count it on the way out.
const fn envelope_payload_depth(value: &Value) -> usize {
    match value {
        Value::Decimal(..) | Value::Time(..) | Value::Timestamp(..) | Value::Duration(..) => 1,
        _ => 0,
    }
}

/// Preflight the structural depth of the exact TOML wire projection.
pub(super) fn check_depth(value: &Value, maximum: usize) -> Result<()> {
    observe_depth(1, maximum)?; // Every TOML document has a root table, including empty input.
    if let Value::Mapping(entries) = value {
        if is_plain_mapping(entries) {
            for (_, value) in entries.iter() {
                check_value_depth(value, 1, maximum)?;
            }
            return Ok(());
        }
    }
    check_root_body_depth(value, 2, maximum)
}

fn check_root_body_depth(value: &Value, body_depth: usize, maximum: usize) -> Result<()> {
    observe_depth(body_depth, maximum)?;
    match value {
        Value::Mapping(entries) => check_mapping_body_depth(entries, body_depth, maximum),
        // The root envelope is the body table itself, so a value that needs one
        // has already been charged for it and only its payload is left.
        value if is_enveloped(value) => observe_depth(
            body_depth.saturating_add(envelope_payload_depth(value)),
            maximum,
        ),
        // A value TOML spells natively rides inside that same body table.
        value => check_value_depth(value, body_depth, maximum),
    }
}

fn check_value_depth(value: &Value, parent_depth: usize, maximum: usize) -> Result<()> {
    match value {
        Value::Sequence(values) => {
            let depth = parent_depth.saturating_add(1);
            observe_depth(depth, maximum)?;
            for value in values.iter() {
                check_value_depth(value, depth, maximum)?;
            }
        }
        Value::Mapping(entries) if is_plain_mapping(entries) => {
            let depth = parent_depth.saturating_add(1);
            observe_depth(depth, maximum)?;
            for (_, value) in entries.iter() {
                check_value_depth(value, depth, maximum)?;
            }
        }
        // A record costs exactly what its plain-table spelling costs.
        Value::Record(_, values) => {
            let depth = parent_depth.saturating_add(1);
            observe_depth(depth, maximum)?;
            for value in values.iter() {
                check_value_depth(value, depth, maximum)?;
            }
        }
        // A temporal TOML spells natively - or as its classic ISO string -
        // goes out as one token, so it costs no more than the string beside it.
        Value::Date(_)
        | Value::Time(..)
        | Value::Timestamp(..)
        | Value::DateTime(..)
        | Value::Duration(..)
            if !is_enveloped(value) => {}
        Value::Mapping(entries) => {
            let wrapper_depth = parent_depth.saturating_add(1);
            let body_depth = wrapper_depth.saturating_add(1);
            observe_depth(wrapper_depth, maximum)?;
            observe_depth(body_depth, maximum)?;
            check_mapping_body_depth(entries, body_depth, maximum)?;
        }
        Value::Null
        | Value::U64(_)
        | Value::I128(_)
        | Value::U128(_)
        | Value::Bytes(_)
        | Value::Decimal(..)
        | Value::Date(_)
        | Value::Time(..)
        | Value::Timestamp(..)
        | Value::DateTime(..)
        | Value::Duration(..) => {
            let wrapper_depth = parent_depth.saturating_add(1);
            let body_depth = wrapper_depth.saturating_add(1);
            observe_depth(wrapper_depth, maximum)?;
            observe_depth(body_depth, maximum)?;
            observe_depth(
                body_depth.saturating_add(envelope_payload_depth(value)),
                maximum,
            )?;
        }
        Value::Bool(_)
        | Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::F32(_)
        | Value::F64(_)
        | Value::String(_) => {}
    }
    Ok(())
}

fn check_mapping_body_depth(
    entries: &[(Value, Value)],
    body_depth: usize,
    maximum: usize,
) -> Result<()> {
    let entries_depth = body_depth.saturating_add(1);
    observe_depth(entries_depth, maximum)?;
    for (key, value) in entries {
        let pair_depth = entries_depth.saturating_add(1);
        observe_depth(pair_depth, maximum)?;
        check_value_depth(key, pair_depth, maximum)?;
        check_value_depth(value, pair_depth, maximum)?;
    }
    Ok(())
}

fn observe_depth(depth: usize, maximum: usize) -> Result<()> {
    if depth > super::MAX_PARSER_DEPTH {
        Err(codec_error(
            "TOML nesting exceeds the parser hard limit of 64 while encoding",
        ))
    } else if depth > maximum {
        Err(codec_error("nesting depth limit exceeded while encoding"))
    } else {
        Ok(())
    }
}

fn write_root_envelope<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    if is_enveloped(value) {
        return write_envelope_body(writer, value);
    }
    writer.write_all(b"{ version = 1, type = \"value\", value = ")?;
    write_value(writer, value)?;
    writer.write_all(b" }")?;
    Ok(())
}

fn write_value<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    match value {
        // A record writes as the plain table its field names spell.
        Value::Record(..) => write_value(writer, &value.record_to_mapping())?,
        Value::Bool(value) => writer.write_all(if *value { b"true" } else { b"false" })?,
        Value::I8(value) => write!(writer, "{value}")?,
        Value::I16(value) => write!(writer, "{value}")?,
        Value::I32(value) => write!(writer, "{value}")?,
        Value::I64(value) => write!(writer, "{value}")?,
        Value::U8(value) => write!(writer, "{value}")?,
        Value::U16(value) => write!(writer, "{value}")?,
        Value::U32(value) => write!(writer, "{value}")?,
        Value::F32(value) => write_float(writer, value.as_f64())?,
        Value::F64(value) => write_float(writer, value.as_f64())?,
        Value::String(value) => write_quoted(writer, value)?,
        Value::Sequence(values) => {
            writer.write_all(b"[")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                write_value(writer, value)?;
            }
            writer.write_all(b"]")?;
        }
        Value::Mapping(entries) if is_plain_mapping(entries) => {
            writer.write_all(b"{")?;
            for (index, (key, value)) in entries.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                let Some(key) = key.as_str() else {
                    return Err(codec_error("plain TOML mapping key is not text"));
                };
                write_quoted(writer, key)?;
                writer.write_all(b" = ")?;
                write_value(writer, value)?;
            }
            writer.write_all(b"}")?;
        }
        // TOML spells a date, a time, and a date-time itself, so a temporal it
        // can hold goes out in that syntax rather than in an envelope.
        Value::Date(_) | Value::Time(..) | Value::DateTime(..) => match native_datetime(value) {
            Some(datetime) => write!(writer, "{datetime}")?,
            None => write_wrapped_envelope(writer, value)?,
        },
        // A zoned instant whose zone is a place has no native TOML offset, but
        // it still has its classic string, brackets and all.
        Value::Timestamp(count, unit, zone) => match native_datetime(value) {
            Some(datetime) => write!(writer, "{datetime}")?,
            None => match crate::generic::iso::format_timestamp(*count, *unit, zone) {
                Some(spelled) => write_quoted(writer, &spelled)?,
                None => write_wrapped_envelope(writer, value)?,
            },
        },
        // TOML has no duration syntax, so a duration is its classic string.
        Value::Duration(count, unit) => match crate::generic::iso::format_duration(*count, *unit) {
            Some(spelled) => write_quoted(writer, &spelled)?,
            None => write_wrapped_envelope(writer, value)?,
        },
        Value::Null
        | Value::U64(_)
        | Value::I128(_)
        | Value::U128(_)
        | Value::Bytes(_)
        | Value::Decimal(..)
        | Value::Mapping(_) => write_wrapped_envelope(writer, value)?,
    }
    Ok(())
}

/// Write one value as the inline table that holds its `$yggdryl` envelope.
fn write_wrapped_envelope<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    writer.write_all(b"{ ")?;
    write_quoted(writer, MARKER)?;
    writer.write_all(b" = ")?;
    write_envelope_body(writer, value)?;
    writer.write_all(b" }")?;
    Ok(())
}

fn write_envelope_body<W: Write>(writer: &mut W, value: &Value) -> Result<()> {
    match value {
        // A record is never enveloped; its mapping spelling answers anyway.
        Value::Record(..) => write_envelope_body(writer, &value.record_to_mapping())?,
        Value::Null => writer.write_all(b"{ version = 1, type = \"null\" }")?,
        Value::Bytes(value) => {
            writer.write_all(b"{ version = 1, type = \"bytes\", value = ")?;
            write_quoted(
                writer,
                &base64::engine::general_purpose::STANDARD.encode(value),
            )?;
            writer.write_all(b" }")?;
        }
        Value::U64(value) => {
            writer.write_all(b"{ version = 1, type = \"u64\", value = ")?;
            write!(writer, "\"{value}\"")?;
            writer.write_all(b" }")?;
        }
        Value::I128(value) => {
            writer.write_all(b"{ version = 1, type = \"i128\", value = ")?;
            write!(writer, "\"{value}\"")?;
            writer.write_all(b" }")?;
        }
        Value::U128(value) => {
            writer.write_all(b"{ version = 1, type = \"u128\", value = ")?;
            write!(writer, "\"{value}\"")?;
            writer.write_all(b" }")?;
        }
        Value::Decimal(unscaled, scale) => {
            writer.write_all(b"{ version = 1, type = \"decimal\", value = [")?;
            write!(writer, "\"{unscaled}\", {scale}")?;
            writer.write_all(b"] }")?;
        }
        Value::Date(days) => {
            writer.write_all(b"{ version = 1, type = \"date\", value = ")?;
            write!(writer, "{days}")?;
            writer.write_all(b" }")?;
        }
        Value::Time(count, unit) => write_temporal_body(writer, "time", *count, *unit, None)?,
        Value::Duration(count, unit) => {
            write_temporal_body(writer, "duration", *count, *unit, None)?;
        }
        Value::Timestamp(count, unit, zone) => {
            write_temporal_body(writer, "timestamp", *count, *unit, Some(zone))?;
        }
        Value::DateTime(count, unit) => {
            write_temporal_body(writer, "timestamp", *count, *unit, None)?;
        }
        Value::Mapping(entries) => {
            writer.write_all(b"{ version = 1, type = \"mapping\", value = [")?;
            for (index, (key, value)) in entries.iter().enumerate() {
                if index != 0 {
                    writer.write_all(b", ")?;
                }
                writer.write_all(b"[")?;
                write_value(writer, key)?;
                writer.write_all(b", ")?;
                write_value(writer, value)?;
                writer.write_all(b"]")?;
            }
            writer.write_all(b"] }")?;
        }
        Value::Bool(_)
        | Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::F32(_)
        | Value::F64(_)
        | Value::String(_)
        | Value::Sequence(_) => {
            return Err(codec_error(
                "native TOML value does not require an envelope",
            ));
        }
    }
    Ok(())
}

/// Write the `["<unit>", count]` body a temporal envelope carries.
///
/// The unit travels as its own name rather than as a number so the payload
/// reads the same here as it does in JSON and YAML, and a zone follows the
/// count when the timestamp has one.
fn write_temporal_body<W: Write>(
    writer: &mut W,
    kind: &str,
    count: i64,
    unit: TimeUnit,
    zone: Option<&Timezone>,
) -> Result<()> {
    writer.write_all(b"{ version = 1, type = ")?;
    write_quoted(writer, kind)?;
    writer.write_all(b", value = [")?;
    write_quoted(writer, unit.as_str())?;
    write!(writer, ", {count}")?;
    if let Some(zone) = zone {
        writer.write_all(b", ")?;
        write_quoted(writer, zone.as_str())?;
    }
    writer.write_all(b"] }")?;
    Ok(())
}

fn is_plain_mapping(entries: &[(Value, Value)]) -> bool {
    entries
        .iter()
        .all(|(key, _)| matches!(key, Value::String(_)))
        && !(entries.len() == 1 && entries[0].0.as_str() == Some(MARKER))
}

/// Write the shortest TOML spelling that reads back as the same float.
fn write_float<W: Write>(writer: &mut W, value: f64) -> Result<()> {
    if value.is_nan() {
        writer.write_all(b"nan")?;
    } else if value == f64::INFINITY {
        writer.write_all(b"inf")?;
    } else if value == f64::NEG_INFINITY {
        writer.write_all(b"-inf")?;
    } else {
        // Rust's `Display` for `f64` never reaches for exponent notation, so it
        // spells 1e300 as three hundred and two digits and 5e-324 as roughly
        // seven hundred and fifty, which is unreadable and needlessly large for
        // a grammar that has accepted exponent form since TOML 1.0. Formatting
        // through `serde_json::Number` borrows the shortest round-tripping
        // spelling that the JSON codec already emits, and serde_json is an
        // existing dependency of this crate, so the alternative would be a
        // second float printer to keep in step with the first.
        let spelling = serde_json::Number::from_f64(value)
            .ok_or_else(|| codec_error("float has no finite TOML spelling"))?
            .to_string();
        writer.write_all(spelling.as_bytes())?;
        // A spelling of an integral value that carries neither a point nor an
        // exponent would read back as a TOML integer rather than a float, so it
        // needs a fractional part to keep its type across a round trip.
        if !spelling.contains(['.', 'e', 'E']) {
            writer.write_all(b".0")?;
        }
    }
    Ok(())
}

fn write_quoted<W: Write>(writer: &mut W, value: &str) -> Result<()> {
    writer.write_all(b"\"")?;
    let mut unescaped = 0;
    for (position, character) in value.char_indices() {
        let escape = match character {
            '"' => Some("\\\""),
            '\\' => Some("\\\\"),
            '\u{0008}' => Some("\\b"),
            '\t' => Some("\\t"),
            '\n' => Some("\\n"),
            '\u{000c}' => Some("\\f"),
            '\r' => Some("\\r"),
            _ => None,
        };
        if let Some(escape) = escape {
            writer.write_all(&value.as_bytes()[unescaped..position])?;
            writer.write_all(escape.as_bytes())?;
            unescaped = position.saturating_add(character.len_utf8());
        } else if character <= '\u{001f}' || character == '\u{007f}' {
            writer.write_all(&value.as_bytes()[unescaped..position])?;
            write!(writer, "\\u{:04X}", u32::from(character))?;
            unescaped = position.saturating_add(character.len_utf8());
        }
    }
    writer.write_all(&value.as_bytes()[unescaped..])?;
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
