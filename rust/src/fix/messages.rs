//! One parsed source expanding lazily into independently typed messages.

use std::sync::Arc;

use super::build::RowStamp;
use super::{FixCodec, FixMsg};
use crate::graph::element::InstantSequence;
use crate::logging::warning::warned;
use crate::text::TextEntries;
use crate::{Error, Result};

enum Source {
    Empty,
    One(Option<Result<FixMsg>>),
    /// The messages a row held, read on another thread and carried over,
    /// each already split and every refusal already passed over.
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
/// trade's sided executions, a batch's entries - which the parse splits off
/// once, here. Each message takes its
/// [place](crate::graph::Event::get_seqnum) among the row's messages of its
/// instant, so a report and the execution split off it are places zero and
/// one, each naming its source by the identity its place gave it. Nothing
/// is collected.
///
/// What a frame states that will not type is null beside an anomaly and a
/// warning - a clock naming no instant is left unstated, the message dated
/// as one stating none is - and a row that is not a row at all is excluded
/// with a warning: the frames after it still read. A source failure is
/// yielded after the messages before it and ends this iterator.
///
/// ```
/// # fn main() -> yggdryl::Result<()> {
/// # use std::sync::Arc;
/// # use yggdryl::{FixCodec, FixRegistry};
/// let codec = FixCodec::new(Arc::new(FixRegistry::new()));
/// // The first frame's sending clock names no instant: the frame still
/// // stands, its clock unstated, and says so as an anomaly.
/// let row = b"8=FIX.4.4|35=D|52=bad|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|";
/// let read: Vec<_> = codec.parse_line(row)?.collect::<yggdryl::Result<_>>()?;
/// assert_eq!(read.len(), 2);
/// assert_eq!(read[0].anomalies()[0].field(), "sendingtime");
/// assert_eq!(read[1].by_tag(11)?.as_str(), Some("B"));
/// # Ok(())
/// # }
/// ```
pub struct FixMessages {
    source: Source,
    /// The messages the last one read split off, still to yield.
    split: std::vec::IntoIter<FixMsg>,
    /// The place each message yielded takes among the row's messages of
    /// its instant.
    sequence: InstantSequence,
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
            sequence: InstantSequence::default(),
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

    /// The messages `result` holds, or the refusal it is: excluded with a
    /// warning where the data refused, yielded where the source failed.
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
                sequence: self.sequence,
            },
        }
    }
}

impl Iterator for FixMessages {
    type Item = Result<FixMsg>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut held = self.next_unplaced()?;
        if let Ok(message) = &mut held {
            self.sequence.place_naming_sources(message);
        }
        Some(held)
    }
}

impl FixMessages {
    /// The next message, split but not yet placed.
    fn next_unplaced(&mut self) -> Option<Result<FixMsg>> {
        loop {
            if let Some(split) = self.split.next() {
                return Some(Ok(split));
            }
            let value = match &mut self.source {
                Source::Empty => None,
                Source::One(value) => value.take(),
                // Already split, and its refusals passed over, where it was
                // read.
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
            match value {
                None => {
                    self.source = Source::Empty;
                    return None;
                }
                Some(Ok(message)) => {
                    // The entries a batch splits into fill from the table
                    // the door fixed, as the batch did.
                    let instruments = match &self.source {
                        Source::Frames { codec, .. } => codec.instruments(),
                        _ => None,
                    };
                    let (message, split) = message.split(instruments.as_deref());
                    self.split = split.into_iter();
                    return Some(Ok(message));
                }
                // A frame that does not build is that frame alone: the row's
                // next frame opens where this one ended.
                Some(Err(error)) => {
                    if let Some(error) = source_failure(
                        error,
                        "FIX message excluded: what arrived does not build a message",
                    ) {
                        self.source = Source::Empty;
                        return Some(Err(error));
                    }
                }
            }
        }
    }
}

impl std::iter::FusedIterator for FixMessages {}

/// The source failure `error` is, handed back; or, where the data refused,
/// nothing, once the refusal is warned about under `what` - the item it
/// excluded and why.
///
/// The one reading every FIX stream gives an `Err` item: a line, a row or a
/// message that cannot stand is passed over and the stream reads on, and
/// only a reader, a store or a runtime that could not answer
/// ([`Error::is_source_failure`]) is an `Err` item, never ahead of what was
/// read before it.
pub(super) fn source_failure(error: Error, what: &'static str) -> Option<Error> {
    if error.is_source_failure() {
        return Some(error);
    }
    warned!(what, refused(&error), "{error}");
    None
}

/// What a warning about `error` is counted under: the field or path the
/// refusal names, else the reading that refused - never the value or the
/// row its text names, which would make every row a warning of its own.
pub(super) fn refused(error: &Error) -> &str {
    match error {
        Error::InvalidRecord { path, .. } | Error::Absent { path, .. } => path,
        Error::Parse { target, .. } => target,
        Error::Codec { format, .. } => format,
        Error::InvalidDataType { kind, .. } => kind,
        Error::Conflict { expected, .. } => expected,
        _ => "message",
    }
}
