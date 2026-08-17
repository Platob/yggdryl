//! Borrowed JSON serialization over the shared structured-text value.

use base64::Engine as _;
use serde::ser::{SerializeMap, SerializeSeq, SerializeTuple};
use serde::{Serialize, Serializer};

use crate::text::wire::JSON_MARKER;
use crate::{TimeUnit, Timezone, Value};

const WIRE_VERSION: u64 = 1;

pub(super) struct JsonRef<'a>(pub(super) &'a Value);

impl Serialize for JsonRef<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            // A record is its named mapping on any schemaless wire.
            Value::Record(..) => {
                let lowered = self.0.record_to_mapping();
                JsonRef(&lowered).serialize(serializer)
            }
            Value::Null => serializer.serialize_none(),
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::I64(value) => serializer.serialize_i64(*value),
            Value::U64(value) => serializer.serialize_u64(*value),
            Value::I128(value) => JsonEnvelopeRef::I128(*value).serialize(serializer),
            Value::U128(value) => JsonEnvelopeRef::U128(*value).serialize(serializer),
            Value::Float(value) if value.as_f64().is_finite() => {
                serializer.serialize_f64(value.as_f64())
            }
            Value::Float(value) => JsonEnvelopeRef::Float(value.as_f64()).serialize(serializer),
            Value::Decimal(unscaled, scale) => {
                JsonEnvelopeRef::Decimal(*unscaled, *scale).serialize(serializer)
            }
            Value::String(value) => serializer.serialize_str(value),
            Value::Bytes(value) => JsonEnvelopeRef::Bytes(value).serialize(serializer),
            Value::Date(days) => JsonEnvelopeRef::Date(*days).serialize(serializer),
            Value::Time(count, unit) => JsonEnvelopeRef::Time(*count, *unit).serialize(serializer),
            Value::Timestamp(count, unit, zone) => {
                JsonEnvelopeRef::Timestamp(*count, *unit, zone.as_ref()).serialize(serializer)
            }
            Value::Duration(count, unit) => {
                JsonEnvelopeRef::Duration(*count, *unit).serialize(serializer)
            }
            Value::Sequence(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values.iter() {
                    sequence.serialize_element(&JsonRef(value))?;
                }
                sequence.end()
            }
            Value::Mapping(entries) if is_plain_json_mapping(entries) => {
                let mut mapping = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries.iter() {
                    let Value::String(key) = key else {
                        unreachable!("plain mappings contain string keys");
                    };
                    mapping.serialize_entry(key.as_str(), &JsonRef(value))?;
                }
                mapping.end()
            }
            Value::Mapping(entries) => JsonEnvelopeRef::Mapping(entries).serialize(serializer),
        }
    }
}

fn is_plain_json_mapping(entries: &[(Value, Value)]) -> bool {
    entries
        .iter()
        .all(|(key, _)| matches!(key, Value::String(_)))
        && !(entries.len() == 1 && entries[0].0.as_str() == Some(JSON_MARKER))
}

enum JsonEnvelopeRef<'a> {
    Bytes(&'a [u8]),
    I128(i128),
    U128(u128),
    Float(f64),
    Decimal(i128, i8),
    Date(i32),
    Time(i64, TimeUnit),
    Timestamp(i64, TimeUnit, Option<&'a Timezone>),
    Duration(i64, TimeUnit),
    Mapping(&'a [(Value, Value)]),
}

impl Serialize for JsonEnvelopeRef<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut outer = serializer.serialize_map(Some(1))?;
        outer.serialize_entry(JSON_MARKER, &JsonEnvelopeBody(self))?;
        outer.end()
    }
}

struct JsonEnvelopeBody<'a>(&'a JsonEnvelopeRef<'a>);

impl Serialize for JsonEnvelopeBody<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Every envelope body is exactly the version, the kind, and the payload.
        let mut mapping = serializer.serialize_map(Some(3))?;
        mapping.serialize_entry("version", &WIRE_VERSION)?;
        let kind = match self.0 {
            JsonEnvelopeRef::Bytes(_) => "bytes",
            JsonEnvelopeRef::I128(_) => "i128",
            JsonEnvelopeRef::U128(_) => "u128",
            JsonEnvelopeRef::Float(_) => "float",
            JsonEnvelopeRef::Decimal(..) => "decimal",
            JsonEnvelopeRef::Date(_) => "date",
            JsonEnvelopeRef::Time(..) => "time",
            JsonEnvelopeRef::Timestamp(..) => "timestamp",
            JsonEnvelopeRef::Duration(..) => "duration",
            JsonEnvelopeRef::Mapping(_) => "mapping",
        };
        mapping.serialize_entry("type", kind)?;
        match self.0 {
            JsonEnvelopeRef::Bytes(value) => mapping.serialize_entry(
                "value",
                &base64::engine::general_purpose::STANDARD.encode(value),
            )?,
            JsonEnvelopeRef::I128(value) => {
                mapping.serialize_entry("value", &value.to_string())?;
            }
            JsonEnvelopeRef::U128(value) => {
                mapping.serialize_entry("value", &value.to_string())?;
            }
            JsonEnvelopeRef::Float(value) => {
                let value = if value.is_nan() {
                    "nan"
                } else if value.is_sign_positive() {
                    "infinity"
                } else {
                    "-infinity"
                };
                mapping.serialize_entry("value", value)?;
            }
            // A coefficient travels as text for the same reason `i128` does:
            // it is wider than the number every JSON reader agrees on.
            JsonEnvelopeRef::Decimal(unscaled, scale) => {
                mapping.serialize_entry("value", &(unscaled.to_string(), scale))?;
            }
            JsonEnvelopeRef::Date(days) => mapping.serialize_entry("value", days)?,
            JsonEnvelopeRef::Time(count, unit) | JsonEnvelopeRef::Duration(count, unit) => {
                mapping.serialize_entry("value", &(unit.as_str(), count))?;
            }
            JsonEnvelopeRef::Timestamp(count, unit, Some(zone)) => {
                mapping.serialize_entry("value", &(unit.as_str(), count, zone.as_str()))?;
            }
            JsonEnvelopeRef::Timestamp(count, unit, None) => {
                mapping.serialize_entry("value", &(unit.as_str(), count))?;
            }
            JsonEnvelopeRef::Mapping(entries) => {
                mapping.serialize_entry("value", &EntriesRef(entries))?;
            }
        }
        mapping.end()
    }
}

struct EntriesRef<'a>(&'a [(Value, Value)]);

impl Serialize for EntriesRef<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            sequence.serialize_element(&PairRef(entry))?;
        }
        sequence.end()
    }
}

struct PairRef<'a>(&'a (Value, Value));

impl Serialize for PairRef<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut pair = serializer.serialize_tuple(2)?;
        pair.serialize_element(&JsonRef(&self.0.0))?;
        pair.serialize_element(&JsonRef(&self.0.1))?;
        pair.end()
    }
}
