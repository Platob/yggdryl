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
/// The body is text, and it is text by construction: a line is made from the
/// bytes the reader cut, and where those are not UTF-8 they are read once,
/// where the line is made, by the charset layer's one rule for bytes offered
/// as UTF-8 that are not - the rule behind
/// [`Charset::transcribe`](crate::Charset::transcribe) - so every reader
/// after that point reads text and none of them validates again. A body that
/// was UTF-8 - every line of every capture this crate holds - stays the range
/// of the page it was read into, so nothing is copied between the stream and
/// the line.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextLine {
    index: u64,
    url: Option<Arc<Url>>,
    timestamp: Option<i128>,
    bodytype: Option<MimeType>,
    /// Text: [`decoded`] made it so, and every door onto this field goes
    /// through it.
    body: TextBytes,
    /// Each text, by the same door.
    captures: Vec<Option<TextBytes>>,
    entries: Option<TextEntries>,
    direction: Option<&'static str>,
    dropped_byte_size: Option<u64>,
    /// How many bytes of the body, and of the captures, were decoded.
    decoded_body: u64,
    decoded_captures: u64,
}

impl TextLine {
    /// One line from its position and the bytes the reader cut for it.
    ///
    /// The bytes become text here. A body that is valid UTF-8 is kept as the
    /// range it is; one that is not is read once into a page of its own, as
    /// [`Charset::transcribe`](crate::Charset::transcribe) reads bytes offered
    /// as UTF-8 - every valid run kept, every other byte as Windows-1252
    /// through the charset layer's table - and how many bytes were read that
    /// way is what [`decoded_byte_size`] counts.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) when the
    /// decoded text is longer than a page can address in 32-bit offsets.
    ///
    /// [`decoded_byte_size`]: Self::decoded_byte_size
    pub fn from_bytes(index: u64, body: TextBytes) -> Result<Self> {
        let (body, decoded_body) = decoded(body)?;
        Ok(Self {
            index,
            url: None,
            timestamp: None,
            bodytype: None,
            body,
            captures: Vec::new(),
            entries: None,
            direction: None,
            dropped_byte_size: None,
            decoded_body,
            decoded_captures: 0,
        })
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
    ///
    /// Text, always: what [`from_bytes`](Self::from_bytes) decoded is what
    /// this answers. Readers that address the line by offset - the scanner,
    /// the codec re-slicing a data field, the Arrow builder registering a
    /// page - take the same bytes as a range through
    /// [`body_bytes`](Self::body_bytes), which is free; this validates the
    /// range on the way out, once per call, so the per-line path does not
    /// ask it.
    #[must_use]
    pub fn body(&self) -> &str {
        std::str::from_utf8(self.body.as_bytes()).expect("a line's body is text by construction")
    }

    /// The body as the range of its page, for a reader that works in offsets.
    #[must_use]
    pub const fn body_bytes(&self) -> &TextBytes {
        &self.body
    }

    /// Replace the body, decoded exactly as [`from_bytes`](Self::from_bytes)
    /// decodes one; [`decoded_byte_size`](Self::decoded_byte_size) counts
    /// the new body.
    ///
    /// # Errors
    ///
    /// Returns the refusal [`from_bytes`](Self::from_bytes) does, leaving
    /// the line unchanged.
    pub fn set_body(&mut self, body: TextBytes) -> Result<()> {
        let (body, decoded_body) = decoded(body)?;
        self.body = body;
        self.decoded_body = decoded_body;
        Ok(())
    }

    /// How many bytes of the line as read were not UTF-8 and were read as
    /// [`Charset::transcribe`](crate::Charset::transcribe) reads them.
    ///
    /// Zero for a line that was text as read. The body's count and the
    /// captures' together, because both are bytes the line held; it is the
    /// one fact the decode keeps, so a reader auditing a capture can find the
    /// lines that were repaired without decoding them again. `0` under a
    /// declared charset for a line the byte limit did not cut inside a
    /// scalar: the transport read it as declared and the line repaired
    /// nothing - whether a resource was declared is the handle's fact,
    /// `MediaType::charset`, and not a per-line count. A limit that lands
    /// inside one scalar of the decoded text leaves the stray bytes the cut
    /// made, and the line reads and counts them exactly as on an undeclared
    /// read: `Zürich` declared `windows-1252` under a limit of `2` is the
    /// body `ZÃ` with `1` decoded.
    #[must_use]
    pub const fn decoded_byte_size(&self) -> u64 {
        self.decoded_body + self.decoded_captures
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

    /// The capture at `index`, when the header declared and matched it.
    ///
    /// Text, by the same door the body came through; a column reads its
    /// capture here and parses it at its own datatype.
    #[must_use]
    pub fn capture(&self, index: usize) -> Option<&str> {
        let held = self.captures.get(index)?.as_ref()?;
        Some(std::str::from_utf8(held.as_bytes()).expect("a capture is text by construction"))
    }

    /// Replace the captures, each decoded exactly as the body is.
    ///
    /// # Errors
    ///
    /// Returns the refusal [`from_bytes`](Self::from_bytes) does, leaving
    /// the line unchanged.
    pub fn set_captures(&mut self, captures: Vec<Option<TextBytes>>) -> Result<()> {
        let mut read = Vec::with_capacity(captures.len());
        let mut decoded_captures = 0;
        for capture in captures {
            read.push(match capture {
                Some(held) => {
                    let (held, count) = decoded(held)?;
                    decoded_captures += count;
                    Some(held)
                }
                None => None,
            });
        }
        self.captures = read;
        self.decoded_captures = decoded_captures;
        Ok(())
    }

    /// Return this line carrying captures, each decoded exactly as the body
    /// is.
    ///
    /// # Errors
    ///
    /// Returns the refusal [`from_bytes`](Self::from_bytes) does.
    pub fn with_captures(mut self, captures: Vec<Option<TextBytes>>) -> Result<Self> {
        self.set_captures(captures)?;
        Ok(self)
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
        formatter.write_str(self.body())
    }
}

/// The bytes as text, and how many of them had to be decoded to be so.
///
/// Valid UTF-8 costs nothing: the range is answered as it is, and `0`. Any
/// other input is read once into a page of its own by the charset layer's one
/// rule for bytes offered as UTF-8 that are not, the rule behind
/// [`Charset::transcribe`](crate::Charset::transcribe): every valid run kept
/// as it is, and every other byte read as the character the layer's generated
/// `windows-1252` table gives it, per invalid run rather than per line. The
/// table, its rule for the five bytes that table leaves unassigned and the
/// reason the reading is per run are the layer's and are stated there once;
/// the count is what that reading answers. What is the line's is that it
/// never refuses: a run a truncation cut inside a character is invalid too,
/// and its orphan bytes read as the characters they are rather than as
/// `U+FFFD`, because a byte the wire held is a fact and a replacement
/// character is the absence of one.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) when the
/// decoded text - up to three bytes per byte decoded - is longer than a page
/// can address in 32-bit offsets.
pub(crate) fn decoded(bytes: TextBytes) -> Result<(TextBytes, u64)> {
    let held = bytes.as_bytes();
    if std::str::from_utf8(held).is_ok() {
        return Ok((bytes, 0));
    }
    // Sized here rather than left to the reading's own reservation, which is
    // the input length: a floor every stray byte overruns by one or two
    // bytes, so a `String::new()` would grow once at the first of them. The
    // line already knows it holds at least one; sixteen bytes of slack keep a
    // line with a handful at the one allocation decision 10 states.
    let mut text = String::with_capacity(held.len() + 16);
    let count = crate::charset::utf8_transcribe_into(held, &mut text) as u64;
    let page = TextBytes::from_whole_page(Arc::new(text.into_bytes()))?;
    Ok((page, count))
}

#[cfg(test)]
mod tests {
    use super::{TextLine, decoded};
    use crate::media::text::TextBytes;

    fn line(bytes: &[u8]) -> TextLine {
        TextLine::from_bytes(0, TextBytes::from_bytes(bytes).expect("a page")).expect("a line")
    }

    #[test]
    fn text_as_read_is_the_range_it_was_read_into() {
        let page =
            TextBytes::from_bytes("8=FIX.4.4|58=caf\u{e9}|10=0|".as_bytes()).expect("a page");
        let (body, count) = decoded(page.clone()).expect("text");
        assert_eq!(count, 0);
        assert!(std::sync::Arc::ptr_eq(
            body.page().unwrap(),
            page.page().unwrap()
        ));
        assert_eq!((body.start(), body.end()), (page.start(), page.end()));
    }

    #[test]
    fn one_latin_1_byte_among_utf_8_decodes_alone() {
        // `caf\xE9` beside a UTF-8 `\u{e9}`: the valid run is kept, and the
        // lone byte reads as the one character it is.
        let read = line(b"58=caf\xE9 caf\xC3\xA9|10=0|");
        assert_eq!(read.body(), "58=caf\u{e9} caf\u{e9}|10=0|");
        assert_eq!(read.decoded_byte_size(), 1);
    }

    #[test]
    fn a_wholly_windows_1252_line_reads_byte_for_byte() {
        let read = line(b"\x80 \x93quoted\x94 \x96 na\xEFve");
        assert_eq!(
            read.body(),
            "\u{20AC} \u{201C}quoted\u{201D} \u{2013} na\u{ef}ve"
        );
        assert_eq!(read.decoded_byte_size(), 5);
    }

    #[test]
    fn the_five_undefined_bytes_read_as_the_controls_of_their_number() {
        for byte in [0x81_u8, 0x8D, 0x8F, 0x90, 0x9D] {
            assert!(crate::Charset::Cp1252.scalar_of(byte).is_none());
            let read = line(&[b'a', byte, b'b']);
            assert_eq!(read.body(), format!("a{}b", byte as char));
            assert_eq!(read.decoded_byte_size(), 1);
        }
    }

    #[test]
    fn a_character_cut_in_two_reads_as_the_bytes_that_are_left() {
        // The first two bytes of a three-byte `\u{20AC}`, as a byte limit
        // would leave them: not `U+FFFD`, the two characters those bytes are.
        let read = line(b"58=\xE2\x82");
        assert_eq!(read.body(), "58=\u{e2}\u{201A}");
        assert_eq!(read.decoded_byte_size(), 2);
    }

    #[test]
    fn captures_take_the_same_decode_and_count_with_the_body() {
        let mut read = line(b"body \xE9");
        read.set_captures(vec![
            Some(TextBytes::from_bytes(b"caf\xE9").expect("a page")),
            None,
            Some(TextBytes::from_bytes(b"plain").expect("a page")),
        ])
        .expect("captures");
        assert_eq!(read.capture(0), Some("caf\u{e9}"));
        assert_eq!(read.capture(1), None);
        assert_eq!(read.capture(2), Some("plain"));
        assert_eq!(read.capture(3), None);
        assert_eq!(read.decoded_byte_size(), 2);
        read.set_body(TextBytes::from_bytes(b"clean").expect("a page"))
            .expect("a body");
        assert_eq!(read.body(), "clean");
        assert_eq!(read.decoded_byte_size(), 1, "the captures' count stays");
    }
}
