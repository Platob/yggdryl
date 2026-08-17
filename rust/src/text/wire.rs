use base64::Engine as _;

use super::{Limits, Value};
use crate::{Error, Result, TimeUnit};

pub(crate) const JSON_MARKER: &str = "$yggdryl";
const YAML_MAPPING: &str = "yggdryl/map";
const YAML_BYTES: &str = "yggdryl/bytes";
const YAML_I128: &str = "yggdryl/i128";
const YAML_U128: &str = "yggdryl/u128";
const YAML_FLOAT: &str = "yggdryl/float";

#[derive(Clone, Debug)]
pub(crate) enum RawValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Sequence(Vec<Self>),
    Mapping(Vec<(Self, Self)>),
    YamlMergeKey,
    YamlMapping(Vec<(Self, Self)>, Vec<usize>),
    YamlTagged(String, Box<Self>, usize),
}

pub(super) struct DecodeState {
    limits: Limits,
    nodes: usize,
    format: &'static str,
}

impl DecodeState {
    pub(super) const fn new(limits: Limits, format: &'static str) -> Self {
        Self {
            limits,
            nodes: 0,
            format,
        }
    }

    fn visit(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth() {
            return Err(Error::Codec {
                format: self.format,
                position: 0,
                reason: "nesting depth limit exceeded".into(),
            });
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes() {
            return Err(Error::Codec {
                format: self.format,
                position: 0,
                reason: "decoded node limit exceeded".into(),
            });
        }
        Ok(())
    }
}

pub(crate) fn from_raw(raw: RawValue, limits: Limits, format: &'static str) -> Result<Value> {
    decode(raw, 0, &mut DecodeState::new(limits, format))
}

fn decode(raw: RawValue, depth: usize, state: &mut DecodeState) -> Result<Value> {
    state.visit(depth)?;
    match raw {
        RawValue::Null => Ok(Value::Null),
        RawValue::Bool(value) => Ok(Value::Bool(value)),
        RawValue::I64(value) => Ok(Value::I64(value)),
        RawValue::U64(value) => Ok(Value::U64(value)),
        RawValue::I128(value) => Ok(Value::from(value)),
        RawValue::U128(value) => Ok(Value::from(value)),
        RawValue::Float(value) => Ok(Value::from(value)),
        RawValue::String(value) => Ok(Value::from(value)),
        RawValue::Bytes(value) => Ok(Value::from(value)),
        RawValue::Sequence(values) => values
            .into_iter()
            .map(|value| decode(value, depth + 1, state))
            .collect::<Result<Vec<_>>>()
            .map(Value::from),
        RawValue::Mapping(entries) => {
            if state.format == "json" {
                if let Some(decoded) = decode_json_envelope(&entries, depth, state) {
                    return decoded;
                }
            }
            // A YAML flow mapping arrives here rather than as `YamlMapping`,
            // and carries the same flat envelope the block form does.
            if state.format == "yaml" {
                if let Some(decoded) = decode_yaml_envelope(&entries, depth, state) {
                    return decoded;
                }
            }
            decode_mapping(entries, depth, state)
        }
        RawValue::YamlMergeKey => Ok(Value::from("<<")),
        RawValue::YamlMapping(entries, key_positions) => {
            // YAML envelopes are ordinary mappings rather than custom `!yggdryl/*`
            // tags, so the marker is recognized here exactly as it is in JSON.
            if let Some(decoded) = decode_yaml_envelope(&entries, depth, state) {
                return decoded;
            }
            decode_yaml_mapping(entries, key_positions, depth, state)
        }
        RawValue::YamlTagged(tag, value, position) => decode_yaml_tag(tag, *value, depth, state)
            .map_err(|error| yaml_error_at(error, position)),
    }
}

fn decode_mapping(
    entries: Vec<(RawValue, RawValue)>,
    depth: usize,
    state: &mut DecodeState,
) -> Result<Value> {
    let entries = entries
        .into_iter()
        .map(|(key, value)| {
            Ok((
                decode(key, depth + 1, state)?,
                decode(value, depth + 1, state)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Value::from_mapping(entries)
}

fn decode_yaml_mapping(
    entries: Vec<(RawValue, RawValue)>,
    key_positions: Vec<usize>,
    depth: usize,
    state: &mut DecodeState,
) -> Result<Value> {
    let entries = entries
        .into_iter()
        .map(|(key, value)| {
            Ok((
                decode(key, depth + 1, state)?,
                decode(value, depth + 1, state)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Value::from_mapping(entries).map_err(|error| match error {
        Error::Codec {
            format: "value",
            position,
            reason,
        } => Error::Codec {
            format: "yaml",
            position: key_positions.get(position).copied().unwrap_or_default(),
            reason,
        },
        error => error,
    })
}

fn decode_json_envelope(
    outer: &[(RawValue, RawValue)],
    depth: usize,
    state: &mut DecodeState,
) -> Option<Result<Value>> {
    if outer.len() != 1 || raw_string(&outer[0].0) != Some(JSON_MARKER) {
        return None;
    }
    let RawValue::Mapping(fields) = &outer[0].1 else {
        return None;
    };
    let version = field(fields, "version");
    let kind = field(fields, "type").and_then(raw_string);
    if !matches!(version, Some(RawValue::I64(1) | RawValue::U64(1))) || kind.is_none() {
        return None;
    }
    let kind = kind.unwrap_or_default();
    if !is_envelope_kind(kind) || !has_exact_fields(fields, &["version", "type", "value"]) {
        return None;
    }
    let value = field(fields, "value")?;
    Some(decode_envelope_payload(kind, value, depth, state))
}

/// Return whether a name is one of the envelope kinds this wire format spells.
///
/// Every kind whose native shape a text format cannot express appears here, so
/// adding a variant to the value means adding its name here and nowhere else.
/// Each one names a `Value` variant; a name that does not is not an envelope,
/// which is why the removed `tag` kind now reads back as the ordinary mapping
/// it is spelled as.
fn is_envelope_kind(kind: &str) -> bool {
    matches!(
        kind,
        "bytes"
            | "i128"
            | "u128"
            | "float"
            | "decimal"
            | "date"
            | "time"
            | "timestamp"
            | "duration"
            | "mapping"
    )
}

fn clone_raw(value: &RawValue) -> Option<RawValue> {
    // Envelopes are borrowed during recognition; recursively cloning only the
    // selected payload keeps the common non-envelope path move-only.
    Some(match value {
        RawValue::Null => RawValue::Null,
        RawValue::Bool(value) => RawValue::Bool(*value),
        RawValue::I64(value) => RawValue::I64(*value),
        RawValue::U64(value) => RawValue::U64(*value),
        RawValue::I128(value) => RawValue::I128(*value),
        RawValue::U128(value) => RawValue::U128(*value),
        RawValue::Float(value) => RawValue::Float(*value),
        RawValue::String(value) => RawValue::String(value.clone()),
        RawValue::Bytes(value) => RawValue::Bytes(value.clone()),
        RawValue::Sequence(values) => {
            RawValue::Sequence(values.iter().map(clone_raw).collect::<Option<_>>()?)
        }
        RawValue::Mapping(entries) => RawValue::Mapping(
            entries
                .iter()
                .map(|(key, value)| Some((clone_raw(key)?, clone_raw(value)?)))
                .collect::<Option<_>>()?,
        ),
        RawValue::YamlMergeKey => RawValue::YamlMergeKey,
        RawValue::YamlMapping(entries, positions) => RawValue::YamlMapping(
            entries
                .iter()
                .map(|(key, value)| Some((clone_raw(key)?, clone_raw(value)?)))
                .collect::<Option<_>>()?,
            positions.clone(),
        ),
        RawValue::YamlTagged(tag, value, position) => {
            RawValue::YamlTagged(tag.clone(), Box::new(clone_raw(value)?), *position)
        }
    })
}

fn parse_i128_for(value: &RawValue, format: &'static str) -> Result<Value> {
    let parsed = match value {
        RawValue::I64(value) => Some(i128::from(*value)),
        RawValue::U64(value) => Some(i128::from(*value)),
        RawValue::I128(value) => Some(*value),
        RawValue::U128(value) => i128::try_from(*value).ok(),
        _ => raw_string(value).and_then(|value| value.parse::<i128>().ok()),
    };
    parsed
        .map(Value::I128)
        .ok_or_else(|| codec_error(format, "invalid i128 envelope"))
}

fn parse_u128_for(value: &RawValue, format: &'static str) -> Result<Value> {
    let parsed = match value {
        RawValue::I64(value) => u128::try_from(*value).ok(),
        RawValue::U64(value) => Some(u128::from(*value)),
        RawValue::I128(value) => u128::try_from(*value).ok(),
        RawValue::U128(value) => Some(*value),
        _ => raw_string(value).and_then(|value| value.parse::<u128>().ok()),
    };
    parsed
        .map(Value::U128)
        .ok_or_else(|| codec_error(format, "invalid u128 envelope"))
}

/// Read the `[unscaled, scale]` payload a decimal envelope carries.
///
/// The coefficient travels as text because a 128-bit integer is wider than the
/// number every JSON and YAML reader agrees on, exactly as `i128` already does.
fn parse_decimal(value: &RawValue, format: &'static str) -> Result<Value> {
    let parts = raw_sequence(value)
        .filter(|parts| parts.len() == 2)
        .ok_or_else(|| codec_error(format, "decimal envelope value must be [unscaled, scale]"))?;
    let unscaled = raw_string(&parts[0])
        .and_then(|unscaled| unscaled.parse::<i128>().ok())
        .ok_or_else(|| codec_error(format, "decimal envelope has no unscaled integer"))?;
    let scale = raw_i64(&parts[1])
        .and_then(|scale| i8::try_from(scale).ok())
        .ok_or_else(|| codec_error(format, "decimal envelope has no scale"))?;
    Ok(Value::decimal(unscaled, scale))
}

/// Read the day count a date envelope carries.
fn parse_date(value: &RawValue, format: &'static str) -> Result<Value> {
    raw_i64(value)
        .and_then(|days| i32::try_from(days).ok())
        .map(Value::date)
        .ok_or_else(|| codec_error(format, "date envelope value must be a day count"))
}

/// Read the `[unit, count]` payload a time, timestamp, or duration carries.
fn parse_temporal(kind: &str, value: &RawValue, format: &'static str) -> Result<Value> {
    let parts = raw_sequence(value)
        .ok_or_else(|| codec_error(format, "temporal envelope value must be a sequence"))?;
    let unit = parts
        .first()
        .and_then(raw_string)
        .and_then(|unit| TimeUnit::from_str(unit).ok())
        .ok_or_else(|| codec_error(format, "temporal envelope has no unit"))?;
    let count = parts
        .get(1)
        .and_then(raw_i64)
        .ok_or_else(|| codec_error(format, "temporal envelope has no count"))?;
    match kind {
        "time" if parts.len() == 2 => Ok(Value::time(count, unit)),
        "duration" if parts.len() == 2 => Ok(Value::duration(count, unit)),
        "timestamp" if parts.len() == 2 => Ok(Value::timestamp_in(count, unit, None)),
        "timestamp" if parts.len() == 3 => {
            let zone = raw_string(&parts[2])
                .ok_or_else(|| codec_error(format, "timestamp envelope zone is not text"))?;
            Value::timestamp(count, unit, Some(zone))
        }
        _ => Err(codec_error(format, "temporal envelope has the wrong shape")),
    }
}

fn raw_sequence(value: &RawValue) -> Option<&[RawValue]> {
    match value {
        RawValue::Sequence(values) => Some(values),
        _ => None,
    }
}

fn raw_i64(value: &RawValue) -> Option<i64> {
    match value {
        RawValue::I64(value) => Some(*value),
        RawValue::U64(value) => i64::try_from(*value).ok(),
        RawValue::I128(value) => i64::try_from(*value).ok(),
        RawValue::U128(value) => i64::try_from(*value).ok(),
        _ => None,
    }
}

fn parse_special_float(value: &RawValue, format: &'static str) -> Result<Value> {
    match raw_string(value) {
        Some("nan") => Ok(Value::from(f64::NAN)),
        Some("infinity") => Ok(Value::from(f64::INFINITY)),
        Some("-infinity") => Ok(Value::from(f64::NEG_INFINITY)),
        _ => Err(codec_error(format, "invalid non-finite float envelope")),
    }
}

fn decode_mapping_entries(
    value: &RawValue,
    depth: usize,
    state: &mut DecodeState,
) -> Result<Value> {
    let RawValue::Sequence(entries) = value else {
        return Err(codec_error(
            state.format,
            "mapping envelope value must be an entry sequence",
        ));
    };
    let mut decoded = Vec::with_capacity(entries.len());
    for entry in entries {
        let RawValue::Sequence(pair) = entry else {
            return Err(codec_error(
                state.format,
                "mapping envelope entry must be a pair",
            ));
        };
        if pair.len() != 2 {
            return Err(codec_error(
                state.format,
                "mapping envelope entry must contain two values",
            ));
        }
        decoded.push((
            decode(
                clone_raw(&pair[0]).ok_or_else(|| codec_error(state.format, "invalid key"))?,
                depth + 1,
                state,
            )?,
            decode(
                clone_raw(&pair[1]).ok_or_else(|| codec_error(state.format, "invalid value"))?,
                depth + 1,
                state,
            )?,
        ));
    }
    Value::from_mapping(decoded)
}

fn decode_yaml_envelope(
    fields: &[(RawValue, RawValue)],
    depth: usize,
    state: &mut DecodeState,
) -> Option<Result<Value>> {
    let kind = field(fields, JSON_MARKER).and_then(raw_string)?;
    if !is_envelope_kind(kind) || !has_exact_fields(fields, &[JSON_MARKER, "value"]) {
        return None;
    }
    let value = field(fields, "value")?;
    Some(decode_envelope_payload(kind, value, depth, state))
}

fn decode_envelope_payload(
    kind: &str,
    value: &RawValue,
    depth: usize,
    state: &mut DecodeState,
) -> Result<Value> {
    match kind {
        "bytes" => raw_string(value).map_or_else(
            || {
                Err(codec_error(
                    state.format,
                    "bytes envelope value must be base64 text",
                ))
            },
            |value| {
                base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .map(Value::from)
                    .map_err(|_| codec_error(state.format, "invalid base64 bytes envelope"))
            },
        ),
        "i128" => parse_i128_for(value, state.format),
        "u128" => parse_u128_for(value, state.format),
        "float" => parse_special_float(value, state.format),
        "decimal" => parse_decimal(value, state.format),
        "date" => parse_date(value, state.format),
        "time" | "timestamp" | "duration" => parse_temporal(kind, value, state.format),
        "mapping" => decode_mapping_entries(value, depth, state),
        _ => unreachable!("recognized Yggdryl envelope kind"),
    }
}

/// Decode one YAML node that carries a non-core tag.
///
/// A `!yggdryl/*` machine tag names a kind this value model has, so it selects
/// the payload it names. Every other tag is a name this value model cannot
/// hold: there is no carrier for one any more, and every runtime that consumed
/// a tagged value already threw the name away and kept the payload. So the tag
/// is read as the annotation YAML defines it to be and the node decodes as the
/// plain value it annotates, rather than failing a document this codec can
/// otherwise read in full. Nothing on the write path emits a non-core tag, so
/// no round trip through this crate can reach here.
fn decode_yaml_tag(
    tag: String,
    value: RawValue,
    depth: usize,
    state: &mut DecodeState,
) -> Result<Value> {
    let tag = tag.trim_start_matches('!');
    if matches!(
        tag,
        YAML_MAPPING | YAML_BYTES | YAML_I128 | YAML_U128 | YAML_FLOAT
    ) {
        if let Some(fields) = raw_mapping(&value) {
            if let Some(decoded) = decode_yaml_envelope(fields, depth, state) {
                return decoded;
            }
        }
        return match tag {
            YAML_MAPPING => decode_mapping_entries(&value, depth, state),
            YAML_BYTES => {
                let RawValue::String(encoded) = value else {
                    return Err(codec_error(
                        "yaml",
                        "yggdryl bytes payload must be base64 text",
                    ));
                };
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map(Value::from)
                    .map_err(|_| codec_error("yaml", "invalid base64 bytes payload"))
            }
            YAML_I128 => parse_i128_for(&value, "yaml"),
            YAML_U128 => parse_u128_for(&value, "yaml"),
            YAML_FLOAT => parse_special_float(&value, "yaml"),
            _ => unreachable!("known Yggdryl YAML tag"),
        };
    }
    decode(value, depth + 1, state)
}

fn raw_mapping(value: &RawValue) -> Option<&[(RawValue, RawValue)]> {
    match value {
        RawValue::Mapping(entries) | RawValue::YamlMapping(entries, _) => Some(entries),
        _ => None,
    }
}

fn yaml_error_at(error: Error, position: usize) -> Error {
    match error {
        Error::Codec { reason, .. } => Error::Codec {
            format: "yaml",
            position,
            reason,
        },
        error => error,
    }
}

fn raw_string(value: &RawValue) -> Option<&str> {
    match value {
        RawValue::String(value) => Some(value),
        _ => None,
    }
}

fn field<'a>(entries: &'a [(RawValue, RawValue)], name: &str) -> Option<&'a RawValue> {
    entries
        .iter()
        .find_map(|(key, value)| (raw_string(key) == Some(name)).then_some(value))
}

fn has_duplicate_string_keys(entries: &[(RawValue, RawValue)]) -> bool {
    entries.iter().enumerate().any(|(index, (key, _))| {
        let Some(key) = raw_string(key) else {
            return false;
        };
        entries[..index]
            .iter()
            .any(|(previous, _)| raw_string(previous) == Some(key))
    })
}

fn has_exact_fields(entries: &[(RawValue, RawValue)], names: &[&str]) -> bool {
    entries.len() == names.len()
        && !has_duplicate_string_keys(entries)
        && entries
            .iter()
            .all(|(key, _)| raw_string(key).is_some_and(|key| names.contains(&key)))
}

pub(super) fn codec_error(format: &'static str, reason: &'static str) -> Error {
    Error::Codec {
        format,
        position: 0,
        reason: reason.into(),
    }
}
