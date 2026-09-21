//! The canonical JSON documents the `FIX:` namespace stores, read borrowed.
//!
//! Five `FIX:` properties hold more than one text can say. Three are
//! documents of entries: the [code set](super::codes), the
//! [directions](super::directions) a line is read under and the
//! [replacements](super::replacements) a value is restated through. Two are
//! bare lists: the alternate [names](super::FixField::names) a field answers
//! to and the alternate [tags](super::FixField::tags) it holds. All are JSON,
//! because a metadata value may hold no control character and so cannot be
//! separator-framed, and all are read on hot paths where building a parse
//! tree per ask would cost more than the lookup.
//!
//! So one convention serves them. **A document is the array of its entries**,
//! `[{...},{...}]`, and never an object wrapping one under a key that only
//! repeats the property's own name. It is rendered with each entry's keys in
//! a **declared order** rather than sorted, compactly, one text per value;
//! the reader walks the bytes and hands back slices of them, allocating
//! nothing. Declared order is what makes the walk safe *and* cheap: a reader
//! knows which key can come next, so a hand-edited document with reordered or
//! repeated keys is refused with its byte position rather than mis-read, and
//! the key a lookup keys on is put first so a scan can stop at it. **A list
//! is the array of its elements**, `["a","b"]` or `[1,2]`, rendered and read
//! by the same primitives an entry's own array-valued keys are.
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

use crate::{Error, Result, Scalar};

