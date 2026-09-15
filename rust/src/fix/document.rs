//! The canonical JSON documents the `fix:` namespace stores, read borrowed.
//!
//! Four `fix:` properties hold more than one text can say as a list: the
//! per-version [lineage](super::lineage), the [code set](super::codes), the
//! [directions](super::directions) a line is read under and the
//! [replacements](super::replacements) a value is restated through. All are
//! JSON, because a metadata value may hold no control character and so
//! cannot be separator-framed, and all are read on hot paths where building a
//! parse tree per ask would cost more than the lookup.
//!
//! So one convention serves them. **A document is the array of its entries**,
//! `[{...},{...}]`, and never an object wrapping one under a key that only
//! repeats the property's own name. It is rendered with each entry's keys in
//! a **declared order** rather than sorted, compactly, one text per value;
//! the reader walks the bytes and hands back slices of them, allocating
//! nothing. Declared order is what makes the walk safe *and* cheap: a reader
//! knows which key can come next, so a hand-edited document with reordered or
//! repeated keys is refused with its byte position rather than mis-read, and
//! the key a lookup keys on is put first so a scan can stop at it.
//!
//! A store dumps one of these as the JSON it is rather than as the escaped
//! text it is held as, and reads it back through the same renderer, so the
//! stored text is canonical however the file spelled it;
//! [`Kind`] is what names one document to that pair.
//!
//! A refusal is a `Copy` [`Refusal`] rather than an [`Error`], so an
//! infallible read of a malformed document costs no allocation either; only
//! a fallible door spends one, through [`Refusal::into_error`].

use std::fmt::{self, Write as _};

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Scalar, Version};

