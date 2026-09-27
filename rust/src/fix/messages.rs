//! One parsed source expanding lazily into independently typed messages.

use std::sync::Arc;

use super::build::RowStamp;
use super::{FixCodec, FixMsg};
use crate::Result;
use crate::text::TextEntries;

enum Source {
    Empty,
    One(Option<Result<FixMsg>>),
    /// The messages a row held, read on another thread and carried over,
    /// each already split.
    Many(std::vec::IntoIter<Result<FixMsg>>),
    /// The frames one row still holds, read where they open.
    Frames {
        codec: FixCodec,
        /// The row's own entries, every message's keys and values ranges of
        /// the one page behind them.
        entries: TextEntries,
        /// Where the next frame opens.
        at: usize,
        /// What the row stated, retained once for every message it answers
        /// for.
        stamp: Option<Arc<RowStamp>>,
    },
}

/// Messages from one captured line or record.
///
/// A row yields none, one or many: a line carrying several
/// frames yields one per frame, re-entering the frame reader where each
/// opens over the page the row already holds, and each message read is
/// followed by the messages it splits into - an order's execution, a
/// trade's sided executions, an unsided quote's sided quotes - which the
/// parse splits off once, here. Nothing is collected. An error is yielded
/// once and ends this iterator.
pub struct FixMessages {
    source: Source,
    /// The messages the last one read split off, still to yield.
    split: std::vec::IntoIter<FixMsg>,
}

impl FixMessages {
    pub(super) fn one(message: FixMsg) -> Self {
        Self::of(Source::One(Some(Ok(message))))
    }

    /// What a row carrying no message at all answers.
    pub(super) fn none() -> Self {
        Self::of(Source::Empty)
    }

    fn of(source: Source) -> Self {
        Self {
            source,
            split: Vec::new().into_iter(),
        }
    }

    /// The frames a row holds from `at` on, read one at a time.
    pub(super) fn frames(
        codec: FixCodec,
        entries: TextEntries,
        at: usize,
        stamp: Option<Arc<RowStamp>>,
    ) -> Self {
        Self::of(Source::Frames {
            codec,
            entries,
            at,
            stamp,
        })
    }

    pub(super) fn from_result(result: Result<Self>) -> Self {
        result.unwrap_or_else(|error| Self::of(Source::One(Some(Err(error)))))
    }

    /// The same messages, read here and now: what a door reading on
    /// several threads hands back, so the reading happens on the thread
    /// that was given the row rather than on the one that pulls. A row's
    /// frames are the one source with reading left to do; every other is
    /// already read, and is handed back as it is.
    pub(super) fn collected(self) -> Self {
        match self.source {
            Source::Frames { .. } => Self::of(Source::Many(self.collect::<Vec<_>>().into_iter())),
            source => Self {
                source,
                split: self.split,
            },
        }
    }
}

impl Iterator for FixMessages {
    type Item = Result<FixMsg>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(split) = self.split.next() {
            return Some(Ok(split));
        }
        let value = match &mut self.source {
            Source::Empty => None,
            Source::One(value) => value.take(),
            // Already split where it was read.
            Source::Many(read) => return read.next(),
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
        value.map(|read| {
            read.map(|message| {
                let (message, split) = message.split();
                self.split = split.into_iter();
                message
            })
        })
    }
}

impl std::iter::FusedIterator for FixMessages {}