/// Why a scan stopped, held without allocating until an error is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// A byte the grammar requires was not there.
    Expected(u8),
    /// A string ran to the end of the document.
    Unclosed,
    /// A word key held something that is not a word: empty, or holding a
    /// quote, a backslash or a control character.
    NotAWord(&'static str),
    /// A number key held something that is not a JSON number of decimal
    /// digits.
    NotANumber(&'static str),
    /// A number key held more than 32 bits.
    TooWide(&'static str),
    /// A key repeated, or came before one already read.
    KeyOrder,
    /// A key this document does not declare.
    UnknownKey,
    /// A key the document must state was absent.
    MissingKey(&'static str),
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
            Self::Unclosed => SmolStr::new_static("expected a closing quote"),
            Self::NotAWord(what) => format_smolstr!(
                "expected {what:?} to hold a word: non-empty, without a quote, a backslash or a control character"
            ),
            Self::NotANumber(what) => format_smolstr!("expected {what:?} to hold a decimal number"),
            Self::TooWide(what) => format_smolstr!("expected {what:?} to fit in 32 bits"),
            Self::KeyOrder => format_smolstr!(
                "expected keys in their declared order, got {:?} out of order",
                key()
            ),
            Self::UnknownKey => format_smolstr!("unknown key {:?}", key()),
            Self::MissingKey(what) => format_smolstr!("expected every entry to state {what:?}"),
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

    /// Reads one JSON string body that is a word: what [`is_word`] admits.
    ///
    /// A FIX name, a code value and a field's alternate name are words - non-
    /// empty, holding nothing the writer would escape - so an escape or an
    /// empty string in one is a hand edit rather than something the writer
    /// produced, and [`Words`] can split the array on its quotes alone.
    #[inline]
    pub(super) fn read_word(&mut self, key: &'static str) -> Scan<&'doc str> {
        let start = self.position;
        let word = self.read_string()?;
        if !is_word(word) {
            self.position = start;
            return Err(Refusal::NotAWord(key));
        }
        Ok(word)
    }

    /// Reads one nested array, handing back the text between its brackets.
    ///
    /// A list of fills is the one array these documents hold whose elements
    /// are objects, and one whose objects hold lists of their own, so the
    /// brackets and braces are balanced together by scanning and strings are
    /// skipped whole exactly as [`Self::read_string`] skips a string. The
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
    /// Every element is read as a decimal number that fits a tag so a
    /// hand-edited array is refused here rather than mis-read by
    /// [`Numbers`], which walks the slice handed back without checking it
    /// again. Whether an element is positive is the caller's rule, not the
    /// grammar's.
    pub(super) fn read_numbers(&mut self, key: &'static str) -> Scan<&'doc str> {
        self.expect(b'[')?;
        let start = self.position;
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(&self.document[start..start]);
        }
        loop {
            let at = self.position;
            if i32::try_from(self.read_number(key)?).is_err() {
                self.position = at;
                return Err(Refusal::TooWide(key));
            }
            if !self.next_element()? {
                return Ok(&self.document[start..self.position - 1]);
            }
        }
    }

    /// Reads one non-negative decimal number as JSON spells one: digits, and
    /// no leading zero in front of another digit.
    #[inline]
    fn read_number(&mut self, key: &'static str) -> Scan<u32> {
        let start = self.position;
        let bytes = self.document.as_bytes();
        while bytes.get(self.position).is_some_and(u8::is_ascii_digit) {
            self.position += 1;
        }
        let digits = &self.document[start..self.position];
        if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
            self.position = start;
            return Err(Refusal::NotANumber(key));
        }
        digits.parse().map_err(|_| {
            self.position = start;
            Refusal::TooWide(key)
        })
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
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(&self.document[start..start]);
        }
        loop {
            self.read_word(key)?;
            if !self.next_element()? {
                return Ok(&self.document[start..self.position - 1]);
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
    let decoded = crate::json::from_utf8(&quoted)?;
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

/// Whether `text` is a word: non-empty, and holding no byte the writer would
/// escape - a quote, a backslash or a control character - so the reader that
/// walks a list of them can split on the quotes alone.
pub(super) fn is_word(text: &str) -> bool {
    !text.is_empty() && needs_no_escape(text)
}

/// Whether `text` holds no byte a JSON string has to escape.
fn needs_no_escape(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte >= 0x20 && !matches!(byte, b'"' | b'\\'))
}

/// The first word that repeats an earlier one with ASCII case folded, which
/// is the one rule every list of names is held to: by the setter that writes
/// one, by the reader that validates one, and by the store crossing one.
pub(super) fn repeated_word<'a>(words: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen: Vec<&'a str> = Vec::new();
    for word in words {
        if seen.iter().any(|held| held.eq_ignore_ascii_case(word)) {
            return Some(word);
        }
        seen.push(word);
    }
    None
}

/// The first tag that repeats an earlier one, the one rule every list of
/// tags is held to.
pub(super) fn repeated_number(numbers: impl IntoIterator<Item = i32>) -> Option<i32> {
    let mut seen: Vec<i32> = Vec::new();
    for number in numbers {
        if seen.contains(&number) {
            return Some(number);
        }
        seen.push(number);
    }
    None
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
pub(super) struct Numbers<'doc>(&'doc str);

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
    /// Almost every value these documents hold is a word - a FIX name, a code
    /// value, an alternate name - and a word carries no quote, backslash or
    /// control character. Those are quoted directly; anything else goes
    /// through the crate's own JSON codec, so there is still exactly one
    /// escaper and it is the one that reads these documents back.
    fn string(&mut self, value: &str) -> Result<()> {
        if needs_no_escape(value) {
            self.text.push('"');
            self.text.push_str(value);
            self.text.push('"');
            return Ok(());
        }
        self.text
            .push_str(&crate::json::into_utf8(&Scalar::from(value))?);
        Ok(())
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
        self.word_list(values)
    }

    /// Renders one bare array of tags: the whole value of a list property.
    pub(super) fn list_of_numbers(values: impl IntoIterator<Item = i32>) -> String {
        let mut writer = Self::default();
        writer.number_list(values);
        writer.text
    }

    /// Renders one bare array of words: the whole value of a list property.
    ///
    /// # Errors
    ///
    /// Propagates the JSON codec's refusal, which text cannot provoke.
    pub(super) fn list_of_words<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<String> {
        let mut writer = Self::default();
        writer.word_list(values)?;
        Ok(writer.text)
    }

    fn number_list(&mut self, values: impl IntoIterator<Item = i32>) {
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

    fn word_list<'a>(&mut self, values: impl IntoIterator<Item = &'a str>) -> Result<()> {
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
// into a sorted `Scalar::Struct`, while the stored text is ordered by the
// grammar each reader walks. So both directions go through the one table
// below, which is the same declared order the writers write, and a hand-
// edited document is restated canonically rather than stored as it was
// spelled.
// ---------------------------------------------------------------------------

/// One `FIX:` property whose stored value is a canonical document.
///
/// Five properties hold one - three arrays of entries and two bare lists -
/// and this is what names one of them to the pair a store crosses:
/// [`Self::value_of`] reads the document as the JSON it is,
/// [`Self::text_of`] restates that JSON as the canonical text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    /// [`super::directions`], under `FIX:directions`.
    Directions,
    /// [`super::replacements`], under `FIX:replacements`.
    Replacements,
    /// [`FixField::names`](super::FixField::names), under `FIX:names`.
    Names,
    /// [`FixField::tags`](super::FixField::tags), under `FIX:tags`.
    Tags,
}

/// What one document's array holds.
#[derive(Clone, Copy)]
enum Shape {
    /// Objects, each stating some of these keys, in this order.
    Entries(&'static [&'static str]),
    /// Texts the writer never escapes.
    Words,
    /// Positive tags.
    Tags,
}

impl Kind {
    /// Every property a store crosses this way.
    pub(super) const ALL: [Self; 4] = [
        Self::Directions,
        Self::Replacements,
        Self::Names,
        Self::Tags,
    ];

    /// The metadata key this document is stored under.
    pub(super) const fn key(self) -> &'static str {
        match self {
            Self::Directions => "FIX:directions",
            Self::Replacements => "FIX:replacements",
            Self::Names => "FIX:names",
            Self::Tags => "FIX:tags",
        }
    }

    /// The one this metadata key names, or nothing for an ordinary property.
    pub(super) fn of_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// What this document calls itself in a refusal, as its reader does.
    const fn target(self) -> &'static str {
        match self {
            Self::Directions => "fix directions",
            Self::Replacements => "fix replacements",
            Self::Names => "fix names",
            Self::Tags => "fix tags",
        }
    }

    /// What this document's array holds: for a document of entries, the keys
    /// one entry states in the order its reader walks them.
    ///
    /// Every key of an entry is a leaf - a text, a number, a flag, or an
    /// array of those - so an entry is restated by putting its stated keys
    /// back into this order and carrying each value as it arrived.
    const fn shape(self) -> Shape {
        match self {
            Self::Directions => Shape::Entries(&super::directions::KEYS),
            Self::Replacements => Shape::Entries(&super::replacements::KEYS),
            Self::Names => Shape::Words,
            Self::Tags => Shape::Tags,
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
    /// The elements keep the order the file gave them - a code set is ranked
    /// by position and a list of names by priority, which is the writer's
    /// business rather than this one's - while each entry's own keys are put
    /// back into the order the grammar declares, whatever order the file
    /// spelled them in.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the value is not the array this document
    /// is: not an array of objects, or an entry stating a key this document
    /// does not declare; not an array of words; not an array of positive
    /// tags.
    pub(super) fn text_of(self, value: &Scalar) -> Result<String> {
        crate::into_json_scalar(&self.ordered(value)?)
    }

    /// The document with every entry's keys in the order this kind states
    /// them: the one order its reader walks and its writer writes. A list
    /// has no order to settle and is held to its element grammar instead.
    fn ordered(self, value: &Scalar) -> Result<Scalar> {
        let elements = value.as_sequence().ok_or_else(|| {
            self.refused(crate::text::expected_got(
                match self.shape() {
                    Shape::Entries(_) => "an array of entries",
                    Shape::Words => "an array of names",
                    Shape::Tags => "an array of tags",
                },
                value.kind(),
            ))
        })?;
        let ordered = match self.shape() {
            Shape::Entries(keys) => elements
                .iter()
                .map(|entry| self.order_entry(keys, entry))
                .collect::<Result<Vec<_>>>()?,
            Shape::Words => {
                let words = elements
                    .iter()
                    .map(|element| self.word(element))
                    .collect::<Result<Vec<_>>>()?;
                if let Some(twice) = repeated_word(words.iter().filter_map(Scalar::as_str)) {
                    return Err(
                        self.refused(format_args!("expected each name once, got {twice:?} twice"))
                    );
                }
                words
            }
            Shape::Tags => {
                let tags = elements
                    .iter()
                    .map(|element| self.tag(element))
                    .collect::<Result<Vec<_>>>()?;
                if let Some(twice) =
                    repeated_number(tags.iter().filter_map(|tag| tag.as_i64()?.try_into().ok()))
                {
                    return Err(
                        self.refused(format_args!("expected each tag once, got {twice} twice"))
                    );
                }
                tags
            }
        };
        Ok(Scalar::from_sequence(ordered))
    }

    /// One element of a list of words, which the reader hands back as the
    /// slice it is: a non-empty text holding nothing the writer would escape.
    fn word(self, element: &Scalar) -> Result<Scalar> {
        let text = element
            .as_str()
            .filter(|text| is_word(text))
            .ok_or_else(|| self.refused(format_args!("expected a word, got {element:?}")))?;
        Ok(Scalar::from(text))
    }

    /// One element of a list of tags: a positive integer that fits an `i32`.
    fn tag(self, element: &Scalar) -> Result<Scalar> {
        let tag = element
            .as_i64()
            .and_then(|held| i32::try_from(held).ok())
            .filter(|held| *held > 0)
            .ok_or_else(|| {
                self.refused(format_args!("expected a positive tag, got {element:?}"))
            })?;
        Ok(Scalar::from(tag))
    }

    /// One entry, its stated keys in the declared order and nothing else.
    fn order_entry(self, keys: &'static [&'static str], entry: &Scalar) -> Result<Scalar> {
        order_entry(self.target(), keys, entry)
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
        refused(self.target(), reason)
    }
}

/// One array of entries with every entry's keys in the order `keys` states
/// them: the one order a reader walks and a writer writes.
///
/// The elements keep the order the file gave them; what is settled is each
/// entry's own keys. The shared half of [`Kind::ordered`] and of a document
/// a store keeps in a file of its own, such as a [code set](super::codes).
///
/// # Errors
///
/// Returns [`Error::Parse`] naming `target` when the value is not an array of
/// entry objects, or one states a key `keys` does not declare.
pub(super) fn ordered_entries(
    target: &'static str,
    keys: &'static [&'static str],
    value: &Scalar,
) -> Result<Scalar> {
    let elements = value.as_sequence().ok_or_else(|| {
        refused(
            target,
            crate::text::expected_got("an array of entries", value.kind()),
        )
    })?;
    elements
        .iter()
        .map(|entry| order_entry(target, keys, entry))
        .collect::<Result<Vec<_>>>()
        .map(Scalar::from_sequence)
}

/// One entry with its stated keys in the order `keys` declares them.
fn order_entry(
    target: &'static str,
    keys: &'static [&'static str],
    entry: &Scalar,
) -> Result<Scalar> {
    if entry.as_struct().is_none() && entry.as_mapping().is_none() {
        return Err(refused(
            target,
            crate::text::expected_got("an entry object", entry.kind()),
        ));
    }
    let mut held: Vec<(Scalar, Scalar)> = Vec::with_capacity(keys.len());
    for key in keys {
        let Some(value) = entry.get_key_str(key) else {
            continue;
        };
        // A flag exists only when it is true, and a key a file spelled null
        // states nothing: both leave the entry rather than being written back
        // as something the reader would refuse.
        if matches!(value, Scalar::Null) || value.as_bool() == Some(false) {
            continue;
        }
        held.push((Scalar::from(*key), value.clone()));
    }
    if let Some(unknown) = entry.keys().into_iter().find(|key| !keys.contains(key)) {
        return Err(refused(target, format_args!("unknown key {unknown:?}")));
    }
    // A mapping rather than a record, because a record sorts its keys and the
    // order this just settled is the whole point.
    Scalar::from_mapping(held)
}

