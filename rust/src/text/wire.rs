use smol_str::SmolStr;

use super::{Limits, Scalar};
use crate::{Error, Result};

/// Parser-neutral syntax nodes.
///
/// These carry only facts present in the source grammar. Schema-directed
/// interpretation happens after this syntax pass.
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
    YamlTagged(Box<Self>),
}

struct DecodeState {
    limits: Limits,
    nodes: usize,
    format: &'static str,
}

impl DecodeState {
    const fn new(limits: Limits, format: &'static str) -> Self {
        Self {
            limits,
            nodes: 0,
            format,
        }
    }

    fn visit(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth() {
            return Err(codec_error(self.format, 0, "nesting depth limit exceeded"));
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes() {
            return Err(codec_error(self.format, 0, "decoded node limit exceeded"));
        }
        Ok(())
    }
}

pub(crate) fn from_raw(raw: RawValue, limits: Limits, format: &'static str) -> Result<Scalar> {
    decode(raw, 0, &mut DecodeState::new(limits, format))
}

fn decode(raw: RawValue, depth: usize, state: &mut DecodeState) -> Result<Scalar> {
    state.visit(depth)?;
    match raw {
        RawValue::Null => Ok(Scalar::Null),
        RawValue::Bool(value) => Ok(Scalar::from(value)),
        RawValue::I64(value) => Ok(Scalar::from(value)),
        RawValue::U64(value) => Ok(Scalar::from(value)),
        RawValue::I128(value) => Ok(Scalar::from(value)),
        RawValue::U128(value) => Ok(Scalar::from(value)),
        RawValue::Float(value) => Ok(Scalar::from(value)),
        RawValue::String(value) => Ok(Scalar::from(value)),
        RawValue::Bytes(value) => Ok(Scalar::from(value)),
        RawValue::Sequence(values) => values
            .into_iter()
            .map(|value| decode(value, depth.saturating_add(1), state))
            .collect::<Result<Vec<_>>>()
            .map(Scalar::from_sequence),
        RawValue::Mapping(entries) => decode_mapping(entries, None, depth, state),
        RawValue::YamlMergeKey => Ok(Scalar::from("<<")),
        RawValue::YamlMapping(entries, positions) => {
            decode_mapping(entries, Some(&positions), depth, state)
        }
        // A YAML tag is an annotation. Scalar has no tag carrier, so standard
        // and application tags retain their natural payload.
        RawValue::YamlTagged(value) => decode(*value, depth, state),
    }
}

fn decode_mapping(
    entries: Vec<(RawValue, RawValue)>,
    positions: Option<&[usize]>,
    depth: usize,
    state: &mut DecodeState,
) -> Result<Scalar> {
    let entries = entries
        .into_iter()
        .map(|(key, value)| {
            Ok((
                decode(key, depth.saturating_add(1), state)?,
                decode(value, depth.saturating_add(1), state)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    if entries.iter().all(|(key, _)| key.as_str().is_some()) {
        let record = entries.into_iter().map(|(key, value)| {
            let Scalar::Text(crate::types::Text::Utf8(name)) = key else {
                unreachable!("the record predicate accepted only strings")
            };
            (name.into_inner(), value)
        });
        return Scalar::from_record(record).map_err(|error| positioned(error, positions, state));
    }

    Scalar::from_mapping(entries).map_err(|error| positioned(error, positions, state))
}

fn positioned(error: Error, positions: Option<&[usize]>, state: &DecodeState) -> Error {
    match error {
        Error::Codec {
            format: "value",
            position,
            reason,
        } => Error::Codec {
            format: state.format,
            position: positions
                .and_then(|positions| positions.get(position))
                .copied()
                .unwrap_or_default(),
            reason,
        },
        other => other,
    }
}

fn codec_error(format: &'static str, position: usize, reason: &'static str) -> Error {
    Error::Codec {
        format,
        position,
        reason: SmolStr::new_static(reason),
    }
}