/// Why a scan stopped, held without allocating until an error is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// A byte the grammar requires was not there.
    Expected(u8),
    /// A literal the grammar requires was not there.
    ExpectedToken(&'static str),
    /// A string ran to the end of the document.
    Unclosed,
    /// A word that may hold no escape held one.
    Escaped(&'static str),
    /// A number key held something that is not decimal digits.
    NotANumber(&'static str),
    /// A number key held more than 32 bits.
    TooWide(&'static str),
    /// A version key held something the version grammar refuses.
    NotAVersion(&'static str),
    /// A key repeated, or came before one already read.
    KeyOrder,
    /// A key this document does not declare.
    UnknownKey,
    /// A key the document must state was absent.
    MissingKey(&'static str),
    /// Two keys that exclude each other were both stated.
    Together(&'static str, &'static str),
    /// A list held fewer elements than the grammar requires of it.
    Short(&'static str, usize),
    /// Bytes stood after the document ended.
    Trailing,
}

/// What every scanning step answers.
pub(super) type Scan<T> = std::result::Result<T, Refusal>;

impl Refusal {
    /// Spells this refusal against the document and byte it stopped on.
    ///
    /// A refusal naming a key reads that key back out of the document rather
    /// than carrying it through the scan, which is what keeps the reason
    /// `Copy` and the read allocation-free until an error is actually built.
    pub(super) fn into_error(self, target: &'static str, document: &str, position: usize) -> Error {
        let key = || {
            document
                .get(position..)
                .and_then(|rest| rest.strip_prefix('"'))
                .and_then(|rest| rest.split_once('"'))
                .map_or("", |(key, _)| key)
        };
        let reason = match self {
            Self::Expected(byte) => format_smolstr!("expected {:?}", char::from(byte)),
            Self::ExpectedToken(token) => format_smolstr!("expected {token:?}"),
            Self::Unclosed => SmolStr::new_static("expected a closing quote"),
            Self::Escaped(what) => format_smolstr!("expected {what:?} to hold no escape"),
            Self::NotANumber(what) => format_smolstr!("expected {what:?} to hold a decimal number"),
            Self::TooWide(what) => format_smolstr!("expected {what:?} to fit in 32 bits"),
            Self::NotAVersion(what) => format_smolstr!("expected {what:?} to hold a version"),
            Self::KeyOrder => format_smolstr!(
                "expected keys in their declared order, got {:?} out of order",
                key()
            ),
            Self::UnknownKey => format_smolstr!("unknown key {:?}", key()),
            Self::MissingKey(what) => format_smolstr!("expected every entry to state {what:?}"),
            Self::Together(left, right) => {
                format_smolstr!("expected {left:?} and {right:?} never together")
            }
            Self::Short(what, least) => {
                format_smolstr!("expected {what:?} to hold at least {least} elements")
            }
            Self::Trailing => SmolStr::new_static("expected the document to end"),
        };
        Error::Parse {
            target,
            position,
            reason,
        }
    }
}

/// A borrowed walk over one canonical document.
///
/// Every read hands back a slice of the document, so a walk allocates
/// nothing whatever it reads.
#[derive(Clone, Debug)]
pub(super) struct Cursor<'doc> {
    document: &'doc str,
    position: usize,
}

impl<'doc> Cursor<'doc> {
    /// Opens a walk at the first byte.
    pub(super) const fn new(document: &'doc str) -> Self {
        Self {
            document,
            position: 0,
        }
    }

    /// The whole document this cursor walks.
    pub(super) const fn document(&self) -> &'doc str {
        self.document
    }

    /// The byte the cursor sits on.
    pub(super) const fn position(&self) -> usize {
        self.position
    }

    /// Moves the cursor to where a search found a record.
    pub(super) const fn seek(&mut self, position: usize) {
        self.position = position;
    }

    /// Whether every byte has been read.
    pub(super) const fn is_done(&self) -> bool {
        self.position >= self.document.len()
    }

    /// The byte the cursor sits on, without moving it.
    #[inline]
    pub(super) fn peek(&self) -> Option<u8> {
        self.document.as_bytes().get(self.position).copied()
    }

    /// Steps over one expected byte.
    #[inline]
    pub(super) fn expect(&mut self, byte: u8) -> Scan<()> {
        if self.peek() == Some(byte) {
            self.position += 1;
            return Ok(());
        }
        Err(Refusal::Expected(byte))
    }

    /// Steps over one expected literal.
    #[inline]
    pub(super) fn expect_all(&mut self, token: &'static str) -> Scan<()> {
        if self.document[self.position..].starts_with(token) {
            self.position += token.len();
            return Ok(());
        }
        Err(Refusal::ExpectedToken(token))
    }

    /// Reads one JSON string body, leaving its escapes in place.
    #[inline]
    pub(super) fn read_string(&mut self) -> Scan<&'doc str> {
        self.expect(b'"')?;
        let start = self.position;
        let bytes = self.document.as_bytes();
        while let Some(byte) = bytes.get(self.position).copied() {
            match byte {
                b'"' => {
                    let body = &self.document[start..self.position];
                    self.position += 1;
                    return Ok(body);
                }
                // A backslash escapes the byte after it, including a quote, so
                // the scan steps over the pair to find the real end.
                b'\\' => self.position += 2,
                _ => self.position += 1,
            }
        }
        Err(Refusal::Unclosed)
    }

    /// Reads one JSON string body that may hold no escape.
    ///
    /// A FIX name, a datatype name, a code value and a version are ASCII
    /// words, so an escape in one is a hand edit rather than something the
    /// writer produced.
    #[inline]
    pub(super) fn read_word(&mut self, key: &'static str) -> Scan<&'doc str> {
        let start = self.position;
        let word = self.read_string()?;
        if word.as_bytes().contains(&b'\\') {
            self.position = start;
            return Err(Refusal::Escaped(key));
        }
        Ok(word)
    }

    /// Reads one nested object, handing back its whole text.
    ///
    /// A datatype is the one value these documents hold that is a document
    /// itself, because it is stored exactly as the field's own datatype is
    /// and a parameterized one carries its parameters as keys. The braces
    /// are balanced by scanning, strings skipped whole so a `{` inside one is
    /// not counted, and the slice handed back is the input's own bytes - so
    /// the crate's JSON reader parses it later without this scan allocating.
    pub(super) fn read_document(&mut self, key: &'static str) -> Scan<&'doc str> {
        let start = self.position;
        self.expect(b'{')?;
        let mut depth = 1_usize;
        while depth > 0 {
            match self.peek() {
                Some(b'"') => {
                    self.read_string()?;
                    continue;
                }
                Some(b'{') => depth += 1,
                Some(b'}') => depth -= 1,
                Some(_) => {}
                None => {
                    self.position = start;
                    return Err(Refusal::Unclosed);
                }
            }
            self.position += 1;
        }
        let body = &self.document[start..self.position];
        if body.len() < 2 {
            self.position = start;
            return Err(Refusal::NotANumber(key));
        }
        Ok(body)
    }

    /// Reads one nested array, handing back the text between its brackets.
    ///
    /// A list of fills is the one array these documents hold whose elements
    /// are objects, and one whose objects hold lists of their own, so the
    /// brackets and braces are balanced together by scanning and strings are
    /// skipped whole exactly as [`Self::read_document`] skips them. The
    /// elements stay unread: the caller's own grammar reads them out of the
    /// slice handed back, and a caller that only wants past them pays the
    /// skip.
    pub(super) fn read_list(&mut self) -> Scan<&'doc str> {
        let start = self.position;
        self.expect(b'[')?;
        let mut depth = 1_usize;
        while depth > 0 {
            match self.peek() {
                Some(b'"') => {
                    self.read_string()?;
                    continue;
                }
                Some(b'[' | b'{') => depth += 1,
                Some(b']' | b'}') => depth -= 1,
                Some(_) => {}
                None => {
                    self.position = start;
                    return Err(Refusal::Unclosed);
                }
            }
            self.position += 1;
        }
        Ok(&self.document[start + 1..self.position - 1])
    }

    /// Reads the body of an array of tags, as one slice.
    ///
    /// Every element is read as a decimal tag so a hand-edited array is
    /// refused here rather than mis-read by [`Numbers`], which walks the
    /// slice handed back without checking it again.
    pub(super) fn read_numbers(&mut self, key: &'static str) -> Scan<&'doc str> {
        self.expect(b'[')?;
        let start = self.position;
        loop {
            if self.peek() == Some(b']') {
                let body = &self.document[start..self.position];
                self.position += 1;
                return Ok(body);
            }
            let at = self.position;
            if i32::try_from(self.read_number(key)?).is_err() {
                self.position = at;
                return Err(Refusal::TooWide(key));
            }
            if self.peek() == Some(b',') {
                self.position += 1;
            }
        }
    }

    /// Reads one non-negative decimal number.
    #[inline]
    pub(super) fn read_number(&mut self, key: &'static str) -> Scan<u32> {
        let start = self.position;
        let bytes = self.document.as_bytes();
        while bytes.get(self.position).is_some_and(u8::is_ascii_digit) {
            self.position += 1;
        }
        if self.position == start {
            return Err(Refusal::NotANumber(key));
        }
        self.document[start..self.position].parse().map_err(|_| {
            self.position = start;
            Refusal::TooWide(key)
        })
    }

    /// Reads one canonically spelled version.
    #[inline]
    pub(super) fn read_version(&mut self, key: &'static str) -> Scan<Version> {
        let start = self.position;
        let text = self.read_word(key)?;
        text.parse().map_err(|_| {
            self.position = start;
            Refusal::NotAVersion(key)
        })
    }

    /// Reads `true`, the only spelling a flag key takes.
    ///
    /// A false flag is absent rather than written, so one declaration has one
    /// stored form.
    #[inline]
    pub(super) fn read_flag(&mut self) -> Scan<bool> {
        self.expect_all("true")?;
        Ok(true)
    }

    /// Reads one key, holding it to the document's declared order.
    ///
    /// `keys` is the declared order and `next` the first index still
    /// admissible; a key at or before it is a repeat or a reordering, and a
    /// key the listing does not hold is a hand edit. Both are refused rather
    /// than read positionally.
    pub(super) fn read_key(&mut self, keys: &[&'static str], next: &mut usize) -> Scan<usize> {
        let at = self.position;
        let key = self.read_string()?;
        // Only the keys still admissible are candidates, so a record of four
        // keys costs four short comparisons rather than four passes over the
        // whole listing - which is the difference between a walk that reads a
        // record and one that searches it.
        let found = keys
            .iter()
            .enumerate()
            .skip(*next)
            .find(|(_, declared)| declared.len() == key.len() && **declared == key);
        let Some((index, _)) = found else {
            self.position = at;
            return Err(if keys.contains(&key) {
                Refusal::KeyOrder
            } else {
                Refusal::UnknownKey
            });
        };
        *next = index + 1;
        self.expect(b':')?;
        Ok(index)
    }

    /// Opens the document, which is the array of entries itself.
    ///
    /// Answers whether that array holds anything, so a caller stops without a
    /// second probe, and steps over the closing bracket when it does not.
    pub(super) fn open_array(&mut self) -> Scan<bool> {
        self.expect(b'[')?;
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(false);
        }
        Ok(true)
    }

    /// Reads the body of an array of words, as one slice.
    ///
    /// The elements stay unread: a caller that wants them walks them with
    /// [`Words`], and one that does not pays only the skip. Words are read
    /// rather than skipped so a hand-edited array is refused here too.
    pub(super) fn read_words(&mut self, key: &'static str) -> Scan<&'doc str> {
        self.expect(b'[')?;
        let start = self.position;
        loop {
            if self.peek() == Some(b']') {
                let body = &self.document[start..self.position];
                self.position += 1;
                return Ok(body);
            }
            self.read_word(key)?;
            if self.peek() == Some(b',') {
                self.position += 1;
            }
        }
    }

    /// Steps to the next entry of the document, answering whether one
    /// follows.
    ///
    /// The prologue every reader shares: the first step opens the array the
    /// document is, each later one takes the separator between two entries,
    /// and either way the end of that array has to be the end of the
    /// document. `started` is the reader's own flag, so a reader stays a
    /// plain iterator over a borrowed cursor.
    pub(super) fn next_entry(&mut self, started: &mut bool) -> Scan<bool> {
        let more = if *started {
            self.next_element()?
        } else {
            *started = true;
            self.open_array()?
        };
        if more {
            return Ok(true);
        }
        if !self.is_done() {
            return Err(Refusal::Trailing);
        }
        Ok(false)
    }

    /// Steps to the next array element, answering whether one follows.
    pub(super) fn next_element(&mut self) -> Scan<bool> {
        if self.peek() == Some(b',') {
            self.position += 1;
            return Ok(true);
        }
        self.expect(b']')?;
        Ok(false)
    }

    /// Steps to the next property of an object, answering whether one follows.
    ///
    /// Closes the object when none does, so a caller's read loop states the
    /// separator and the terminator once between them.
    #[inline]
    pub(super) fn next_property(&mut self) -> Scan<bool> {
        if self.peek() == Some(b',') {
            self.position += 1;
            return Ok(true);
        }
        self.expect(b'}')?;
        Ok(false)
    }
}

/// Decodes one stored text body, which a caller asks for and the hot path
/// never does.
///
/// The body is held escaped exactly as the document holds it, so decoding is
/// re-quoting it and handing it to the crate's own JSON codec rather than to
/// a second unescaper.
///
/// # Errors
///
/// Returns [`Error::Parse`] when the stored body is not a legal string body,
/// which [`Writer`] never produces.
pub(super) fn decode_text(
    target: &'static str,
    key: &'static str,
    body: Option<&str>,
) -> Result<Option<String>> {
    let Some(body) = body else {
        return Ok(None);
    };
    let mut quoted = String::with_capacity(body.len() + 2);
    quoted.push('"');
    quoted.push_str(body);
    quoted.push('"');
    let decoded = crate::text::json::from_utf8(&quoted)?;
    decoded
        .as_str()
        .map(ToOwned::to_owned)
        .map(Some)
        .ok_or_else(|| Error::Parse {
            target,
            position: 0,
            reason: format_smolstr!("expected {key:?} to hold text"),
        })
}

/// The words one array-valued key holds, borrowed.
///
/// The reader that produced the body already held every word to the grammar,
/// so nothing here re-validates and nothing allocates.
#[derive(Clone, Debug)]
pub struct Words<'doc>(&'doc str);

impl<'doc> Words<'doc> {
    /// Walks one array body a reader answered.
    pub(super) const fn over(body: &'doc str) -> Self {
        Self(body)
    }
}

impl<'doc> Iterator for Words<'doc> {
    type Item = &'doc str;

    fn next(&mut self) -> Option<Self::Item> {
        let rest = self.0.strip_prefix(',').unwrap_or(self.0);
        let rest = rest.strip_prefix('"')?;
        let (word, tail) = rest.split_once('"')?;
        self.0 = tail;
        Some(word)
    }
}

impl std::iter::FusedIterator for Words<'_> {}

/// The tags one array-valued key holds, borrowed.
///
/// The reader that produced the body already held every element to the tag
/// grammar, so nothing here re-validates and nothing allocates.
#[derive(Clone, Debug)]
pub struct Numbers<'doc>(&'doc str);

impl<'doc> Numbers<'doc> {
    /// Walks one array body a reader answered.
    pub(super) const fn over(body: &'doc str) -> Self {
        Self(body)
    }
}

impl Iterator for Numbers<'_> {
    type Item = i32;

    fn next(&mut self) -> Option<Self::Item> {
        let rest = self.0.strip_prefix(',').unwrap_or(self.0);
        let end = rest
            .bytes()
            .position(|byte| !byte.is_ascii_digit())
            .unwrap_or(rest.len());
        let (digits, tail) = rest.split_at(end);
        self.0 = tail;
        digits.parse().ok()
    }
}

impl std::iter::FusedIterator for Numbers<'_> {}

/// Renders one canonical document, keys in their declared order.
///
/// Values are escaped through the crate's own JSON codec rather than by a
/// second escaper, so what this writes is exactly what that codec reads.
#[derive(Debug, Default)]
pub(super) struct Writer {
    text: String,
    empty: bool,
}

impl Writer {
    /// Opens a document, which is the array of its entries.
    pub(super) fn open_array() -> Self {
        Self {
            text: String::from("["),
            empty: true,
        }
    }

    /// Opens one element of that array.
    pub(super) fn open_element(&mut self) {
        if !self.empty {
            self.text.push(',');
        }
        self.empty = false;
        self.text.push('{');
    }

    /// Closes one element.
    pub(super) fn close_element(&mut self) {
        self.text.push('}');
    }

    /// Opens one array-valued key whose elements are objects.
    ///
    /// One flag serves every nesting depth because elements nest strictly:
    /// the list opened here is the only one taking elements until it closes,
    /// and closing it leaves the enclosing array mid-way, where its next
    /// element needs a separator.
    pub(super) fn open_list(&mut self, first: bool, key: &str) {
        self.key(first, key);
        self.text.push('[');
        self.empty = true;
    }

    /// Closes the list [`Self::open_list`] opened.
    pub(super) fn close_list(&mut self) {
        self.text.push(']');
        self.empty = false;
    }

    /// Writes one text-valued key.
    ///
    /// # Errors
    ///
    /// Propagates the JSON codec's refusal, which text cannot provoke.
    pub(super) fn text(&mut self, first: bool, key: &str, value: &str) -> Result<()> {
        self.key(first, key);
        self.string(value)
    }

    /// Writes one string, escaping it only when it needs escaping.
    ///
    /// Almost every value these documents hold is a FIX name, a code value or
    /// a version, and none of those can carry a quote, a backslash or a
    /// control character. Those are quoted directly; anything else goes
    /// through the crate's own JSON codec, so there is still exactly one
    /// escaper and it is the one that reads these documents back.
    fn string(&mut self, value: &str) -> Result<()> {
        if value
            .bytes()
            .all(|byte| byte >= 0x20 && !matches!(byte, b'"' | b'\\'))
        {
            self.text.push('"');
            self.text.push_str(value);
            self.text.push('"');
            return Ok(());
        }
        self.text
            .push_str(&crate::text::json::into_utf8(&Scalar::from(value))?);
        Ok(())
    }

    /// Writes one key whose value is an already-rendered document.
    ///
    /// The text comes from the crate's own JSON writer, so it is spliced
    /// rather than re-escaped: escaping a document would make it a string.
    pub(super) fn document(&mut self, first: bool, key: &str, value: &str) {
        self.key(first, key);
        self.text.push_str(value);
    }

    /// Writes one number-valued key.
    pub(super) fn number(&mut self, first: bool, key: &str, value: impl fmt::Display) {
        self.key(first, key);
        // Writing into a `String` cannot fail.
        let _ = write!(self.text, "{value}");
    }

    /// Writes one array-of-tags key.
    pub(super) fn numbers(
        &mut self,
        first: bool,
        key: &str,
        values: impl IntoIterator<Item = i32>,
    ) {
        self.key(first, key);
        self.text.push('[');
        for (index, value) in values.into_iter().enumerate() {
            if index > 0 {
                self.text.push(',');
            }
            // Writing into a `String` cannot fail.
            let _ = write!(self.text, "{value}");
        }
        self.text.push(']');
    }

    /// Writes one flag key, which exists only when true.
    pub(super) fn flag(&mut self, first: bool, key: &str) {
        self.key(first, key);
        self.text.push_str("true");
    }

    /// Writes one array-of-words key.
    ///
    /// # Errors
    ///
    /// Propagates the JSON codec's refusal, which text cannot provoke.
    pub(super) fn words<'a>(
        &mut self,
        first: bool,
        key: &str,
        values: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        self.key(first, key);
        self.text.push('[');
        for (index, value) in values.into_iter().enumerate() {
            if index > 0 {
                self.text.push(',');
            }
            self.string(value)?;
        }
        self.text.push(']');
        Ok(())
    }

    /// Finishes the document, closing the array it is.
    pub(super) fn finish(mut self) -> String {
        self.text.push(']');
        self.text
    }

    fn key(&mut self, first: bool, key: &str) {
        if !first {
            self.text.push(',');
        }
        self.text.push('"');
        self.text.push_str(key);
        self.text.push_str("\":");
    }
}

// ---------------------------------------------------------------------------
// One document, two representations: the canonical text a field's metadata
// holds, and the JSON a store dumps it as.
//
// A metadata value is inert text, so a code set stored beside a field is one
// escaped line however the file around it is indented. That is unreadable in
// a store a person edits, and the text *is* JSON - so a store writes it as
// the JSON it is and reads it back through here.
//
// Reading it back cannot trust the file's key order: a JSON object decodes
// into a sorted `Scalar::Record`, while the stored text is ordered by the
// grammar each reader walks. So both directions go through the one table
// below, which is the same declared order the writers write, and a hand-
// edited document is restated canonically rather than stored as it was
// spelled.
// ---------------------------------------------------------------------------

/// What one key of an entry holds, where it is not an ordinary leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// A text, a number, a flag, or an array of those: carried as it arrived,
    /// because a JSON array keeps the order it was written in.
    Leaf,
    /// One datatype document, restated through the datatype's own writer, so
    /// its keys come back in the order that writer states them rather than
    /// in the order a JSON reader sorted them into.
    Datatype,
    /// A list of fills, whose members are a list of fills again.
    Fills,
}

/// What each key of a code holds. The order is [`super::codes::KEYS`]'s, which
/// stays the one owner of it; this says only which keys are not leaves.
const CODE_PARTS: [Part; 9] = [Part::Leaf; 9];

/// The same for a lineage entry, whose `type` is a datatype document.
const LINEAGE_PARTS: [Part; 7] = [
    Part::Leaf,
    Part::Leaf,
    Part::Leaf,
    Part::Datatype,
    Part::Leaf,
    Part::Leaf,
    Part::Leaf,
];

/// The same for a direction.
const DIRECTION_PARTS: [Part; 2] = [Part::Leaf; 2];

/// The same for a replacement, whose `fills` is a list of fills.
const REPLACEMENT_PARTS: [Part; 7] = [
    Part::Leaf,
    Part::Leaf,
    Part::Leaf,
    Part::Leaf,
    Part::Leaf,
    Part::Fills,
    Part::Leaf,
];

/// The same for one fill, whose `members` is a list of fills again.
const FILL_PARTS: [Part; 6] = [
    Part::Leaf,
    Part::Leaf,
    Part::Leaf,
    Part::Leaf,
    Part::Leaf,
    Part::Fills,
];

// A parts table says what each of a reader's own keys holds, so the two are
// one table read side by side and a key added to a reader without a part is a
// build failure rather than a key this quietly stops carrying.
const _: () = assert!(CODE_PARTS.len() == super::codes::KEYS.len());
const _: () = assert!(LINEAGE_PARTS.len() == super::lineage::KEYS.len());
const _: () = assert!(DIRECTION_PARTS.len() == super::directions::KEYS.len());
const _: () = assert!(REPLACEMENT_PARTS.len() == super::replacements::KEYS.len());
const _: () = assert!(FILL_PARTS.len() == super::replacements::FILL_KEYS.len());

/// One `fix:` property whose stored value is a canonical document.
///
/// Four properties hold one, and this is what names one of them to the pair
/// a store crosses: [`Self::into_value`] writes the document as the JSON it
/// is, [`Self::from_value`] reads that JSON back as the canonical text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    /// [`super::codes`], under `fix:codes`.
    Codes,
    /// [`super::lineage`], under `fix:lineage`.
    Lineage,
    /// [`super::directions`], under `fix:directions`.
    Directions,
    /// [`super::replacements`], under `fix:replacements`.
    Replacements,
}

