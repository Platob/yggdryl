//! One parsed source expanding lazily into independently typed messages.

use std::sync::Arc;

use super::build::RowStamp;
use super::{FixCodec, FixMsg, UlPlugins};
use crate::Result;
use crate::media::text::TextEntries;

enum Source {
    Empty,
    One(Option<Result<FixMsg>>),
    Configs {
        codec: FixCodec,
        values: UlPlugins,
    },
    /// The frames one row still holds, read where they open.
    Frames {
        codec: FixCodec,
        /// The row's own entries, every message's keys and values ranges of
        /// the one page behind them.
        entries: TextEntries,
        /// Where the next frame opens.
        at: usize,
        /// What the row stated, retained once for every message it answers
        /// for (decision 8).
        stamp: Option<Arc<RowStamp>>,
    },
}

/// Messages from one captured line or record.
///
/// A row yields none, one or many (decision 16): a line carrying several
/// frames yields one per frame, re-entering the frame reader where each
/// opens over the page the row already holds, and a bulk UL configuration
/// document yields one item for each configuration, retaining only the
/// parsed source document and the current conversion. Nothing is collected.
/// An error is yielded once and ends this iterator.
pub struct FixMessages {
    source: Source,
}

impl FixMessages {
    pub(super) fn one(message: FixMsg) -> Self {
        Self {
            source: Source::One(Some(Ok(message))),
        }
    }

    /// What a row carrying no message at all answers.
    pub(super) const fn none() -> Self {
        Self {
            source: Source::Empty,
        }
    }

    /// The frames a row holds from `at` on, read one at a time.
    pub(super) const fn frames(
        codec: FixCodec,
        entries: TextEntries,
        at: usize,
        stamp: Option<Arc<RowStamp>>,
    ) -> Self {
        Self {
            source: Source::Frames {
                codec,
                entries,
                at,
                stamp,
            },
        }
    }

    pub(super) fn from_result(result: Result<Self>) -> Self {
        result.unwrap_or_else(|error| Self {
            source: Source::One(Some(Err(error))),
        })
    }

    pub(super) fn from_ulconfigs(codec: FixCodec, values: UlPlugins) -> Self {
        Self {
            source: Source::Configs { codec, values },
        }
    }
}

impl Iterator for FixMessages {
    type Item = Result<FixMsg>;

    fn next(&mut self) -> Option<Self::Item> {
        let value = match &mut self.source {
            Source::Empty => None,
            Source::One(value) => value.take(),
            Source::Configs { codec, values } => {
                values.next().map(|value| value.into_fixmsg(codec))
            }
            Source::Frames {
                codec,
                entries,
                at,
                stamp,
            } => codec.message_at(entries, *at, stamp.as_ref()).map(|read| {
                *at = read.next;
                read.message
            }),
        };
        if value.as_ref().is_none_or(Result::is_err) {
            self.source = Source::Empty;
        }
        value
    }
}

impl std::iter::FusedIterator for FixMessages {}
