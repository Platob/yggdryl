//! The key/value tree one text line carries.

use std::borrow::Cow;
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
///
/// Two entries are one value when their keys, their values, their marks and
/// their nested trees agree. The mark counts because it is a fact about what
/// the line wrote and not a rendering of it: a bridge restating a pair writes
/// `#ORDERID=123` under an `ORDERID=123` it already sent, and an identity that
/// ignored the mark would answer that the line said one thing twice.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextEntry {
    key: TextBytes,
    value: TextBytes,
    marked: bool,
    entries: Option<TextEntries>,
}

impl TextEntry {
    /// Pair one key with one value.
    #[must_use]
    pub const fn new(key: TextBytes, value: TextBytes) -> Self {
        Self {
            key,
            value,
            marked: false,
            entries: None,
        }
    }

    /// Return this entry with a nested tree.
    #[must_use]
    pub fn with_entries(mut self, entries: TextEntries) -> Self {
        self.entries = Some(entries);
        self
    }

    /// Return this entry marked, or not, as the line wrote it.
    #[must_use]
    pub const fn with_marked(mut self, marked: bool) -> Self {
        self.marked = marked;
        self
    }

    /// The key, as text.
    ///
    /// Borrowed wherever the range is text, which on a line the reader made
    /// is always: the scanner cuts a key at `=` and at the bytes that end a
    /// field, all of them ASCII, so no range it cuts divides a character, and
    /// the line itself was decoded before it was scanned. A range a caller
    /// built from bytes of their own that are not text is answered as the
    /// lossy decode of it, owned; [`key_bytes`](Self::key_bytes) is the
    /// range either way.
    #[must_use]
    pub fn key(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(self.key.as_bytes())
    }

    /// The value, as text, exactly as [`key`](Self::key) answers the key.
    #[must_use]
    pub fn value(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(self.value.as_bytes())
    }

    /// The key as the range of its page, for a reader that works in offsets.
    #[must_use]
    pub const fn key_bytes(&self) -> &TextBytes {
        &self.key
    }

    /// The value as the range of its page, for a reader that works in
    /// offsets: the FIX codec re-slicing a data field to the length its `Len`
    /// field stated, or re-emitting a frame byte for byte.
    #[must_use]
    pub const fn value_bytes(&self) -> &TextBytes {
        &self.value
    }

    /// Replace the value.
    pub fn set_value(&mut self, value: TextBytes) {
        self.value = value;
    }

    /// Whether the line wrote a `#` in front of this key.
    ///
    /// A bridge marks a key to say that what follows restates a pair it
    /// already wrote, rather than stating a second one. The key range excludes
    /// the marker so that every path lifting a bridge key asks for the name
    /// the bridge gave the field, which is what makes the mark a fact of its
    /// own: after stripping there is nothing in the bytes to read it back
    /// from. What the mark *means* - a duplicate, a restatement, a group's
    /// stem, a stated absence - is a dialect's reading of it and belongs to
    /// whoever holds that dictionary.
    ///
    /// A mark arrives with the line and only with it. An entry a caller
    /// created, or one rebuilt from a lifted column, is unmarked: the column
    /// carries the value the path reached, and no column carries this.
    #[must_use]
    pub const fn marked(&self) -> bool {
        self.marked
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
        // The mark is written back, because two entries differing only in it
        // are two values and a rendering that hid one would show them alike.
        if self.marked {
            formatter.write_str("#")?;
        }
        write!(formatter, "{}={}", self.key(), self.value())
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

    /// Every pair one run of bytes declares, as ranges of the page it names.
    ///
    /// The one door onto the scanner, so a reader that holds a page - a whole
    /// line, or one field's value inside it - asks the same walk the text read
    /// asks and gets the same answer. Nothing is copied: `body` already points
    /// into a page and each key and value is a range of it, so a tree costs one
    /// vector per level and no byte of what it names.
    ///
    /// Where a value ends is the scanner's to say, and so is whether the line
    /// marked a key; both travel into the entry unchanged. Nesting descends
    /// only into a value that is itself pair-shaped - the mixed form, where a
    /// numeric envelope carries a payload stating its own fields - and stops
    /// eight levels down. A line is bounded input from outside, so the descent
    /// is bounded here rather than by the stack.
    ///
    /// `None` where the bytes declare no pair at all, which is the same answer
    /// [`TextLine::entries`](super::TextLine::entries) carries for a line
    /// nothing asked a tree of: an absence, never an empty tree.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// use yggdryl::media::text::{TextBytes, TextEntries};
    ///
    /// let body = TextBytes::from_bytes("8=FIX.4.4|35=D|58=a, b|10=0|")?;
    /// let entries = TextEntries::from_bytes(&body).expect("a framed line states pairs");
    /// let read: Vec<_> = entries.iter().map(ToString::to_string).collect();
    /// assert_eq!(read, ["8=FIX.4.4", "35=D", "58=a, b", "10=0"]);
    /// // Every key and value is a range of the page `body` holds, never a copy.
    /// assert_eq!(entries.as_slice()[2].value(), "a, b");
    /// assert_eq!(entries.as_slice()[2].value_bytes().as_bytes(), b"a, b");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn from_bytes(body: &TextBytes) -> Option<Self> {
        read_entries_at(body, MAX_ENTRY_DEPTH)
    }