impl Kind {
    /// Every property a store crosses this way.
    pub(super) const ALL: [Self; 4] = [
        Self::Codes,
        Self::Lineage,
        Self::Directions,
        Self::Replacements,
    ];

    /// The metadata key this document is stored under.
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Codes => "fix:codes",
            Self::Lineage => "fix:lineage",
            Self::Directions => "fix:directions",
            Self::Replacements => "fix:replacements",
        }
    }

    /// The one this metadata key names, or nothing for an ordinary property.
    pub(super) fn of_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// What this document calls itself in a refusal, as its reader does.
    const fn target(self) -> &'static str {
        match self {
            Self::Codes => "fix codes",
            Self::Lineage => "fix lineage",
            Self::Directions => "fix directions",
            Self::Replacements => "fix replacements",
        }
    }

    /// The keys one entry states, in the order its reader walks them, beside
    /// what each of them holds.
    const fn entry(self) -> (&'static [&'static str], &'static [Part]) {
        match self {
            Self::Codes => (&super::codes::KEYS, &CODE_PARTS),
            Self::Lineage => (&super::lineage::KEYS, &LINEAGE_PARTS),
            Self::Directions => (&super::directions::KEYS, &DIRECTION_PARTS),
            Self::Replacements => (&super::replacements::KEYS, &REPLACEMENT_PARTS),
        }
    }

    /// The JSON value this document's canonical text already is.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the text is not this document: not JSON
    /// at all, not an array of objects, or an entry stating a key this
    /// document does not declare.
    pub(super) fn value_of(self, document: &str) -> Result<Scalar> {
        let parsed = crate::from_json_scalar(document)
            .map_err(|error| self.refused(format_args!("{error}")))?;
        self.ordered(&parsed)
    }

    /// The canonical text one dumped value restates.
    ///
    /// The entries keep the order the file gave them - a code set is ordered
    /// by wire value and a lineage by version, and both are the writer's
    /// business rather than this one's - while each entry's own keys are put
    /// back into the order the grammar declares, whatever order the file
    /// spelled them in.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the value is not an array of objects or
    /// an entry states a key this document does not declare, and whatever
    /// the datatype reader returns for a `type` that is not one.
    pub(super) fn text_of(self, value: &Scalar) -> Result<String> {
        crate::into_json_scalar(&self.ordered(value)?)
    }

    /// The document with every entry's keys in the order this kind states
    /// them: the one order its reader walks and its writer writes.
    fn ordered(self, value: &Scalar) -> Result<Scalar> {
        let entries = value.as_sequence().ok_or_else(|| {
            self.refused(crate::text::expected_got(
                "an array of entries",
                value.kind(),
            ))
        })?;
        let (keys, parts) = self.entry();
        let ordered = entries
            .iter()
            .map(|entry| self.order_entry(keys, parts, entry))
            .collect::<Result<Vec<_>>>()?;
        Ok(Scalar::from_sequence(ordered))
    }

    /// One entry, its stated keys in the declared order and nothing else.
    fn order_entry(
        self,
        keys: &'static [&'static str],
        parts: &'static [Part],
        entry: &Scalar,
    ) -> Result<Scalar> {
        if entry.as_record().is_none() && entry.as_mapping().is_none() {
            return Err(self.refused(crate::text::expected_got("an entry object", entry.kind())));
        }
        let mut held: Vec<(Scalar, Scalar)> = Vec::with_capacity(keys.len());
        for (key, part) in keys.iter().zip(parts) {
            let Some(value) = entry.get_key_str(key) else {
                continue;
            };
            // A flag exists only when it is true, and a key a file spelled
            // null states nothing: both leave the entry rather than being
            // written back as something the reader would refuse.
            if matches!(value, Scalar::Null) || value.as_bool() == Some(false) {
                continue;
            }
            let value = match part {
                Part::Leaf => value.clone(),
                Part::Datatype => crate::DataType::from_value(value.clone())?.into_value(),
                Part::Fills => {
                    let fills = value.as_sequence().ok_or_else(|| {
                        self.refused(crate::text::expected_got(
                            format_args!("{key:?} to hold a list of fills"),
                            value.kind(),
                        ))
                    })?;
                    Scalar::from_sequence(
                        fills
                            .iter()
                            .map(|fill| {
                                self.order_entry(&super::replacements::FILL_KEYS, &FILL_PARTS, fill)
                            })
                            .collect::<Result<Vec<_>>>()?,
                    )
                }
            };
            held.push((Scalar::from(*key), value));
        }
        if let Some(unknown) = entry.keys().into_iter().find(|key| !keys.contains(key)) {
            return Err(self.refused(format_args!("unknown key {unknown:?}")));
        }
        // A mapping rather than a record, because a record sorts its keys and
        // the order this just settled is the whole point.
        Scalar::from_mapping(held)
    }

    /// The refusal a store raises, naming the property a caller has to fix.
    fn invalid(self, reason: impl fmt::Display) -> Error {
        Error::InvalidMetadataValue {
            key: self.key().into(),
            reason: format_smolstr!("{reason}"),
        }
    }

    /// This document's own refusal, so a reader names the document a caller
    /// has to fix rather than the JSON beneath it.
    fn refused(self, reason: impl fmt::Display) -> Error {
        Error::Parse {
            target: self.target(),
            position: 0,
            reason: format_smolstr!("{reason}"),
        }
    }
}

