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

use std::collections::HashMap;
use std::fmt;

use super::entry::FixEntry;
use super::group_plan::GroupPlan;
use super::msg::FixMsg;
use super::{FixId, MsgType};
use crate::{DataType, Field, Scalar};

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
    // A stack of levels rather than one slice: the walk is pre-order, so a
    // nested anomaly is reported exactly as a flat one is.
    entries: Vec<std::slice::Iter<'msg, FixEntry>>,
    // One lazy schema/value cursor per declared counter tag. Only requested
    // cursors allocate their depth-bounded stack; each drains the tree once.
    groups: HashMap<i32, GroupCursor<'msg>>,
    msgtype: Option<&'msg MsgType>,
    // Numeric arrivals follow compiled group delimiters. Only the active
    // contexts are held (at most 64), so omitted nullable members never shift
    // a later occurrence's value onto an earlier raw entry.
    numeric: Vec<NumericContext<'msg>>,
}

struct NumericContext<'msg> {
    plan: &'msg GroupPlan,
    rows: &'msg [Scalar],
    row: Option<usize>,
}

impl<'msg> NumericContext<'msg> {
    fn advance(&mut self, tag: i32) -> Option<&'msg Scalar> {
        self.row = Some(match self.row {
            Some(row) if self.plan.delimiter() == Some(tag) => row.checked_add(1)?,
            Some(row) => row,
            None => 0,
        });
        self.plan.column_value(self.rows.get(self.row?)?, tag)
    }
}

#[derive(Clone, Copy)]
struct GroupValue<'msg> {
    field: &'msg Field,
    counter: Option<&'msg Scalar>,
    occurrences: &'msg [Scalar],
}

struct GroupNode<'msg> {
    field: &'msg Field,
    value: &'msg Scalar,
    parent: Option<(&'msg Field, &'msg [Scalar])>,
}

struct GroupFrame<'msg> {
    field: &'msg Field,
    values: &'msg [Scalar],
    position: usize,
    list: bool,
}

impl<'msg> GroupFrame<'msg> {
    fn next(&mut self) -> Option<GroupNode<'msg>> {
        let value = self.values.get(self.position)?;
        let (field, parent) = if self.list {
            (self.field, None)
        } else {
            (
                self.field.fields().get(self.position)?,
                Some((self.field, self.values)),
            )
        };
        self.position += 1;
        Some(GroupNode {
            field,
            value,
            parent,
        })
    }
}

struct GroupCursor<'msg> {
    tag: i32,
    root: Option<GroupNode<'msg>>,
    stack: Vec<GroupFrame<'msg>>,
    /// The last instance this cursor answered.
    ///
    /// A counter a frame states twice at one level appends to the group the
    /// first statement opened rather than opening a second one, so the row
    /// holds one instance for two arrivals and every arrival is measured
    /// against it.
    last: Option<GroupValue<'msg>>,
}

impl<'msg> GroupCursor<'msg> {
    fn next(&mut self) -> Option<GroupValue<'msg>> {
        loop {
            let node = if let Some(root) = self.root.take() {
                root
            } else {
                loop {
                    let Some(level) = self.stack.last_mut() else {
                        return self.last;
                    };
                    if let Some(node) = level.next() {
                        break node;
                    }
                    self.stack.pop();
                }
            };
            let Some(values) = node.value.as_sequence() else {
                continue;
            };
            match node.field.dtype() {
                DataType::Struct(_) => {
                    if self.stack.len() < 64 {
                        self.stack.push(GroupFrame {
                            field: node.field,
                            values,
                            position: 0,
                            list: false,
                        });
                    }
                }
                DataType::List(item) | DataType::LargeList(item) => {
                    if self.stack.len() < 64 {
                        self.stack.push(GroupFrame {
                            field: item,
                            values,
                            position: 0,
                            list: true,
                        });
                    }
                    if node.field.as_fix().counter().ok().flatten() == Some(self.tag) {
                        let counter = node.parent.and_then(|(parent, row)| {
                            let index = parent.fields().iter().position(|field| {
                                field.as_fix().tag().ok().flatten() == Some(self.tag)
                            })?;
                            row.get(index)
                        });
                        let value = GroupValue {
                            field: node.field,
                            counter,
                            occurrences: values,
                        };
                        self.last = Some(value);
                        return Some(value);
                    }
                }
                _ => {}
            }
        }
    }
}

fn group_cursors<'msg>(
    field: &Field,
    message: &'msg FixMsg,
    groups: &mut HashMap<i32, GroupCursor<'msg>>,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    if let Some(tag) = field.as_fix().counter().ok().flatten() {
        groups.entry(tag).or_insert_with(|| GroupCursor {
            tag,
            root: Some(GroupNode {
                field: message.as_field(),
                value: message.as_value(),
                parent: None,
            }),
            stack: Vec::new(),
            last: None,
        });
    }
    match field.dtype() {
        DataType::List(item) | DataType::LargeList(item) => {
            group_cursors(item, message, groups, depth + 1)
        }
        _ => {
            for child in field.fields() {
                group_cursors(child, message, groups, depth + 1);
            }
        }
    }
}

