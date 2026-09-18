use base64::Engine as _;
use serde::ser::{Error as _, SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use crate::{Scalar, TimeUnit, Timezone};
use crate::types::code_scalars;

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
            #[cfg(feature = "arrow")]
            Scalar::Arrow(_) => {
                let native = self.0.into_native().map_err(S::Error::custom)?;
                JsonRef(&native).serialize(serializer)
            }
            Scalar::Null => serializer.serialize_none(),
            Scalar::Boolean(value) => serializer.serialize_bool(value.get()),
            Scalar::Int8(value) => serializer.serialize_i8(value.get()),
            Scalar::Int16(value) => serializer.serialize_i16(value.get()),
            Scalar::Int32(value) => serializer.serialize_i32(value.get()),
            Scalar::Int64(value) => serializer.serialize_i64(value.get()),
            Scalar::UInt8(value) => serializer.serialize_u8(value.get()),
            Scalar::UInt16(value) => serializer.serialize_u16(value.get()),
            Scalar::UInt32(value) => serializer.serialize_u32(value.get()),
            Scalar::UInt64(value) => serializer.serialize_u64(value.get()),
            Scalar::Int128(value) => serializer.serialize_i128(value.get()),
            Scalar::UInt128(value) => serializer.serialize_u128(value.get()),
            Scalar::Float16(value) => serialize_float(serializer, value.as_f64()),
            Scalar::Float32(value) => serialize_float(serializer, value.as_f64()),
            Scalar::Float64(value) => serialize_float(serializer, value.as_f64()),
            // A decimal leaf displays exactly its canonical decimal text.
            Scalar::Decimal32(value) => serializer.collect_str(value),
            Scalar::Decimal64(value) => serializer.collect_str(value),
            Scalar::Decimal128(value) => serializer.collect_str(value),
            Scalar::Decimal256(value) => serializer.collect_str(value),
            Scalar::String(value) => serializer.serialize_str(value.as_str()),
            code_scalars!() => serializer
                .serialize_str(self.0.as_str().expect("a code borrowed its text")),
            Scalar::Version(value) => serializer.collect_str(value),
            Scalar::Url(value) => serializer.collect_str(value),
            Scalar::Timezone(value) => serializer.serialize_str(value.as_str()),
            Scalar::MimeType(value) => serializer.serialize_str(value.as_str()),
            Scalar::MediaType(value) => serializer.collect_str(value),
            Scalar::Uuid(value) => {
                let mut slot = [0_u8; crate::types::Uuid::TEXT_LEN];
                serializer.serialize_str(value.render(&mut slot))
            }
            Scalar::Bytes(value) => serializer
                .serialize_str(&base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
            Scalar::Geometry(value) => serializer
                .serialize_str(&base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
            Scalar::Geography(value) => serializer
                .serialize_str(&base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
            Scalar::Date32(value) => {
                if value.unit() == TimeUnit::Day {
                    if let Some(text) = crate::types::temporal::iso::format_date(value.count()) {
                        return serializer.serialize_str(&text);
                    }
                }
                serializer.serialize_i32(value.count())
            }
            Scalar::Date64(value) => {
                const DAY_MILLISECONDS: i64 = 86_400_000;
                if value.unit() == TimeUnit::Millisecond {
                    let days = value.count().div_euclid(DAY_MILLISECONDS);
                    if value.count().rem_euclid(DAY_MILLISECONDS) == 0 {
                        if let Ok(days) = i32::try_from(days) {
                            if let Some(text) = crate::types::temporal::iso::format_date(days) {
                                return serializer.serialize_str(&text);
                            }
                        }
                    }
                }
                serializer.serialize_i64(value.count())
            }
            Scalar::Time32(value) => serialize_time(
                serializer,
                i64::from(value.count()),
                value.unit(),
                &value.timezone(),
            ),
            Scalar::Time64(value) => {
                serialize_time(serializer, value.count(), value.unit(), &value.timezone())
            }
            Scalar::DateTime64(value) => {
                let text = if value.timezone().is_naive() {
                    crate::types::temporal::iso::format_datetime(value.count(), value.unit())
                } else {
                    crate::types::temporal::iso::format_timestamp(
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
            Scalar::Duration32(value) => serialize_duration(
                serializer,
                i64::from(value.count()),
                value.unit(),
                &value.timezone(),
            ),
            Scalar::Duration64(value) => {
                serialize_duration(serializer, value.count(), value.unit(), &value.timezone())
            }
            Scalar::Interval(value) => match value.unit() {
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
            Scalar::Sequence(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.as_slice().len()))?;
                for value in values.as_slice() {
                    sequence.serialize_element(&JsonRef(value))?;
                }
                sequence.end()
            }
            Scalar::Record(entries) => {
                let mut mapping = serializer.serialize_map(Some(entries.as_map().len()))?;
                for (name, value) in entries.as_map() {
                    mapping.serialize_entry(name, &JsonRef(value))?;
                }
                mapping.end()
            }
            Scalar::Mapping(entries) => {
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
    let Some(text) = crate::types::temporal::iso::format_time(count, unit) else {
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
        if let Some(text) = crate::types::temporal::iso::format_duration(count, unit) {
            return serializer.serialize_str(&text);
        }
    } else {
        return Err(S::Error::custom("duration cannot carry a timezone"));
    }
    serializer.serialize_i64(count)
}