/// The refusal a document raises, naming the document it is.
fn refused(target: &'static str, reason: impl fmt::Display) -> Error {
    Error::Parse {
        target,
        position: 0,
        reason: format_smolstr!("{reason}"),
    }
}

/// One field as a FIX store spells it: a native `Field` document whose `FIX:`
/// document properties are the JSON they are rather than one escaped line.
///
/// The shape [`FixRegistry::write_into`](super::FixRegistry::write_into) and
/// [`FixRegistry::into_json`](super::FixRegistry::into_json) write and
/// [`from_fix_document`] reads, exposed on its own so a caller editing one
/// document out of a store - what `ygg fix read --json` prints and
/// `ygg fix ... --input` takes - writes the same shape the store does. It is
/// the FIX spelling of [`Field::into_value`](crate::Field::into_value), and
/// the only difference between them is those four properties.
///
/// A field's `FIX:codeset` is not one of them: it names the
/// [`FixCodeSet`](super::FixCodeSet) the field reads by, and the members live
/// in the store's own `codesets/` folder rather than in the field.
///
/// ```
/// use yggdryl::{DataType, Scalar, fix};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut side = DataType::utf8().nullable_field("Side");
/// side.as_fix_mut().set_tag(54)?;
/// side.as_fix_mut().set_codeset("sidecodeset")?;
/// side.as_fix_mut().set_names(["side"])?;
///
/// let document = fix::into_fix_document(side.clone())?;
/// let metadata = document.get_key_str("metadata").expect("the metadata");
/// // The set it names, carried as the text it is.
/// assert_eq!(
///     metadata.get_key_str("FIX:codeset").and_then(Scalar::as_str),
///     Some("sidecodeset"),
/// );
/// // A list property is still the JSON it is, not the text it is stored as.
/// assert_eq!(metadata.get_key_str("FIX:names").map(|names| names.len()), Some(1));
/// assert_eq!(fix::from_fix_document(document)?, side);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`Error::InvalidMetadataValue`] naming the property when a field
/// holds text under one of the five keys that is not the document that key
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
/// document spells one of the five keys as anything but the array it is, and
/// what [`Field::from_value`](crate::Field::from_value) returns
/// for a document that is not a field.
pub fn from_fix_document(document: Scalar) -> Result<crate::Field> {
    crate::Field::from_value(load(document)?)
}

/// One native Field document as a store writes it: every `FIX:` document
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
/// spells one of these keys as anything but the array it is - the escaped
/// text an older writer wrote included, because a store reads one shape - or
/// when an entry states a key the document does not declare.
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

/// One `metadata` map with the five document properties crossed over.
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
    if value.as_struct().is_none() && value.as_mapping().is_none() {
        return None;
    }
    Some(value.keys().into_iter().map(str::to_owned).collect())
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/mod_.rs` pins and a caller cannot reach.
    //!
    //! [`into_fix_document`](crate::into_fix_document) and
    //! [`from_fix_document`](crate::from_fix_document) are the published pair
    //! over a whole field; these two are the same rewrite over one stored
    //! value, which a snapshot comparison reads a definition at a time.
    use crate::{Result, Scalar};

    /// Write one stored definition value as its compact document.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where the value is not a definition.
    pub fn dump(value: Scalar) -> Result<Scalar> {
        super::dump(value)
    }

    /// Read one compact document back as the stored definition value.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where the document is malformed.
    pub fn load(value: Scalar) -> Result<Scalar> {
        super::load(value)
    }
}
