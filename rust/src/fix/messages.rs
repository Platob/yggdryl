//! One parsed source expanding lazily into independently typed messages.

use super::{FixCodec, FixMsg, UlPlugins};
use crate::Result;

enum Source {
    Empty,
    One(Option<Result<FixMsg>>),
    Configs {
        codec: FixCodec,
        values: UlPlugins,
        enrich: bool,
    },
}

/// Messages from one captured line or record.
///
/// Ordinary FIX frames yield one item. A bulk UL configuration document yields
/// one item for each configuration, retaining only the parsed source document
/// and the current conversion. An error is yielded once and ends this iterator.
pub struct FixMessages {
    source: Source,
}

impl FixMessages {
    pub(super) fn one(message: FixMsg) -> Self {
        Self {
            source: Source::One(Some(Ok(message))),
        }
    }

    pub(super) fn from_result(result: Result<Self>) -> Self {
        result.unwrap_or_else(|error| Self {
            source: Source::One(Some(Err(error))),
        })
    }

    pub(super) fn from_ulconfigs(codec: FixCodec, values: UlPlugins, enrich: bool) -> Self {
        Self {
            source: Source::Configs {
                codec,
                values,
                enrich,
            },
        }
    }
}

impl Iterator for FixMessages {
    type Item = Result<FixMsg>;

    fn next(&mut self) -> Option<Self::Item> {
        let value = match &mut self.source {
            Source::Empty => None,
            Source::One(value) => value.take(),
            Source::Configs {
                codec,
                values,
                enrich,
            } => values.next().map(|value| value.into_fixmsg(codec, *enrich)),
        };
        if value.as_ref().is_none_or(Result::is_err) {
            self.source = Source::Empty;
        }
        value
    }
}

impl std::iter::FusedIterator for FixMessages {}
