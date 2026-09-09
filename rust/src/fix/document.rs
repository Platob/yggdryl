//! The canonical JSON documents the `fix:` namespace stores, read borrowed.
//!
//! Three `fix:` properties hold more than one text can say as a list: the
//! per-version [lineage](super::lineage), the [code set](super::codes) and
//! the [replacements](super::replacements) a value is restated through. All
//! are JSON, because a metadata value may hold no control character and so
//! cannot be separator-framed, and all are read on hot paths where building a
//! parse tree per ask would cost more than the lookup.
//!
//! So one convention serves them. A document is rendered with its keys in a
//! **declared order** rather than sorted, compactly, with one text per value;
//! the reader walks the bytes and hands back slices of them, allocating
//! nothing. Declared order is what makes the walk safe *and* cheap: a reader
//! knows which key can come next, so a hand-edited document with reordered or
//! repeated keys is refused with its byte position rather than mis-read, and
//! the key a lookup keys on is put first so a scan can stop at it.
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
    /// The document did not open on the array it holds.
    WrongRoot(&'static str),
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
            Self::WrongRoot(what) => {
                format_smolstr!("expected the document to hold {what:?}")
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

    /// Opens the document on the one array it holds.
    ///
    /// Answers whether that array holds anything, so a caller stops without a
    /// second probe.
    pub(super) fn open_array(&mut self, key: &'static str) -> Scan<bool> {
        self.expect(b'{')?;
        let at = self.position;
        if self.read_string()? != key {
            self.position = at;
            return Err(Refusal::WrongRoot(key));
        }
        self.expect(b':')?;
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
    /// Opens a document on the array it holds.
    pub(super) fn open_array(key: &str) -> Self {
        let mut text = String::new();
        text.push_str("{\"");
        text.push_str(key);
        text.push_str("\":[");
        Self { text, empty: true }
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

    /// Closes the array, leaving the document open for its trailing keys.
    pub(super) fn close_array(&mut self) {
        self.text.push(']');
        self.empty = false;
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

    /// Finishes the document.
    pub(super) fn finish(mut self) -> String {
        self.text.push('}');
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