impl<'msg> FixAnomalies<'msg> {
    pub(super) fn new(message: &'msg FixMsg) -> Self {
        let mut groups = HashMap::new();
        group_cursors(message.as_field(), message, &mut groups, 0);
        Self {
            message,
            entries: vec![message.entries().iter()],
            groups,
            msgtype: message
                .get_by_tag(35)
                .and_then(Scalar::as_str)
                .and_then(|code| {
                    message
                        .registry()
                        .known_msgtype(code, Some(message.branch()))
                }),
            numeric: Vec::new(),
        }
    }

    fn numeric_value(
        &mut self,
        entry: &FixEntry,
        group: Option<&GroupValue<'msg>>,
    ) -> Option<&'msg Scalar> {
        if super::field::parse_tag(entry.key()) != Some(entry.tag()) {
            self.numeric.clear();
            return None;
        }
        let mut value = None;
        while let Some(context) = self.numeric.last_mut() {
            if context.plan.tag_index(entry.tag()).is_some() {
                value = context.advance(entry.tag());
                break;
            }
            self.numeric.pop();
        }
        if let Some(group) = group {
            let nested = self
                .numeric
                .last()
                .and_then(|context| context.plan.nested(entry.tag()).map(|(_, plan)| plan));
            let plan = nested.or_else(|| {
                let branch = group.field.as_fix().branch().ok()?;
                let id = FixId::from_parts(&branch, entry.tag()).ok()?;
                match self.msgtype.filter(|message| message.has_group_counter(id)) {
                    Some(message) => message.get_group_plan_by_counter(id),
                    None => self.message.registry().get_group_plan_by_counter(id),
                }
            });
            if let Some(plan) = plan.filter(|_| self.numeric.len() < 64) {
                self.numeric.push(NumericContext {
                    plan,
                    rows: group.occurrences,
                    row: None,
                });
            }
        }
        value
    }

    /// The one anomaly an entry carries, when it carries one.
    ///
    /// At most one per entry and in this order: a lossy decode says the text
    /// is not the authority, which makes any further reading of it
    /// meaningless; then a miscount, which is about the group rather than
    /// this value; then the value's own typing.
    fn anomaly(&mut self, entry: &'msg FixEntry) -> Option<FixAnomaly<'msg>> {
        let group = self
            .groups
            .get_mut(&entry.tag())
            .and_then(GroupCursor::next);
        let numeric = self.numeric_value(entry, group.as_ref());
        if entry.value().contains(REPLACEMENT) {
            return Some(FixAnomaly::Lossy {
                tag: entry.tag(),
                key: entry.key(),
            });
        }
        if let Some(anomaly) = group
            .as_ref()
            .and_then(|group| Self::miscount(entry, group))
        {
            return Some(anomaly);
        }
        // A row's null where a value arrived is a value that would not type:
        // a stated absence produced no entry at all (P7-R85), and a gapped
        // occurrence was never stated, so neither reaches here.
        let held = group
            .and_then(|group| group.counter)
            .or(numeric)
            .or_else(|| self.message.get_by_tag(entry.tag()))?;
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
    /// the group's own shape, a List of occurrence Structs, before its value is
    /// read as a number of occurrences at all.
    fn miscount(entry: &'msg FixEntry, group: &GroupValue<'msg>) -> Option<FixAnomaly<'msg>> {
        // A count outside int32 is an untyped scalar, even if its raw spelling
        // fits the wider arithmetic used for comparing occurrence lengths.
        // A counter the frame states twice at one level lands in a List of the
        // counts it stated, so the typed reading is looked for through the
        // column's own occurrences as well as in the column itself.
        //
        // The count compared is the column's typed reading, which is the
        // entry's own as the codec read it and the re-count where a pass
        // appended an occurrence the wire never stated - a message restated
        // at the newest version says of itself what it said before. Only the
        // twice-stated counter falls back to the entry's text.
        let counter = group.counter?;
        let typed = counter.as_i128().and_then(|held| i64::try_from(held).ok());
        if typed.is_none()
            && !counter
                .as_sequence()
                .is_some_and(|stated| stated.iter().any(|held| held.as_integer().is_some()))
        {
            return None;
        }
        let stated = match typed {
            Some(held) => held,
            None => entry.value().parse::<i64>().ok()?,
        };
        let held = group.occurrences;
        let column = group.field;
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
            let entry = loop {
                let level = self.entries.last_mut()?;
                match level.next() {
                    Some(held) => {
                        if !held.children().is_empty() {
                            self.entries.push(held.children().iter());
                        }
                        break held;
                    }
                    None => {
                        self.entries.pop();
                    }
                }
            };
            if let Some(anomaly) = self.anomaly(entry) {
                return Some(anomaly);
            }
        }
    }
}

impl std::iter::FusedIterator for FixAnomalies<'_> {}
