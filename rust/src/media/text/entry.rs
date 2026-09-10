//! The key/value tree one text line carries.

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, FieldPath, FieldSegment, Result};

use super::TextBytes;

/// How deep the entry scan descends into a nested payload.
///
/// A line is bounded input from outside, so the descent is bounded here rather
/// than by the stack. Two levels is what the shapes this reads actually have -
/// a frame, and one pair-shaped payload inside it - and a third is evidence of
/// something other than a message.
pub(crate) const MAX_ENTRY_DEPTH: usize = 8;

/// One key and value a line declared, with whatever it nested.
///
/// Keys and values are ranges into the page the line was read into, not owned
/// buffers: materializing the tree costs one vector, never a copy of any byte
/// the entries name.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextEntry {
    key: TextBytes,
    value: TextBytes,
    entries: Option<TextEntries>,
}

impl TextEntry {
    /// Pair one key with one value.
    #[must_use]
    pub const fn new(key: TextBytes, value: TextBytes) -> Self {
        Self {
            key,
            value,
            entries: None,
        }
    }

    /// Return this entry with a nested tree.
    #[must_use]
    pub fn with_entries(mut self, entries: TextEntries) -> Self {
        self.entries = Some(entries);
        self
    }

    /// Borrow the key.
    #[must_use]
    pub const fn key(&self) -> &TextBytes {
        &self.key
    }

    /// Borrow the value.
    #[must_use]
    pub const fn value(&self) -> &TextBytes {
        &self.value
    }

    /// Replace the value.
    pub fn set_value(&mut self, value: TextBytes) {
        self.value = value;
    }

    /// Borrow the nested tree.
    #[must_use]
    pub const fn entries(&self) -> Option<&TextEntries> {
        self.entries.as_ref()
    }

    /// Borrow the nested tree for mutation.
    pub const fn entries_mut(&mut self) -> Option<&mut TextEntries> {
        self.entries.as_mut()
    }

    /// Set or clear the nested tree.
    pub fn set_entries(&mut self, entries: Option<TextEntries>) {
        self.entries = entries;
    }
}

impl fmt::Display for TextEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}={}",
            self.key.as_str().unwrap_or("?"),
            self.value.as_str().unwrap_or("?")
        )
    }
}

/// The ordered entries one line or one nested payload declared.
///
/// Insertion order, not sorted: a frame states its fields in an order that is
/// part of what it said, and a repeated key is two entries rather than one
/// overwriting the other.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextEntries {
    entries: Vec<TextEntry>,
}

impl TextEntries {
    /// An empty tree.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// An empty tree sized for a known entry count.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    /// Borrow the entries in the order the line declared them.
    #[must_use]
    pub fn as_slice(&self) -> &[TextEntry] {
        &self.entries
    }

    /// The number of direct entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether this tree holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add one entry at the end.
    pub fn push(&mut self, entry: TextEntry) {
        self.entries.push(entry);
    }

