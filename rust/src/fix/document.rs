//! The canonical JSON documents the `fix:` namespace stores, read borrowed.
//!
//! Two `fix:` properties hold more than one text can say as a list: the
//! per-version [lineage](super::lineage) and the [code set](super::codes).
//! Both are JSON, because a metadata value may hold no control character and
//! so cannot be separator-framed, and both are read on hot paths where
//! building a parse tree per ask would cost more than the lookup.
//!
//! So one convention serves both. A document is rendered with its keys in a
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
        let Some(index) = keys.iter().position(|declared| *declared == key) else {
            self.position = at;
            return Err(Refusal::UnknownKey);
        };
        if index < *next {
            self.position = at;
            return Err(Refusal::KeyOrder);
        }
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

    /// Writes one text-valued key.
    ///
    /// # Errors
    ///
    /// Propagates the JSON codec's refusal, which text cannot provoke.
    pub(super) fn text(&mut self, first: bool, key: &str, value: &str) -> Result<()> {
        self.key(first, key);
        self.text
            .push_str(&crate::text::json::into_utf8(&Scalar::from(value))?);
        Ok(())
    }

    /// Writes one number-valued key.
    pub(super) fn number(&mut self, first: bool, key: &str, value: u32) {
        self.key(first, key);
        // Writing into a `String` cannot fail.
        use std::fmt::Write as _;
        let _ = write!(self.text, "{value}");
    }

    /// Writes one flag key, which exists only when true.
    pub(super) fn flag(&mut self, first: bool, key: &str) {
        self.key(first, key);
        self.text.push_str("true");
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
