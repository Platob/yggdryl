use base64::Engine as _;
use serde::ser::{Error as _, SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use crate::types::{Integer, Nested, Temporal};
use crate::{Scalar, TimeUnit, Timezone};

/// A natural JSON view of [`Scalar`].
///
/// JSON has no private type envelopes. Types outside its grammar use their
/// interoperable scalar spelling; a [`crate::Field`] restores exact types on
/// schema-directed reads.
pub(super) struct JsonRef<'a>(pub(super) &'a Scalar);

impl Serialize for JsonRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            Scalar::Null => serializer.serialize_none(),
            Scalar::Boolean(value) => serializer.serialize_bool(value.get()),
            Scalar::Integer(value) => match value {
                Integer::I8(value) => serializer.serialize_i8(value.get()),
                Integer::I16(value) => serializer.serialize_i16(value.get()),
                Integer::I32(value) => serializer.serialize_i32(value.get()),
                Integer::I64(value) => serializer.serialize_i64(value.get()),
                Integer::U8(value) => serializer.serialize_u8(value.get()),
                Integer::U16(value) => serializer.serialize_u16(value.get()),
                Integer::U32(value) => serializer.serialize_u32(value.get()),
                Integer::U64(value) => serializer.serialize_u64(value.get()),
                Integer::I128(value) => serializer.serialize_i128(value.get()),
                Integer::U128(value) => serializer.serialize_u128(value.get()),
            },
            Scalar::Floating(value) => serialize_float(serializer, value.as_f64()),
            Scalar::Decimal(value) => serializer.serialize_str(
                &crate::types::decimal::scalars::decimal_text(value.coefficient(), value.scale()),
            ),
            Scalar::Text(value) => serializer.serialize_str(value.as_str()),
            Scalar::Ascii(value) => serializer.serialize_str(value.as_str()),
            Scalar::Guid(value) => serializer.serialize_str(&value.to_string()),
            Scalar::Enum(value) => serializer.serialize_str(value.as_str()),
            Scalar::Bytes(value) => serializer
                .serialize_str(&base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
            Scalar::Geospatial(value) => serializer
                .serialize_str(&base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
            Scalar::Temporal(Temporal::Date32(value)) => {
                if value.unit() == TimeUnit::Day {
                    if let Some(text) = crate::types::ascii::iso::format_date(value.count()) {
                        return serializer.serialize_str(&text);
                    }
                }
                serializer.serialize_i32(value.count())
            }
            Scalar::Temporal(Temporal::Date64(value)) => {
                const DAY_MILLISECONDS: i64 = 86_400_000;
                if value.unit() == TimeUnit::Millisecond {
                    let days = value.count().div_euclid(DAY_MILLISECONDS);
                    if value.count().rem_euclid(DAY_MILLISECONDS) == 0 {
                        if let Ok(days) = i32::try_from(days) {
                            if let Some(text) = crate::types::ascii::iso::format_date(days) {
                                return serializer.serialize_str(&text);
                            }
                        }
                    }
                }
                serializer.serialize_i64(value.count())
            }
            Scalar::Temporal(Temporal::Time32(value)) => serialize_time(
                serializer,
                i64::from(value.count()),
                value.unit(),
                &value.timezone(),
            ),
            Scalar::Temporal(Temporal::Time64(value)) => {
                serialize_time(serializer, value.count(), value.unit(), &value.timezone())
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
                    Some(text) => serializer.serialize_str(&text),
                    None => serializer.serialize_i64(value.count()),
                }
            }
            Scalar::Temporal(Temporal::Duration32(value)) => serialize_duration(
                serializer,
                i64::from(value.count()),
                value.unit(),
                &value.timezone(),
            ),
            Scalar::Temporal(Temporal::Duration64(value)) => {
                serialize_duration(serializer, value.count(), value.unit(), &value.timezone())
            }
            Scalar::Temporal(Temporal::Interval(value)) => match value.unit() {
                TimeUnit::YearMonth => serializer.serialize_i32(value.months()),
                TimeUnit::DayTime => {
                    [i64::from(value.days()), value.nanoseconds() / 1_000_000].serialize(serializer)
                }
                TimeUnit::MonthDayNano => [
                    i64::from(value.months()),
                    i64::from(value.days()),
                    value.nanoseconds(),
                ]
                .serialize(serializer),
                _ => Err(S::Error::custom("invalid interval layout")),
            },
            Scalar::Nested(Nested::Sequence(values)) => {
                let mut sequence = serializer.serialize_seq(Some(values.as_slice().len()))?;
                for value in values.as_slice() {
                    sequence.serialize_element(&JsonRef(value))?;
                }
                sequence.end()
            }
            Scalar::Nested(Nested::Record(entries)) => {
                let mut mapping = serializer.serialize_map(Some(entries.as_map().len()))?;
                for (name, value) in entries.as_map() {
                    mapping.serialize_entry(name, &JsonRef(value))?;
                }
                mapping.end()
            }
            Scalar::Nested(Nested::Mapping(entries)) => {
                let mut mapping = serializer.serialize_map(Some(entries.as_slice().len()))?;
                for (key, value) in entries.as_slice() {
                    let Some(key) = key.as_str() else {
                        return Err(S::Error::custom(
                            "JSON object keys must be strings; use a record or string-key mapping",
                        ));
                    };
                    mapping.serialize_entry(key, &JsonRef(value))?;
                }
                mapping.end()
            }
        }
    }
}

fn serialize_float<S: Serializer>(serializer: S, value: f64) -> Result<S::Ok, S::Error> {
    if value.is_finite() {
        serializer.serialize_f64(value)
    } else {
        Err(S::Error::custom("JSON cannot represent a non-finite float"))
    }
}

fn serialize_time<S: Serializer>(
    serializer: S,
    count: i64,
    unit: TimeUnit,
    zone: &Timezone,
) -> Result<S::Ok, S::Error> {
    if !zone.is_naive() {
        return Err(S::Error::custom(
            "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
        ));
    }
    let Some(text) = crate::types::ascii::iso::format_time(count, unit) else {
        return serializer.serialize_i64(count);
    };
    serializer.serialize_str(&text)
}

fn serialize_duration<S: Serializer>(
    serializer: S,
    count: i64,
    unit: TimeUnit,
    zone: &Timezone,
) -> Result<S::Ok, S::Error> {
    if zone.is_naive() {
        if let Some(text) = crate::types::ascii::iso::format_duration(count, unit) {
            return serializer.serialize_str(&text);
        }
    } else {
        return Err(S::Error::custom("duration cannot carry a timezone"));
    }
    serializer.serialize_i64(count)
}