    /// Every pair one run of bytes declares directly, none of them descended
    /// into.
    ///
    /// The same walk as [`from_bytes`](Self::from_bytes) stopped at one
    /// level: each entry is a range of the page and none carries a tree. For
    /// a reader that reads a nested value by its own rules - FIX reads a
    /// data field to the length it stated and scans what that holds in its
    /// own scope - the descent would be a second reading of the same bytes,
    /// paid on every value that happens to hold an `=`, and thrown away.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// use yggdryl::media::text::{TextBytes, TextEntries};
    ///
    /// let body = TextBytes::from_bytes("8=FIX.4.4|213=a=1 b=2|10=0|")?;
    /// let direct = TextEntries::from_bytes_direct(&body).expect("pairs");
    /// assert!(direct.as_slice()[1].entries().is_none());
    /// let tree = TextEntries::from_bytes(&body).expect("pairs");
    /// assert_eq!(tree.as_slice()[1].entries().map(TextEntries::len), Some(2));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn from_bytes_direct(body: &TextBytes) -> Option<Self> {
        read_entries_at(body, 1)
    }

    /// [`from_bytes_direct`](Self::from_bytes_direct), beside where the frame
    /// the scan located opens in `body` - `None` where it located none.
    ///
    /// One scan for both, because the reader that bounds a message to its
    /// frame asks both of the same bytes, and locating the frame is a walk
    /// over every pair a line holds in front of it.
    pub(crate) fn from_bytes_direct_located(body: &TextBytes) -> (Option<Self>, Option<usize>) {
        let bytes = body.as_bytes();
        let (frame_at, spans) = crate::mime_type::line::located_entry_spans(bytes);
        (collect_entries(body, spans, 1), frame_at)
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

/// One level of the walk [`TextEntries::from_bytes`] opens, with `levels`
/// left to read: this one, and `levels - 1` beneath it.
fn read_entries_at(body: &TextBytes, levels: usize) -> Option<TextEntries> {
    if levels == 0 {
        return None;
    }
    let spans = crate::mime_type::line::entry_spans(body.as_bytes());
    collect_entries(body, spans, levels)
}

/// The entries one level's spans name, each a range of `body`.
fn collect_entries(
    body: &TextBytes,
    spans: impl Iterator<Item = crate::mime_type::line::PairSpan>,
    levels: usize,
) -> Option<TextEntries> {
    let bytes = body.as_bytes();
    let mut entries: Option<TextEntries> = None;
    for span in spans {
        let (Ok(key), Ok(value)) = (
            body.slice(span.key.start, span.key.end),
            body.slice(span.value.start, span.value.end),
        ) else {
            continue;
        };
        let mut entry = TextEntry::new(key, value).with_marked(span.marked);
        // Whether the value nests is asked only where a level is left to
        // read it at: the question scans the value for an `=`.
        if levels > 1 && span.nested(bytes) {
            if let Some(nested) = read_entries_at(entry.value_bytes(), levels - 1) {
                entry.set_entries(Some(nested));
            }
        }
        // Sized on the first push from the `=` signs the bytes hold, which is
        // where the pairs are and so an upper bound on how many there are: one
        // vector per level rather than one per doubling of it.
        entries
            .get_or_insert_with(|| {
                TextEntries::with_capacity(memchr::memchr_iter(b'=', bytes).count())
            })
            .push(entry);
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
