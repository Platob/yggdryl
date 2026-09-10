//! One decoded text row.

use std::fmt;
use std::sync::Arc;

use crate::{FieldPath, MimeType, Result, Url};

use super::{TextBytes, TextEntries, TextEntry};

/// One decoded text row, typed the way its columns are.
///
/// Every field already holds what its column holds, so building a batch reads
/// the struct rather than re-deriving a datatype per value. The row is not a
/// map: nothing here is looked up by name on the per-row path.
///
/// The body is a range of the page the line was read into, so a line that fits
/// one page copies no byte between the stream and the Arrow array that ends up
/// pointing at that same page.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextLine {
    index: u64,
    url: Option<Arc<Url>>,
    timestamp: Option<i128>,
    bodytype: Option<MimeType>,
    body: TextBytes,
    captures: Vec<Option<TextBytes>>,
    entries: Option<TextEntries>,
    direction: Option<&'static str>,
    dropped_byte_size: Option<u64>,
}

impl TextLine {
    /// One line holding only its position and its bytes.
    #[must_use]
    pub fn new(index: u64, body: TextBytes) -> Self {
        Self {
            index,
            url: None,
            timestamp: None,
            bodytype: None,
            body,
            captures: Vec::new(),
            entries: None,
            direction: None,
            dropped_byte_size: None,
        }
    }

    /// The physical line number within the object, from zero.
    ///
    /// The row-number column and every row-located error need it, and no other
    /// field can recover it.
    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Set the physical line number.
    pub const fn set_index(&mut self, index: u64) {
        self.index = index;
    }

    /// The object this line was read from.
    ///
    /// Shared rather than owned: every line of one handle carries the same URL,
    /// and a URL is several small strings that would otherwise be rebuilt once
    /// per row.
    #[must_use]
    pub fn url(&self) -> Option<&Url> {
        self.url.as_deref()
    }

    /// Set or clear the object this line was read from.
    pub fn set_url(&mut self, url: Option<Arc<Url>>) {
        self.url = url;
    }

    /// Return this line addressed to one object.
    #[must_use]
    pub fn with_url(mut self, url: Arc<Url>) -> Self {
        self.url = Some(url);
        self
    }

    /// When the record was written, in nanoseconds UTC.
    ///
    /// A raw count, not a scalar: the unit and the zone are fixed and the
    /// options state them before the read, so carrying a datatype beside every
    /// value would carry the same two facts once per row.
    ///
    /// Counted in 128 bits, which is wider than the column it fills. A
    /// nanosecond count in 64 bits runs out in 2262, and a capture reading a
    /// date past that, or an arithmetic step over one, has somewhere to land
    /// here rather than wrapping silently. Narrowing to the column's own width
    /// happens once, where the column is built, and a count that will not fit
    /// is refused there by name rather than truncated.
    #[must_use]
    pub const fn timestamp(&self) -> Option<i128> {
        self.timestamp
    }

    /// Set or clear when the record was written.
    pub const fn set_timestamp(&mut self, timestamp: Option<i128>) {
        self.timestamp = timestamp;
    }

    /// Return this line stamped with a write time.
    #[must_use]
    pub const fn with_timestamp(mut self, timestamp: i128) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    /// What the line was classified as.
    #[must_use]
    pub const fn bodytype(&self) -> Option<&MimeType> {
        self.bodytype.as_ref()
    }

    /// Set or clear the classification.
    pub fn set_bodytype(&mut self, bodytype: Option<MimeType>) {
        self.bodytype = bodytype;
    }

    /// Return this line classified.
    #[must_use]
    pub fn with_bodytype(mut self, bodytype: MimeType) -> Self {
        self.bodytype = Some(bodytype);
        self
    }

    /// The line, with whatever was read off its front removed.
    #[must_use]
    pub const fn body(&self) -> &TextBytes {
        &self.body
    }

    /// Replace the body.
    pub fn set_body(&mut self, body: TextBytes) {
        self.body = body;
    }

