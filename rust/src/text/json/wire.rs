use base64::Engine as _;
use serde::ser::{Error as _, SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use crate::{I256, Scalar, TimeUnit, Timezone};

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
            Scalar::Bool(value) => serializer.serialize_bool(*value),
            Scalar::I8(value) => serializer.serialize_i8(*value),
            Scalar::I16(value) => serializer.serialize_i16(*value),
            Scalar::I32(value) => serializer.serialize_i32(*value),
            Scalar::I64(value) => serializer.serialize_i64(*value),
            Scalar::U8(value) => serializer.serialize_u8(*value),
            Scalar::U16(value) => serializer.serialize_u16(*value),
            Scalar::U32(value) => serializer.serialize_u32(*value),
            Scalar::U64(value) => serializer.serialize_u64(*value),
            Scalar::I128(value) => serializer.serialize_i128(*value),
            Scalar::U128(value) => serializer.serialize_u128(*value),
            Scalar::F16(value) => serialize_float(serializer, value.as_f64()),
            Scalar::F32(value) => serialize_float(serializer, value.as_f64()),
            Scalar::F64(value) => serialize_float(serializer, value.as_f64()),
            Scalar::D128(unscaled, scale) => serializer.serialize_str(
                &crate::types::decimal::scalars::decimal_text(I256::from_i128(*unscaled), *scale),
            ),
            Scalar::D256(unscaled, scale) => serializer.serialize_str(
                &crate::types::decimal::scalars::decimal_text(*unscaled, *scale),
            ),
            Scalar::String(value) => serializer.serialize_str(value),
            Scalar::Enum(value) => serializer.serialize_str(value.as_str()),
            Scalar::Bytes(value) | Scalar::Geospatial(value) => {
                serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(value))
            }
            Scalar::Date32(count, unit, zone) => {
                if !zone.is_naive() {
                    return Err(S::Error::custom("Date32 cannot carry a timezone"));
                }
                if *unit == TimeUnit::Day && zone.is_naive() {
                    if let Some(text) = crate::types::ascii::iso::format_date(*count) {
                        return serializer.serialize_str(&text);
                    }
                }
                serializer.serialize_i32(*count)
            }
            Scalar::Date64(count, unit, zone) => {
                if !zone.is_naive() {
                    return Err(S::Error::custom("Date64 cannot carry a timezone"));
                }
                const DAY_MILLISECONDS: i64 = 86_400_000;
                if *unit == TimeUnit::Millisecond && zone.is_naive() {
                    let days = count.div_euclid(DAY_MILLISECONDS);
                    if count.rem_euclid(DAY_MILLISECONDS) == 0 {
                        if let Ok(days) = i32::try_from(days) {
                            if let Some(text) = crate::types::ascii::iso::format_date(days) {
                                return serializer.serialize_str(&text);
                            }
                        }
                    }
                }
                serializer.serialize_i64(*count)
            }
            Scalar::Time32(count, unit, zone) => {
                serialize_time(serializer, i64::from(*count), *unit, zone)
            }
            Scalar::Time64(count, unit, zone) => serialize_time(serializer, *count, *unit, zone),
            Scalar::DateTime64(count, unit, zone) => {
                let text = if zone.is_naive() {
                    crate::types::ascii::iso::format_datetime(*count, *unit)
                } else {
                    crate::types::ascii::iso::format_timestamp(*count, *unit, zone)
                };
                match text {
                    Some(text) => serializer.serialize_str(&text),
                    None => serializer.serialize_i64(*count),
                }
            }
            Scalar::Duration32(count, unit, zone) => {
                serialize_duration(serializer, i64::from(*count), *unit, zone)
            }
            Scalar::Duration64(count, unit, zone) => {
                serialize_duration(serializer, *count, *unit, zone)
            }
            Scalar::Sequence(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values.iter() {
                    sequence.serialize_element(&JsonRef(value))?;
                }
                sequence.end()
            }
            Scalar::Record(entries) => {
                let mut mapping = serializer.serialize_map(Some(entries.len()))?;
                for (name, value) in entries.iter() {
                    mapping.serialize_entry(name, &JsonRef(value))?;
                }
                mapping.end()
            }
            Scalar::Mapping(entries) => {
                let mut mapping = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries.iter() {
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