/// One field as a FIX store spells it: a native `Field` document whose `fix:`
/// document properties are the JSON they are rather than one escaped line.
///
/// The shape [`FixRegistry::write_into`](super::FixRegistry::write_into) and
/// [`FixRegistry::into_json`](super::FixRegistry::into_json) write and
/// [`from_fix_document`] reads, exposed on its own so a caller editing one
/// document out of a store - what `ygg fix read --json` prints and
/// `ygg fix ... --input` takes - writes the same shape the store does. It is
/// the FIX spelling of [`Field::into_value`](crate::Field::into_value), and
/// the only difference between
/// them is those four properties.
///
/// ```
/// use yggdryl::{DataType, FixCode, Scalar, fix};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut side = DataType::utf8().nullable_field("Side");
/// side.as_fix_mut().set_tag(54)?;
/// side.as_fix_mut().set_codes(&[FixCode::new("Buy", "1")])?;
///
/// let document = fix::into_fix_document(side.clone())?;
/// let codes = document
///     .get_key_str("metadata")
///     .and_then(|metadata| metadata.get_key_str("fix:codes"))
///     .expect("the code set");
/// // The JSON it is, not the text it is stored as.
/// assert_eq!(codes.len(), 1);
/// assert_eq!(
///     codes.get(0).and_then(|code| code.get_key_str("name")).and_then(Scalar::as_str),
///     Some("Buy"),
/// );
/// assert_eq!(fix::from_fix_document(document)?, side);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`Error::InvalidMetadataValue`] naming the property when a field
/// holds text under one of the four keys that is not the document that key
/// declares.
pub fn into_fix_document(field: crate::Field) -> Result<Scalar> {
    dump(field.into_value())
}