    /// Which way the line moved, when the marker was taken off the body.
    ///
    /// A field rather than an entry, because reading it removes the marker from
    /// the body: a line that carried one has a different body than a line that
    /// did not, and a fact that changes the body is not a key/value pair.
    ///
    /// The canonical spelling the direction vocabulary states, which is what
    /// the column is built from. It is a borrowed constant rather than a value
    /// because the vocabulary is closed and every line answers one of its
    /// members.
    #[must_use]
    pub const fn direction(&self) -> Option<&'static str> {
        self.direction
    }

    /// Set or clear the direction.
    pub const fn set_direction(&mut self, direction: Option<&'static str>) {
        self.direction = direction;
    }

    /// How many bytes of this record went over the retained limit.
    #[must_use]
    pub const fn dropped_byte_size(&self) -> Option<u64> {
        self.dropped_byte_size
    }

    /// Set or clear the dropped byte count.
    pub const fn set_dropped_byte_size(&mut self, size: Option<u64>) {
        self.dropped_byte_size = size;
    }

    /// The row header's named captures, in the order the expression declares
    /// them.
    ///
    /// Positional, not named: the expression fixes the order before the read,
    /// so a column reads its capture by position and never by a per-row name
    /// lookup. A capture the header declared but did not match on this line is
    /// `None`, which is the null its column holds.
    ///
    /// Separate from the entries, because they are different facts with
    /// different owners: a capture is what the caller's expression asked for,
    /// an entry is what the line itself wrote down.
    #[must_use]
    pub fn captures(&self) -> &[Option<TextBytes>] {
        &self.captures
    }

    /// Replace the captures.
    pub fn set_captures(&mut self, captures: Vec<Option<TextBytes>>) {
        self.captures = captures;
    }

    /// Return this line carrying captures.
    #[must_use]
    pub fn with_captures(mut self, captures: Vec<Option<TextBytes>>) -> Self {
        self.captures = captures;
        self
    }

    /// The key/value tree this line carries.
    ///
    /// `None` where nothing asked for one. Materializing the tree is the only
    /// thing on the decode path that allocates, so it happens when a column
    /// reads an entry or a caller asks, and not otherwise.
    #[must_use]
    pub const fn entries(&self) -> Option<&TextEntries> {
        self.entries.as_ref()
    }

    /// Borrow the tree for mutation.
    pub const fn entries_mut(&mut self) -> Option<&mut TextEntries> {
        self.entries.as_mut()
    }

    /// Set or clear the tree.
    pub fn set_entries(&mut self, entries: Option<TextEntries>) {
        self.entries = entries;
    }

    /// Return this line carrying a tree.
    #[must_use]
    pub fn with_entries(mut self, entries: TextEntries) -> Self {
        self.entries = Some(entries);
        self
    }

    /// The entry a path reaches.
    ///
    /// A miss is `None`: a path naming something this line did not carry is
    /// the ordinary case, and it becomes a null in the column that lifted it.
    #[must_use]
    pub fn get_entry_by_path(&self, path: &FieldPath) -> Option<&TextEntry> {
        self.entries.as_ref()?.get_entry_by_path(path)
    }

    /// The entry a path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidRecord`] naming the path when no entry is
    /// there.
    pub fn entry_by_path(&self, path: &FieldPath) -> Result<&TextEntry> {
        match &self.entries {
            Some(entries) => entries.entry_by_path(path),
            None => Err(crate::Error::InvalidRecord {
                path: smol_str::format_smolstr!("$.entries.{path}"),
                reason: smol_str::format_smolstr!(
                    "expected an entry at {path}, got a line carrying none"
                ),
            }),
        }
    }

    /// The entry a path reaches, for mutation.
    pub fn get_entry_by_path_mut(&mut self, path: &FieldPath) -> Option<&mut TextEntry> {
        self.entries.as_mut()?.get_entry_by_path_mut(path)
    }

    /// Set the value a path reaches, creating what is not there.
    ///
    /// # Errors
    ///
    /// Returns the refusals [`TextEntries::set_entry_by_path`] states. Failure
    /// leaves this line unchanged.
    pub fn set_entry_by_path(&mut self, path: &FieldPath, value: TextBytes) -> Result<()> {
        self.entries
            .get_or_insert_with(TextEntries::new)
            .set_entry_by_path(path, value)
    }

    /// Remove the entry a path reaches.
    pub fn remove_entry_by_path(&mut self, path: &FieldPath) -> Option<TextEntry> {
        self.entries.as_mut()?.remove_entry_by_path(path)
    }
}

impl fmt::Display for TextLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.body.as_str() {
            Some(text) => formatter.write_str(text),
            None => write!(formatter, "{} bytes", self.body.len()),
        }
    }
}