    /// Iterate the direct entries.
    pub fn iter(&self) -> std::slice::Iter<'_, TextEntry> {
        self.entries.iter()
    }

    /// The entry a path reaches.
    ///
    /// A miss is `None` rather than an error: a path naming something this line
    /// did not carry is the ordinary case a caller lifts a path for, and
    /// absence is not a failure on the read path anywhere else in this crate.
    #[must_use]
    pub fn get_entry_by_path(&self, path: &FieldPath) -> Option<&TextEntry> {
        let (last, head) = path.segments().split_last()?;
        let mut entries = self;
        for segment in head {
            entries = entries.get_direct(segment)?.entries()?;
        }
        entries.get_direct(last)
    }

    /// The entry a path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the path when no entry is there.
    pub fn entry_by_path(&self, path: &FieldPath) -> Result<&TextEntry> {
        self.get_entry_by_path(path).ok_or_else(|| absent(path))
    }

    /// The entry a path reaches, for mutation.
    pub fn get_entry_by_path_mut(&mut self, path: &FieldPath) -> Option<&mut TextEntry> {
        let mut entries = self;
        let mut segments = path.segments().iter().peekable();
        while let Some(segment) = segments.next() {
            let index = entries.index_of(segment)?;
            if segments.peek().is_none() {
                return entries.entries.get_mut(index);
            }
            entries = entries.entries.get_mut(index)?.entries.as_mut()?;
        }
        None
    }

    /// Set the value a path reaches, creating what is not there.
    ///
    /// A setter that only overwrote what a line happened to carry would be
    /// useless for the case a caller reaches for it, which is enriching a line
    /// before writing it back. An index segment is the exception: a position
    /// names an entry that already exists, so a position past the end is a
    /// refusal rather than a silent extension.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for an empty path, or for a position
    /// segment that names no entry. Failure leaves this tree unchanged.
    pub fn set_entry_by_path(&mut self, path: &FieldPath, value: TextBytes) -> Result<()> {
        if path.is_root() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.entries"),
                reason: SmolStr::new_static("expected a path naming an entry, got the root"),
            });
        }
        // Every position segment is checked before anything is written, so a
        // refusal in the middle of a deep path leaves nothing half-created.
        self.check_positions(path)?;
        let mut entries = self;
        let mut segments = path.segments().iter().peekable();
        while let Some(segment) = segments.next() {
            let index = match entries.index_of(segment) {
                Some(index) => index,
                None => {
                    let key = TextBytes::from_bytes(
                        segment
                            .as_name()
                            .ok_or_else(|| position_absent(path, segment))?
                            .as_bytes(),
                    )?;
                    entries.entries.push(TextEntry::new(key, TextBytes::new()));
                    entries.entries.len() - 1
                }
            };
            let entry = &mut entries.entries[index];
            if segments.peek().is_none() {
                entry.value = value;
                return Ok(());
            }
            entries = entry.entries.get_or_insert_with(TextEntries::new);
        }
        Ok(())
    }

    /// Remove the entry a path reaches.
    pub fn remove_entry_by_path(&mut self, path: &FieldPath) -> Option<TextEntry> {
        let (last, head) = path.segments().split_last()?;
        let mut entries = self;
        for segment in head {
            let index = entries.index_of(segment)?;
            entries = entries.entries.get_mut(index)?.entries.as_mut()?;
        }
        let index = entries.index_of(last)?;
        Some(entries.entries.remove(index))
    }

    /// The direct entry one segment names.
    fn get_direct(&self, segment: &FieldSegment) -> Option<&TextEntry> {
        self.entries.get(self.index_of(segment)?)
    }

    /// Where one segment lands among the direct entries.
    ///
    /// A name matches an entry whose key is exactly those bytes. A key that is
    /// not valid text is reachable by position and is not reachable by name,
    /// because transliterating it would invent a spelling the line never wrote.
    fn index_of(&self, segment: &FieldSegment) -> Option<usize> {
        if let Some(position) = segment.as_index() {
            let len = self.entries.len();
            let index = if position < 0 {
                len.checked_sub(position.unsigned_abs() as usize)?
            } else {
                usize::try_from(position).ok()?
            };
            return (index < len).then_some(index);
        }
        let name = segment.as_name()?.as_bytes();
        self.entries
            .iter()
            .position(|entry| entry.key.as_bytes() == name)
    }

    /// Refuse every position segment that names no entry, before writing any.
    fn check_positions(&self, path: &FieldPath) -> Result<()> {
        let mut entries = Some(self);
        for segment in path {
            let Some(current) = entries else {
                // A position under a branch that does not exist yet cannot be
                // created, because a position names an existing entry.
                if segment.as_index().is_some() {
                    return Err(position_absent(path, segment));
                }
                return Ok(());
            };
            if segment.as_index().is_some() && current.index_of(segment).is_none() {
                return Err(position_absent(path, segment));
            }
            entries = current
                .index_of(segment)
                .and_then(|index| current.entries.get(index))
                .and_then(TextEntry::entries);
        }
        Ok(())
    }
}

/// Read every pair one line declares into a tree.
///
/// The bytes are ranges of the page `body` already points into, so a tree costs
/// one vector per level and copies nothing the line holds.
///
/// Nesting descends only into a value that is itself pair-shaped - the mixed
/// form, where a numeric envelope carries a payload stating its own fields -
/// and stops at [`MAX_ENTRY_DEPTH`]. A line is bounded input from outside, so
/// the descent is bounded here rather than by the stack.
pub(crate) fn read_entries(body: &TextBytes) -> Option<TextEntries> {
    read_entries_at(body, 0)
}

fn read_entries_at(body: &TextBytes, depth: usize) -> Option<TextEntries> {
    if depth >= MAX_ENTRY_DEPTH {
        return None;
    }
    let bytes = body.as_bytes();
    let mut entries: Option<TextEntries> = None;
    for span in crate::mime_type::line::entry_spans(bytes) {
        let (Ok(key), Ok(value)) = (
            body.slice(span.key.start, span.key.end),
            body.slice(span.value.start, span.value.end),
        ) else {
            continue;
        };
        let mut entry = TextEntry::new(key, value);
        if span.nested {
            if let Some(nested) = read_entries_at(entry.value(), depth + 1) {
                entry.set_entries(Some(nested));
            }
        }
        entries.get_or_insert_with(TextEntries::new).push(entry);
    }
    entries
}

impl<'a> IntoIterator for &'a TextEntries {
    type Item = &'a TextEntry;
    type IntoIter = std::slice::Iter<'a, TextEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl FromIterator<TextEntry> for TextEntries {
    fn from_iter<I: IntoIterator<Item = TextEntry>>(entries: I) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }
}

impl fmt::Display for TextEntries {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, entry) in self.entries.iter().enumerate() {
            if index > 0 {
                formatter.write_str(" ")?;
            }
            write!(formatter, "{entry}")?;
        }
        Ok(())
    }
}

/// The refusal a raising lookup answers with.
fn absent(path: &FieldPath) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.entries.{path}"),
        reason: format_smolstr!("expected an entry at {path}, got none"),
    }
}

/// The refusal a position segment naming no entry answers with.
fn position_absent(path: &FieldPath, segment: &FieldSegment) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.entries.{path}"),
        reason: format_smolstr!(
            "expected an entry at position {segment}, got none; a position names an entry that is already there"
        ),
    }
}
