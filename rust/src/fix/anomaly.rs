//! What a message says about itself that does not add up.
//!
//! All real, none fatal. A counter disagreeing with the group it introduces,
//! a value that would not type, an entry whose text is a lossy decode of the
//! bytes it arrived as - a reader that refused any of them would drop a row a
//! monitor exists to see, and one that repaired them would state something
//! the wire did not.
//!
//! They are **derived on demand** by comparing the row against the entries,
//! the way [`FixId`](super::FixId) is derived rather than stored. There is no
//! error channel on a message, nothing to keep in step with an edit, and a
//! caller who never asks pays nothing.

use std::fmt;

use super::entry::FixEntry;
use super::msg::FixMsg;
use crate::DataType;

/// The replacement character a lossy decode leaves behind.
const REPLACEMENT: char = '\u{FFFD}';

/// One disagreement between what arrived and what the row made of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixAnomaly<'msg> {
    /// A value arrived and would not type, so the row holds null instead.
    ///
    /// The text is still here, exactly as it arrived: a null nobody can
    /// explain is worse than the value that actually came in.
    Untyped {
        /// The tag the key named, `0` when it named none.
        tag: i32,
        /// The key as it arrived.
        key: &'msg str,
        /// The value as it arrived, untyped.
        value: &'msg str,
    },
    /// A repeating group's counter states a count the group does not have.
    ///
    /// Stated rather than corrected. A filtered occurrence (P7-R87) and a
    /// truncated capture both land here, and renumbering either would hide
    /// which one happened.
    Miscounted {
        /// The counter's tag.
        tag: i32,
        /// The group's canonical name in the row.
        name: &'msg str,
        /// What the counter said.
        stated: i64,
        /// What the row holds.
        held: usize,
    },
    /// An entry's text is a lossy decode, so the row is the authority.
    ///
    /// A `data` field's bytes are not text, and the entry holds a decode of
    /// them only so the record is readable. Re-emitting from this entry
    /// would write the replacement character onto the wire.
    Lossy {
        /// The tag the key named.
        tag: i32,
        /// The key as it arrived.
        key: &'msg str,
    },
}

impl FixAnomaly<'_> {
    /// The tag this anomaly is about, `0` when the key named no field.
    #[must_use]
    pub const fn tag(&self) -> i32 {
        match self {
            Self::Untyped { tag, .. } | Self::Miscounted { tag, .. } | Self::Lossy { tag, .. } => {
                *tag
            }
        }
    }
}

impl fmt::Display for FixAnomaly<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Untyped { tag, key, value } => {
                write!(formatter, "{key} ({tag}) would not type from {value:?}")
            }
            Self::Miscounted {
                tag,
                name,
                stated,
                held,
            } => write!(
                formatter,
                "{name} ({tag}) states {stated} occurrences and holds {held}"
            ),
            Self::Lossy { tag, key } => {
                write!(formatter, "{key} ({tag}) decoded lossily and is not text")
            }
        }
    }
}

/// Every disagreement one message holds, derived as it is walked.
pub struct FixAnomalies<'msg> {
    message: &'msg FixMsg,
    entries: std::slice::Iter<'msg, FixEntry>,
}

impl<'msg> FixAnomalies<'msg> {
    pub(super) fn new(message: &'msg FixMsg) -> Self {
        Self {
            message,
            entries: message.entries().iter(),
        }
    }

    /// The one anomaly an entry carries, when it carries one.
    ///
    /// At most one per entry and in this order: a lossy decode says the text
    /// is not the authority, which makes any further reading of it
    /// meaningless; then a miscount, which is about the group rather than
    /// this value; then the value's own typing.
    fn anomaly(&self, entry: &'msg FixEntry) -> Option<FixAnomaly<'msg>> {
        if entry.value().contains(REPLACEMENT) {
            return Some(FixAnomaly::Lossy {
                tag: entry.tag(),
                key: entry.key(),
            });
        }
        if let Some(anomaly) = self.miscount(entry) {
            return Some(anomaly);
        }
        // A row's null where a value arrived is a value that would not type:
        // a stated absence produced no entry at all (P7-R85), and a gapped
        // occurrence was never stated, so neither reaches here.
        let held = self.message.get_by_tag(entry.tag())?;
        if held.is_null() && !entry.value().is_empty() {
            return Some(FixAnomaly::Untyped {
                tag: entry.tag(),
                key: entry.key(),
                value: entry.value(),
            });
        }
        None
    }

    /// The disagreement between a counter and the group it introduces.
    ///
    /// Only a counter states a count. A column holding a List of values is a
    /// tag that arrived twice, not a group, and reading the second
    /// `PartyRole=1` as a count would invent one - so the column has to be
    /// the group's own shape, a List of `item` Structs, before its value is
    /// read as a number of occurrences at all.
    fn miscount(&self, entry: &'msg FixEntry) -> Option<FixAnomaly<'msg>> {
        let stated = entry.value().parse::<i64>().ok()?;
        let held = self.message.get_by_tag(entry.tag())?.as_sequence()?;
        let index = self.message.index_of_tag(entry.tag())?;
        let column = self.message.as_field().dtype().as_fields()?.get(index)?;
        let (DataType::List(item) | DataType::LargeList(item)) = column.dtype() else {
            return None;
        };
        if !item.dtype().is_nested() {
            return None;
        }
        let name = column.name();
        if i64::try_from(held.len()).is_ok_and(|count| count == stated) {
            return None;
        }
        Some(FixAnomaly::Miscounted {
            tag: entry.tag(),
            name,
            stated,
            held: held.len(),
        })
    }
}

impl<'msg> Iterator for FixAnomalies<'msg> {
    type Item = FixAnomaly<'msg>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = self.entries.next()?;
            if let Some(anomaly) = self.anomaly(entry) {
                return Some(anomaly);
            }
        }
    }
}

impl std::iter::FusedIterator for FixAnomalies<'_> {}
