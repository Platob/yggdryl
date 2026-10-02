use base64::Engine as _;
use serde::ser::{Error as _, SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use crate::{DataType, Scalar, TimeUnit, Timezone};
use crate::{bytes_scalars, code_scalars, string_scalars};

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
            // A variant is the value its bytes hold: JSON writes that, which
            // is what every other variant reader shows.
            Scalar::Variant(value) => {
                let held = value.scalar().map_err(S::Error::custom)?;
                JsonRef(&held).serialize(serializer)
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
            Scalar::Decimal(value) => serializer.collect_str(value),
            Scalar::BigDecimal(value) => serializer.collect_str(value),
            string_scalars!(value) => serializer.serialize_str(value.as_str()),
            code_scalars!() => {
                serializer.serialize_str(self.0.as_str().expect("a code borrowed its text"))
            }
            crate::enum_scalars!() => {
                serializer.serialize_str(self.0.enum_name().expect("an enum member names itself"))
            }
            Scalar::Version(value) => serializer.collect_str(value),
            Scalar::Url(value) => serializer.collect_str(value),
            Scalar::Urn(value) => serializer.collect_str(value),
            Scalar::Timezone(value) => serializer.serialize_str(value.as_str()),
            Scalar::MimeType(value) => serializer.serialize_str(value.as_str()),
            Scalar::MediaType(value) => serializer.collect_str(value),
            Scalar::Uuid(value) => {
                let mut slot = [0_u8; crate::Uuid::TEXT_LEN];
                serializer.serialize_str(value.render(&mut slot))
            }
            bytes_scalars!(value) => serializer
                .serialize_str(&base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
            Scalar::Geometry(value) => serializer
                .serialize_str(&base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
            Scalar::Geography(value) => serializer
                .serialize_str(&base64::engine::general_purpose::STANDARD.encode(value.as_bytes())),
            Scalar::Date32(value) => {
                if value.unit() == TimeUnit::Day
                    && let Some(text) = crate::temporal::format_date(value.count())
                {
                    return serializer.serialize_str(&text);
                }
                serializer.serialize_i32(value.count())
            }
            Scalar::Date64(value) => {
                const DAY_MILLISECONDS: i64 = 86_400_000;
                if value.unit() == TimeUnit::Millisecond {
                    let days = value.count().div_euclid(DAY_MILLISECONDS);
                    if value.count().rem_euclid(DAY_MILLISECONDS) == 0
                        && let Ok(days) = i32::try_from(days)
                        && let Some(text) = crate::temporal::format_date(days)
                    {
                        return serializer.serialize_str(&text);
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
                    crate::temporal::format_datetime(value.count(), value.unit())
                } else {
                    crate::temporal::format_timestamp(
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
            Scalar::Serie(values)
            | Scalar::SerieView(values)
            | Scalar::FixedSizeSerie(values)
            | Scalar::LargeSerie(values)
            | Scalar::LargeSerieView(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values.iter() {
                    sequence.serialize_element(&JsonRef(&value))?;
                }
                sequence.end()
            }
            Scalar::Struct(entries) => {
                let mut mapping = serializer.serialize_map(Some(entries.as_map().len()))?;
                for (name, value) in entries.as_map() {
                    mapping.serialize_entry(name, &JsonRef(value))?;
                }
                mapping.end()
            }
            Scalar::Map(entries) | Scalar::SortedMap(entries) => {
                let mut mapping = serializer.serialize_map(Some(entries.as_slice().len()))?;
                for (key, value) in entries.as_slice() {
                    mapping.serialize_entry(&JsonKey(key), &JsonRef(value))?;
                }
                mapping.end()
            }
        }
    }
}

/// A map key, spelled as [`JsonRef`] spells its value where serde_json's key
/// rules take that spelling - text, and a number or a boolean quoted - which
/// the reading half reads back under the key's datatype. An interval would
/// be a quoted count no reader takes for one, so it is refused, as a nested
/// key is by serde_json itself.
struct JsonKey<'a>(&'a Scalar);

impl Serialize for JsonKey<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if matches!(self.0, Scalar::Interval(_)) {
            return Err(S::Error::custom("JSON has no map key for an interval"));
        }
        JsonRef(self.0).serialize(serializer)
    }
}

/// A canonical value written as natural JSON under the datatype that holds
/// it.
///
/// This is the streaming form of
/// [`Field::into_natural_value`](crate::Field::into_natural_value) followed
/// by [`JsonRef`], with no natural value built between them: a struct row -
/// positional once canonical - is an object keyed by its fields' names in
/// declaration order, a serie an array, a map an object whose keys
/// serde_json spells as its own rules allow (text, a number, a boolean), and
/// a union the `[type_id, value]` pair the reading half takes back.
/// Encodings are transparent and every leaf is [`JsonRef`]'s spelling, so
/// what the reading half reads back under the same datatype is the value
/// written.
pub(crate) struct JsonField<'a>(pub(crate) &'a Scalar, pub(crate) &'a DataType);

impl Serialize for JsonField<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Self(value, dtype) = *self;
        if value.is_null() {
            return serializer.serialize_none();
        }
        match dtype {
            DataType::Struct(fields) => match value.as_serie() {
                Some(cells) => {
                    if cells.len() != fields.len() {
                        return Err(S::Error::custom(format_args!(
                            "expected {} struct values, got {}",
                            fields.len(),
                            cells.len()
                        )));
                    }
                    let mut object = serializer.serialize_map(Some(fields.len()))?;
                    for (cell, child) in cells.iter().zip(fields.iter()) {
                        object.serialize_entry(child.name(), &JsonField(&cell, child.dtype()))?;
                    }
                    object.end()
                }
                // A record already carries its names.
                None => JsonRef(value).serialize(serializer),
            },
            DataType::Serie(item)
            | DataType::SerieView(item)
            | DataType::FixedSizeSerie(item, _)
            | DataType::LargeSerie(item)
            | DataType::LargeSerieView(item) => match value.as_serie() {
                Some(items) => {
                    let mut array = serializer.serialize_seq(Some(items.len()))?;
                    for cell in items.iter() {
                        array.serialize_element(&JsonField(&cell, item.dtype()))?;
                    }
                    array.end()
                }
                None => JsonRef(value).serialize(serializer),
            },
            DataType::Map(_) | DataType::SortedMap(_) => {
                let (Some(entries), Some(map)) = (value.as_mapping(), dtype.as_mapping()) else {
                    return JsonRef(value).serialize(serializer);
                };
                let [_, item] = map.entries().fields() else {
                    return Err(S::Error::custom(
                        "map entries do not contain key and value fields",
                    ));
                };
                let mut object = serializer.serialize_map(Some(entries.len()))?;
                for (key, cell) in entries {
                    object.serialize_entry(&JsonKey(key), &JsonField(cell, item.dtype()))?;
                }
                object.end()
            }
            DataType::Union(fields, _) => {
                let pair = value.sequence_rows();
                let Some([type_id, payload]) = pair.as_deref() else {
                    return Err(S::Error::custom("expected [type_id, value] for a union"));
                };
                let branch = type_id
                    .as_i128()
                    .and_then(|id| i8::try_from(id).ok())
                    .and_then(|id| {
                        fields
                            .iter()
                            .find_map(|(candidate, branch)| (candidate == id).then_some(branch))
                    })
                    .ok_or_else(|| S::Error::custom("union type id is not declared"))?;
                let mut array = serializer.serialize_seq(Some(2))?;
                array.serialize_element(&JsonRef(type_id))?;
                array.serialize_element(&JsonField(payload, branch.dtype()))?;
                array.end()
            }
            DataType::Dictionary(dictionary) => {
                JsonField(value, dictionary.value()).serialize(serializer)
            }
            DataType::RunEndEncoded(encoded) => {
                JsonField(value, encoded.values().dtype()).serialize(serializer)
            }
            _ => JsonRef(value).serialize(serializer),
        }
    }
}

pub(super) fn serialize_float<S: Serializer>(serializer: S, value: f64) -> Result<S::Ok, S::Error> {
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
    let Some(text) = crate::temporal::format_time(count, unit) else {
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
        if let Some(text) = crate::temporal::format_duration(count, unit) {
            return serializer.serialize_str(&text);
        }
    } else {
        return Err(S::Error::custom("duration cannot carry a timezone"));
    }
    serializer.serialize_i64(count)
}
