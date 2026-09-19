//! Bounded TOML 1.1 decoding from the parser's borrowed, spanned value tree.

use crate::text::Limits;
use crate::{Error, Result, Scalar};

use super::wire;

pub(super) fn parse(input: &str, limits: Limits) -> Result<Scalar> {
    if limits.max_documents() == 0 {
        return Err(codec_error(0, "document limit exceeded"));
    }
    let root = toml::de::DeTable::parse(input).map_err(toml_error)?;
    let position = root.span().start;
    let mut state = State { limits, nodes: 0 };
    state.convert_table(root.into_inner(), 0, position)
}

struct State {
    limits: Limits,
    nodes: usize,
}

impl State {
    fn observe_node(&mut self, position: usize) -> Result<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes() {
            Err(codec_error(position, "decoded node limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn observe_container_depth(&self, depth: usize, position: usize) -> Result<()> {
        let depth = depth.saturating_add(1);
        if depth > super::MAX_PARSER_DEPTH {
            Err(codec_error(
                position,
                "TOML nesting exceeds the parser hard limit of 64",
            ))
        } else if depth > self.limits.max_depth() {
            Err(codec_error(position, "nesting depth limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn convert_value(
        &mut self,
        value: toml::de::DeValue<'_>,
        depth: usize,
        position: usize,
    ) -> Result<Scalar> {
        self.observe_node(position)?;
        match value {
            toml::de::DeValue::String(value) => Ok(Scalar::from(value.into_owned())),
            toml::de::DeValue::Integer(value) => i64::from_str_radix(value.as_str(), value.radix())
                .map(Scalar::from)
                .map_err(|_| {
                    codec_error(position, "TOML integer is outside the signed 64-bit range")
                }),
            toml::de::DeValue::Float(value) => parse_float(value.as_str())
                .map(Scalar::from)
                .ok_or_else(|| codec_error(position, "TOML float is outside the f64 range")),
            toml::de::DeValue::Boolean(value) => Ok(Scalar::from(value)),
            toml::de::DeValue::Datetime(value) => {
                wire::datetime_value(value).map_err(|error| at_position(error, position))
            }
            toml::de::DeValue::Array(values) => {
                self.observe_container_depth(depth, position)?;
                let mut decoded = Vec::with_capacity(values.len());
                for value in values {
                    let position = value.span().start;
                    decoded.push(self.convert_value(
                        value.into_inner(),
                        depth.saturating_add(1),
                        position,
                    )?);
                }
                Ok(Scalar::from_sequence(decoded))
            }
            toml::de::DeValue::Table(values) => {
                self.convert_table_after_node(values, depth, position)
            }
        }
    }

    fn convert_table(
        &mut self,
        values: toml::de::DeTable<'_>,
        depth: usize,
        position: usize,
    ) -> Result<Scalar> {
        self.observe_node(position)?;
        self.convert_table_after_node(values, depth, position)
    }

    fn convert_table_after_node(
        &mut self,
        values: toml::de::DeTable<'_>,
        depth: usize,
        position: usize,
    ) -> Result<Scalar> {
        self.observe_container_depth(depth, position)?;
        let mut decoded = Vec::with_capacity(values.len());
        for (key, value) in values {
            let key_position = key.span().start;
            self.observe_node(key_position)?;
            let value_position = value.span().start;
            decoded.push((
                key.into_inner().into_owned(),
                self.convert_value(value.into_inner(), depth.saturating_add(1), value_position)?,
            ));
        }
        wire::decode_table(decoded).map_err(|error| at_position(error, position))
    }
}

/// Convert one TOML float spelling, returning `None` when it cannot be held.
///
/// A finite spelling whose magnitude exceeds the f64 range is rejected, so a
/// document never gains an infinity it did not itself spell. A non-zero
/// spelling that rounds to zero, such as `1e-400`, is instead accepted as a
/// signed zero: that is ordinary IEEE-754 rounding, and rejecting it would
/// reject output any conforming producer may emit. The sign survives, so
/// `-1e-400` decodes to negative zero rather than positive zero.
fn parse_float(value: &str) -> Option<f64> {
    match value {
        "inf" | "+inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        "nan" | "+nan" | "-nan" => Some(f64::NAN),
        value => value.parse().ok().filter(|value: &f64| value.is_finite()),
    }
}

fn toml_error(error: toml::de::Error) -> Error {
    Error::Codec {
        format: "toml",
        position: error.span().map_or(0, |span| span.start),
        reason: error.message().into(),
    }
}

fn at_position(error: Error, position: usize) -> Error {
    match error {
        Error::Codec { reason, .. } => Error::Codec {
            format: "toml",
            position,
            reason,
        },
        error => error,
    }
}

fn codec_error(position: usize, reason: &'static str) -> Error {
    Error::Codec {
        format: "toml",
        position,
        reason: reason.into(),
    }
}