/// The field one such document restates.
///
/// The inverse of [`into_fix_document`], and the door a store reads through:
/// each document property is restated as the canonical text a field's
/// metadata holds, in the order the grammar declares, so a file may spell an
/// entry's keys however it likes and the field still holds one text.
///
/// # Errors
///
/// Returns [`Error::InvalidMetadataValue`] naming the property when the
/// document spells one of the four keys as anything but the array of entries
/// it is, and what [`Field::from_value`](crate::Field::from_value) returns
/// for a document that is not a field.
pub fn from_fix_document(document: Scalar) -> Result<crate::Field> {
    crate::Field::from_value(load(document)?)
}

/// One native Field document as a store writes it: every `fix:` document
/// property expanded into the JSON it is.
///
/// Applied to the whole document rather than to one field, because a named
/// definition carries its members' metadata inside its own datatype.
///
/// # Errors
///
/// Returns [`Error::InvalidMetadataValue`] naming the property when a field
/// holds text under one of these keys that is not the document the key
/// declares. There is no escaped fallback: a store writes one shape, and a
/// value nothing can read is named where it is found rather than copied out
/// for a reader to refuse later.
pub(super) fn dump(value: Scalar) -> Result<Scalar> {
    cross(value, &|kind, held| {
        let text = held
            .as_str()
            .ok_or_else(|| kind.invalid(crate::text::expected_got("its text", held.kind())))?;
        kind.value_of(text)
            .map_err(|error| kind.invalid(format_args!("{error}")))
    })
}

/// The inverse: one Field document a store read, every expanded property
/// restated as the canonical text a field's metadata holds.
///
/// The entries keep the order the file gave them and each entry's own keys
/// are put back into the order the grammar declares, so a person may edit the
/// file and spell an entry's keys however they like.
///
/// # Errors
///
/// Returns [`Error::InvalidMetadataValue`] naming the property when the file
/// spells one of these keys as anything but the array of entries it is - the
/// escaped text an older writer wrote included, because a store reads one
/// shape - or when an entry states a key the document does not declare.
pub(super) fn load(value: Scalar) -> Result<Scalar> {
    cross(value, &|kind, held| {
        if let Some(text) = held.as_str() {
            return Err(kind.invalid(crate::text::expected_got(
                "the document itself",
                format_args!("the text {:?}", crate::text::elide_to(text, 32)),
            )));
        }
        kind.text_of(held)
            .map(Scalar::from)
            .map_err(|error| kind.invalid(format_args!("{error}")))
    })
}

/// Rewrite every `metadata` map the document holds, at any depth.
fn cross(value: Scalar, across: &dyn Fn(Kind, &Scalar) -> Result<Scalar>) -> Result<Scalar> {
    if let Some(values) = value.as_sequence() {
        return values
            .iter()
            .map(|held| cross(held.clone(), across))
            .collect::<Result<Vec<_>>>()
            .map(Scalar::from_sequence);
    }
    let Some(keys) = named(&value) else {
        return Ok(value);
    };
    let mut held: Vec<(Scalar, Scalar)> = Vec::with_capacity(keys.len());
    for key in keys {
        let Some(child) = value.get_key_str(&key) else {
            continue;
        };
        let child = if key == "metadata" {
            properties(child, across)?
        } else {
            cross(child.clone(), across)?
        };
        held.push((Scalar::from(key), child));
    }
    Scalar::from_mapping(held)
}

/// One `metadata` map with the four document properties crossed over.
fn properties(value: &Scalar, across: &dyn Fn(Kind, &Scalar) -> Result<Scalar>) -> Result<Scalar> {
    let Some(keys) = named(value) else {
        return Ok(value.clone());
    };
    let mut held: Vec<(Scalar, Scalar)> = Vec::with_capacity(keys.len());
    for key in keys {
        let Some(child) = value.get_key_str(&key) else {
            continue;
        };
        let child = match Kind::of_key(&key) {
            Some(kind) => across(kind, child)?,
            None => child.clone(),
        };
        held.push((Scalar::from(key), child));
    }
    Scalar::from_mapping(held)
}

/// The keys of a named value, in the order it answers them, or nothing where
/// it is not one.
fn named(value: &Scalar) -> Option<Vec<String>> {
    if value.as_record().is_none() && value.as_mapping().is_none() {
        return None;
    }
    Some(value.keys().into_iter().map(str::to_owned).collect())
}
