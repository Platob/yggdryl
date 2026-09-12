//! The character encodings Yggdryl reads and writes, and how to apply them.
//!
//! One [`Charset`] vocabulary names every encoding the way [`Codec`] names
//! every content coding, and the modules beside it own the implementations.
//! Every charset exposes the same four operations: `decode`/`encode` for whole
//! buffers and `reader`/`writer` for streams, with [`Decoder`] underneath them
//! for callers that already hold their bytes in chunks.
//!
//! Text crosses this boundary exactly once. A byte payload is decoded to UTF-8
//! at intake - by a [`Transcoded`] handle, by [`crate::media::text::TextOptions`],
//! or by a direct [`Charset::decode`] - and everything past that point is
//! `str`, `Scalar::String`, or an Arrow string array whose bytes are already
//! UTF-8. Nothing re-decodes, and no layer branches on a charset per row.
//!
//! ```
//! use yggdryl::Charset;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // One byte per scalar on the wire, three in UTF-8.
//! let wire = b"prix: 12\x80";
//! assert_eq!(Charset::Cp1252.decode(wire)?, "prix: 12€");
//! assert_eq!(Charset::Cp1252.encode("prix: 12€")?.as_ref(), wire);
//!
//! // The same bytes are a different document under a different charset.
//! assert_eq!(Charset::Latin1.decode(wire)?, "prix: 12\u{0080}");
//!
//! // A name from a header, a filename, or a configuration file resolves once.
//! assert_eq!(Charset::from_str("windows-1252")?, Charset::Cp1252);
//! # Ok(())
//! # }
//! ```
//!
//! [`Codec`]: crate::Codec
//!
//! # Borrowing
//!
//! Every charset here agrees with US-ASCII below `0x80`, so an all-ASCII
//! payload is already UTF-8 and [`Charset::decode`] borrows it rather than
//! transcoding it. That is the common case for a legacy export - a
//! `windows-1252` file whose accented names are a fraction of its bytes - and
//! it is asserted in the counting allocator rather than argued.

use std::borrow::Cow;
use std::fmt;
use std::io::{Read, Write};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

mod ascii;
mod bom;
mod decoder;
mod reader;
mod single_byte;
mod sink;
mod tables;
mod transcoded;
mod unicode;
mod writer;

pub use decoder::Decoder;
pub use reader::Reader;
pub use transcoded::Transcoded;
pub use writer::Writer;

pub(crate) use ascii::ascii_len;
use ascii::text;

/// How many bytes `SmolStr` holds without reaching the heap.
///
/// Pinned rather than imported: `smol_str` does not export it, and a value at
/// or under this width is the case [`Charset::transcribe_smol`] exists to keep
/// free. The test beside it fails if the dependency ever moves it.
const INLINE_CAPACITY: usize = 23;
use single_byte::SingleByte;

use crate::{Error, MediaType, Result, Url};

/// The scalar a lossy decode puts in place of bytes it cannot read.
pub(crate) const REPLACEMENT: char = '\u{FFFD}';

/// [`REPLACEMENT`] in UTF-8, for a target that takes bytes.
pub(crate) const REPLACEMENT_UTF8: &[u8] = "\u{FFFD}".as_bytes();

/// One character encoding, and the only place a name selects an implementation.
///
/// The canonical spelling of each is its IANA name, which is also what a
/// `Content-Type` header and a [`MediaType`] carry. [`Charset::from_str`]
/// additionally accepts the customary aliases - `utf8`, `latin1`, `cp1252`,
/// `macroman` - and resolves every one of them to the same value.
///
/// Two deliberate refusals:
///
/// * `iso-8859-1` decodes as ISO 8859-1, not as `windows-1252`. The two differ
///   over `0x80..=0x9F`, and a value that silently answered one for the other
///   would put a smart quote where a control character was written.
/// * Bare `utf-16` is not a spelling of either order. RFC 2781 reads an
///   unmarked stream as big-endian and the WHATWG Encoding Standard reads it as
///   little-endian, so the name is ambiguous rather than defaulted:
///   [`Charset::from_bom`] answers a marked payload, and `utf-16le`/`utf-16be`
///   name an unmarked one.
///
/// New charsets are a table and a variant, so this is open the way every other
/// shared vocabulary in the crate is: match it with a final arm.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Charset {
    /// UTF-8, the encoding every decoded value in this crate is already in.
    #[default]
    Utf8,
    /// UTF-16, little-endian code units.
    Utf16Le,
    /// UTF-16, big-endian code units.
    Utf16Be,
    /// US-ASCII: seven bits per scalar, and nothing above them.
    Ascii,
    /// ISO 8859-1 (Latin-1): `0x00..=0xFF` maps to `U+0000..=U+00FF`.
    Latin1,
    /// ISO 8859-2 (Latin-2), for Central and Eastern European Latin scripts.
    Latin2,
    /// ISO 8859-15 (Latin-9): Latin-1 with the euro sign and seven others.
    Latin9,
    /// Windows code page 1250, Central European.
    Cp1250,
    /// Windows code page 1251, Cyrillic.
    Cp1251,
    /// Windows code page 1252, Western European: Latin-1 with 27 printable
    /// scalars where Latin-1 has C1 control characters.
    Cp1252,
    /// IBM code page 437, the original IBM PC character set.
    Cp437,
    /// IBM code page 850, the Western European DOS character set.
    Cp850,
    /// Mac OS Roman, as classic Mac OS wrote Western European text.
    MacRoman,
}

/// Every alias [`Charset::from_str`] accepts beside the canonical names.
///
/// Intake is the only place this is read: a name resolves to a [`Charset`]
/// once, and nothing past that boundary sees a string again.
const ALIASES: &[(&str, Charset)] = &[
    ("utf8", Charset::Utf8),
    ("utf_8", Charset::Utf8),
    ("u8", Charset::Utf8),
    ("unicode-1-1-utf-8", Charset::Utf8),
    ("csutf8", Charset::Utf8),
    ("utf16le", Charset::Utf16Le),
    ("utf-16-le", Charset::Utf16Le),
    ("utf_16_le", Charset::Utf16Le),
    ("csutf16le", Charset::Utf16Le),
    ("utf16be", Charset::Utf16Be),
    ("utf-16-be", Charset::Utf16Be),
    ("utf_16_be", Charset::Utf16Be),
    ("csutf16be", Charset::Utf16Be),
    ("ascii", Charset::Ascii),
    ("usascii", Charset::Ascii),
    ("us", Charset::Ascii),
    ("iso-ir-6", Charset::Ascii),
    ("iso646-us", Charset::Ascii),
    ("ansi_x3.4-1968", Charset::Ascii),
    ("ansi_x3.4-1986", Charset::Ascii),
    ("cp367", Charset::Ascii),
    ("ibm367", Charset::Ascii),
    ("csascii", Charset::Ascii),
    ("iso8859-1", Charset::Latin1),
    ("iso_8859-1", Charset::Latin1),
    ("iso88591", Charset::Latin1),
    ("8859-1", Charset::Latin1),
    ("latin1", Charset::Latin1),
    ("latin-1", Charset::Latin1),
    ("l1", Charset::Latin1),
    ("iso-ir-100", Charset::Latin1),
    ("cp819", Charset::Latin1),
    ("ibm819", Charset::Latin1),
    ("csisolatin1", Charset::Latin1),
    ("iso8859-2", Charset::Latin2),
    ("iso_8859-2", Charset::Latin2),
    ("iso88592", Charset::Latin2),
    ("8859-2", Charset::Latin2),
    ("latin2", Charset::Latin2),
    ("latin-2", Charset::Latin2),
    ("l2", Charset::Latin2),
    ("iso-ir-101", Charset::Latin2),
    ("csisolatin2", Charset::Latin2),
    ("iso8859-15", Charset::Latin9),
    ("iso_8859-15", Charset::Latin9),
    ("iso885915", Charset::Latin9),
    ("8859-15", Charset::Latin9),
    ("latin9", Charset::Latin9),
    ("latin-9", Charset::Latin9),
    ("l9", Charset::Latin9),
    ("iso-ir-203", Charset::Latin9),
    ("csisolatin9", Charset::Latin9),
    ("cp1250", Charset::Cp1250),
    ("windows1250", Charset::Cp1250),
    ("x-cp1250", Charset::Cp1250),
    ("cp1251", Charset::Cp1251),
    ("windows1251", Charset::Cp1251),
    ("x-cp1251", Charset::Cp1251),
    ("cp1252", Charset::Cp1252),
    ("windows1252", Charset::Cp1252),
    ("x-cp1252", Charset::Cp1252),
    ("cp437", Charset::Cp437),
    ("437", Charset::Cp437),
    ("oem-us", Charset::Cp437),
    ("cspc8codepage437", Charset::Cp437),
    ("cp850", Charset::Cp850),
    ("850", Charset::Cp850),
    ("cspc850multilingual", Charset::Cp850),
    ("mac", Charset::MacRoman),
    ("macroman", Charset::MacRoman),
    ("mac-roman", Charset::MacRoman),
    ("x-mac-roman", Charset::MacRoman),
    ("csmacintosh", Charset::MacRoman),
];

impl Charset {
    /// Every charset, in declaration order.
    pub const ALL: [Self; 13] = [
        Self::Utf8,
        Self::Utf16Le,
        Self::Utf16Be,
        Self::Ascii,
        Self::Latin1,
        Self::Latin2,
        Self::Latin9,
        Self::Cp1250,
        Self::Cp1251,
        Self::Cp1252,
        Self::Cp437,
        Self::Cp850,
        Self::MacRoman,
    ];

    /// The `charset` parameter name, as a `Content-Type` header spells it.
    pub const PARAMETER: &'static str = "charset";

    /// Resolve a charset name or alias.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the canonical vocabulary and the input.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical IANA name without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => unicode::UTF8,
            Self::Utf16Le => unicode::UTF16LE,
            Self::Utf16Be => unicode::UTF16BE,
            Self::Ascii => ascii::NAME,
            Self::Latin1 => tables::LATIN1.name,
            Self::Latin2 => tables::LATIN2.name,
            Self::Latin9 => tables::LATIN9.name,
            Self::Cp1250 => tables::CP1250.name,
            Self::Cp1251 => tables::CP1251.name,
            Self::Cp1252 => tables::CP1252.name,
            Self::Cp437 => tables::CP437.name,
            Self::Cp850 => tables::CP850.name,
            Self::MacRoman => tables::MAC_ROMAN.name,
        }
    }

    /// Return whether decoding this charset is the identity on UTF-8 bytes.
    pub const fn is_utf8(self) -> bool {
        matches!(self, Self::Utf8)
    }

    /// Return whether this is one of the Unicode encoding forms.
    pub const fn is_unicode(self) -> bool {
        matches!(self, Self::Utf8 | Self::Utf16Le | Self::Utf16Be)
    }

    /// Return whether bytes `0x00..=0x7F` mean what US-ASCII says they mean.
    ///
    /// Every charset but UTF-16 answers `true`, which is what lets a decode
    /// borrow an all-ASCII payload and lets a line scan split on `\n` before
    /// anything is decoded.
    pub const fn is_ascii_compatible(self) -> bool {
        !matches!(self, Self::Utf16Le | Self::Utf16Be)
    }

    /// Return whether one byte is one scalar.
    pub const fn is_single_byte(self) -> bool {
        !matches!(self, Self::Utf8 | Self::Utf16Le | Self::Utf16Be)
    }

    /// Return the width of one code unit in bytes.
    ///
    /// A payload in this charset is a whole number of units, so this is the
    /// alignment a positional read has to respect.
    pub const fn unit_size(self) -> usize {
        match self {
            Self::Utf16Le | Self::Utf16Be => 2,
            _ => 1,
        }
    }

    /// Return the byte-order mark this charset is written with, if it has one.
    pub const fn bom(self) -> Option<&'static [u8]> {
        bom::mark(self)
    }

    /// Recover the charset a payload's own byte-order mark declares, with the
    /// length of that mark.
    ///
    /// A mark is a content read, so it sits last in the precedence an intake
    /// follows: an explicit argument first, then a declared [`MediaType`], then
    /// this. Nothing strips the mark on a caller's behalf - the length is
    /// answered so the caller decides whether `U+FEFF` is data or framing.
    ///
    /// ```
    /// use yggdryl::Charset;
    ///
    /// let payload = b"\xff\xfeA\x00";
    /// let (charset, mark) = Charset::from_bom(payload).expect("a marked payload");
    /// assert_eq!(charset, Charset::Utf16Le);
    /// assert_eq!(&payload[mark..], b"A\x00");
    /// ```
    pub fn from_bom(input: &[u8]) -> Option<(Self, usize)> {
        bom::detect(input)
    }

    /// Recover the charset a media type declares, defaulting to UTF-8.
    ///
    /// A media type carrying no charset answers [`Charset::Utf8`], because a
    /// payload that declares nothing is read as the encoding this crate's own
    /// values are already in.
    pub fn from_media_type(value: &MediaType) -> Self {
        value.charset().unwrap_or_default()
    }

    /// Recover the charset a location's media type declares.
    pub fn from_url(value: &Url) -> Self {
        Self::from_media_type(&value.media_type())
    }

    /// Read the `charset` parameter out of a `Content-Type` header value.
    ///
    /// Answers `None` when the header carries no such parameter, which is the
    /// header saying nothing rather than saying UTF-8.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the parameter names no known charset.
    ///
    /// ```
    /// use yggdryl::Charset;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let declared = Charset::from_content_type("text/csv; charset=Windows-1252")?;
    /// assert_eq!(declared, Some(Charset::Cp1252));
    /// assert_eq!(Charset::from_content_type("text/csv")?, None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_content_type(value: &str) -> Result<Option<Self>> {
        let Some(parameter) = content_type_charset(value) else {
            return Ok(None);
        };
        Self::from_str(parameter).map(Some)
    }

    /// The scalar one byte decodes to in a single-byte charset.
    ///
    /// Answers `None` for a byte the charset leaves unassigned, and for every
    /// byte of a charset whose scalars are not one byte wide - UTF-8 and
    /// UTF-16 have no per-byte answer to give.
    pub fn scalar_of(self, byte: u8) -> Option<char> {
        match self.table() {
            Some(table) => table.scalar_of(byte),
            None if matches!(self, Self::Ascii) => byte.is_ascii().then(|| char::from(byte)),
            None => None,
        }
    }

    /// The byte one scalar encodes to in a single-byte charset.
    ///
    /// Answers `None` where the charset has no byte for the scalar, and for
    /// the Unicode encoding forms, whose scalars are not one byte wide.
    pub fn byte_of(self, scalar: char) -> Option<u8> {
        match self.table() {
            Some(table) => table.byte_of(scalar),
            None if matches!(self, Self::Ascii) => scalar.is_ascii().then_some(scalar as u8),
            None => None,
        }
    }

    /// Decode a complete buffer.
    ///
    /// The answer borrows `input` whenever the payload is already UTF-8: an
    /// all-ASCII payload under any ASCII-compatible charset, and any valid
    /// payload under [`Charset::Utf8`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] naming the charset, the byte position, and
    /// what was found there.
    pub fn decode(self, input: &[u8]) -> Result<Cow<'_, str>> {
        match self {
            Self::Utf8 => unicode::utf8_decode(input),
            Self::Utf16Le => unicode::utf16_decode::<false>(input),
            Self::Utf16Be => unicode::utf16_decode::<true>(input),
            Self::Ascii => ascii::decode(input),
            _ => match self.table() {
                Some(table) => table.decode(input),
                None => unicode::utf8_decode(input),
            },
        }
    }

    /// Decode a complete buffer, replacing what it cannot read.
    ///
    /// Each undecodable byte or broken sequence becomes `U+FFFD`. This is the
    /// contract a capture wants - a wire line is read whatever it holds - and
    /// never the one a stored column wants, which is [`Charset::decode`].
    pub fn decode_lossy(self, input: &[u8]) -> Cow<'_, str> {
        match self {
            Self::Utf8 => String::from_utf8_lossy(input),
            Self::Utf16Le => unicode::utf16_decode_lossy::<false>(input),
            Self::Utf16Be => unicode::utf16_decode_lossy::<true>(input),
            Self::Ascii => {
                let mut target = String::new();
                match ascii::decode_into::<true>(input, &mut target) {
                    Ok(()) => Cow::Owned(target),
                    Err(_) => Cow::Owned(String::from(REPLACEMENT)),
                }
            }
            _ => match self.table() {
                Some(table) => table.decode_lossy(input),
                None => String::from_utf8_lossy(input),
            },
        }
    }

    /// Decode a complete buffer, transcribing what it cannot read exactly.
    ///
    /// This is the permissive door, and it recovers rather than replaces
    /// wherever a recovery is defined rather than guessed:
    ///
    /// * A byte a single-byte charset leaves unassigned reads as the scalar
    ///   ISO 8859-1 gives it. Every byte has one, and for the five bytes
    ///   `windows-1252` leaves unassigned it is exactly what the WHATWG
    ///   Encoding Standard's own index answers - so this is the transcription,
    ///   not a fallback.
    /// * Bytes offered as UTF-8 that are not UTF-8 are read as ISO 8859-1
    ///   instead, which is what a mislabelled Western export nearly always is
    ///   and which assigns every byte. A payload that really was broken UTF-8
    ///   reads as mojibake rather than as `U+FFFD`, and that is the trade:
    ///   this door never loses a byte, and [`Charset::decode_lossy`] is the
    ///   one that marks damage where it is.
    /// * A broken UTF-16 sequence has no byte-wise reading at all - a lone
    ///   surrogate is not a scalar in any encoding - so those become
    ///   `U+FFFD`, exactly as [`Charset::decode_lossy`] leaves them.
    ///
    /// The three doors are one verb with three contracts:
    /// [`Charset::decode`] refuses and says where, [`Charset::decode_lossy`]
    /// marks each fault with `U+FFFD`, and this one reads every byte it can.
    ///
    /// ```
    /// use yggdryl::Charset;
    ///
    /// // `0x81` is unassigned in windows-1252; ISO 8859-1 gives it `U+0081`.
    /// assert!(Charset::Cp1252.decode(b"ok\x81").is_err());
    /// assert_eq!(Charset::Cp1252.decode_lossy(b"ok\x81"), "ok\u{FFFD}");
    /// assert_eq!(Charset::Cp1252.transcribe(b"ok\x81"), "ok\u{0081}");
    ///
    /// // Bytes that are not the UTF-8 they claim to be still read.
    /// assert_eq!(Charset::Utf8.transcribe(b"caf\xe9"), "café");
    /// ```
    pub fn transcribe(self, input: &[u8]) -> Cow<'_, str> {
        match self {
            // Valid UTF-8 is itself; anything else is read as the single-byte
            // charset that assigns every byte, which is what a mislabelled
            // Western export is.
            Self::Utf8 => match unicode::utf8_decode(input) {
                Ok(text) => text,
                Err(_) => Self::Latin1.decode_lossy(input),
            },
            // A lone surrogate is not a scalar anywhere, so there is nothing
            // to transcribe it to.
            Self::Utf16Le | Self::Utf16Be => self.decode_lossy(input),
            // US-ASCII assigns no byte above `0x7F`, and ISO 8859-1 assigns
            // every one of them, so a high byte reads as the Latin-1 scalar.
            Self::Ascii => Self::Latin1.decode_lossy(input),
            _ => match self.table() {
                Some(table) if table.is_complete() => self.decode_lossy(input),
                Some(table) => {
                    // The three incomplete tables are the only charsets with a
                    // byte to transcribe, and they are still ASCII-compatible,
                    // so an all-ASCII payload is already its own answer. This
                    // is the borrow `decode`, `decode_lossy` and `encode` all
                    // take at their first line; without it this door was the
                    // one that allocated for text it did not have to touch.
                    if let Ok(borrowed) = text(input) {
                        if ascii_len(input) == input.len() {
                            return Cow::Borrowed(borrowed);
                        }
                    }
                    let mut target = String::new();
                    match table.transcribe_into(input, &mut target) {
                        Ok(()) => Cow::Owned(target),
                        Err(_) => self.decode_lossy(input),
                    }
                }
                None => self.decode_lossy(input),
            },
        }
    }

    /// Transcribe a complete buffer into compact string storage.
    ///
    /// The same reading as [`Charset::transcribe`], written into the storage
    /// every text value in this crate holds. A payload that transcribes to
    /// twenty-three bytes or fewer never reaches the heap at all, where
    /// `SmolStr::new(charset.transcribe(..))` would build a `String` first and
    /// copy out of it.
    ///
    /// Above that width there is no saving to claim and none is claimed: the
    /// builder spills to a `String` and `finish` allocates the `Arc<str>` it
    /// hands back, which is the same two allocations, because `String` and
    /// `Arc<str>` have different layouts and no conversion between them is
    /// free.
    ///
    /// ```
    /// use yggdryl::Charset;
    ///
    /// // `0x81` is unassigned in windows-1252 and reads as its ISO 8859-1
    /// // scalar, exactly as `transcribe` answers it.
    /// assert_eq!(Charset::Cp1252.transcribe_smol(b"ok\x81"), "ok\u{0081}");
    /// assert_eq!(Charset::Cp1252.transcribe_smol(b"caf\xe9"), "café");
    /// ```
    #[must_use]
    pub fn transcribe_smol(self, input: &[u8]) -> smol_str::SmolStr {
        // A borrow means the answer is the input, and `SmolStr` copies a short
        // one inline; only an owned answer had an intermediate worth avoiding.
        match self.transcribe_borrowed(input) {
            Some(borrowed) => smol_str::SmolStr::new(borrowed),
            // A transcription never shrinks - every byte answers at least one
            // UTF-8 byte - so an input past the inline buffer is an answer
            // past it too, and the builder would only spill to a `String` it
            // could not have reserved. That case takes the sized `String`
            // directly: one buffer and one handle, which is the floor, since
            // `String` and `Arc<str>` have different layouts.
            None if input.len() > INLINE_CAPACITY => {
                let mut target = String::new();
                match self.transcribe_sink(input, &mut target) {
                    Ok(()) => smol_str::SmolStr::new(target),
                    Err(_) => smol_str::SmolStr::new(self.decode_lossy(input)),
                }
            }
            None => {
                let mut target = smol_str::SmolStrBuilder::new();
                match self.transcribe_sink(input, &mut target) {
                    Ok(()) => target.finish(),
                    Err(_) => smol_str::SmolStr::new(self.decode_lossy(input)),
                }
            }
        }
    }

    /// The transcription that is the input itself, when it is.
    fn transcribe_borrowed(self, input: &[u8]) -> Option<&str> {
        match self {
            Self::Utf16Le | Self::Utf16Be => None,
            Self::Utf8 => unicode::utf8_decode(input)
                .ok()
                .and_then(|text| match text {
                    Cow::Borrowed(text) => Some(text),
                    Cow::Owned(_) => None,
                }),
            // Every other charset here agrees with US-ASCII below `0x80`.
            _ => (ascii_len(input) == input.len())
                .then(|| text(input).ok())
                .flatten(),
        }
    }

    /// The one transcription body, written against whichever target a caller
    /// brought.
    fn transcribe_sink(self, input: &[u8], target: &mut impl sink::Utf8Sink) -> Result<()> {
        match self {
            Self::Utf8 => match unicode::utf8_decode(input) {
                Ok(_) => unicode::utf8_decode_into::<false>(input, target),
                Err(_) => Self::Latin1.decode_sink::<true>(input, target),
            },
            Self::Utf16Le | Self::Utf16Be => self.decode_sink::<true>(input, target),
            Self::Ascii => Self::Latin1.decode_sink::<true>(input, target),
            _ => match self.table() {
                Some(table) if table.is_complete() => self.decode_sink::<true>(input, target),
                Some(table) => table.transcribe_sink(input, target),
                None => self.decode_sink::<true>(input, target),
            },
        }
    }

    /// Decode a complete buffer onto the end of `target`.
    ///
    /// # Errors
    ///
    /// Returns the same refusal as [`Charset::decode`]; `target` may already
    /// hold the text decoded before the failing byte.
    pub fn decode_into(self, input: &[u8], target: &mut String) -> Result<()> {
        self.decode_sink::<false>(input, target)
    }

    /// Decode a complete buffer onto the end of a UTF-8 byte target.
    ///
    /// # Errors
    ///
    /// Returns the same refusal as [`Charset::decode`].
    pub fn decode_bytes_into(self, input: &[u8], target: &mut Vec<u8>) -> Result<()> {
        self.decode_sink::<false>(input, target)
    }

    /// The one decode, written against whichever target a caller brought.
    fn decode_sink<const LOSSY: bool>(
        self,
        input: &[u8],
        target: &mut impl sink::Utf8Sink,
    ) -> Result<()> {
        match self {
            Self::Utf8 => unicode::utf8_decode_into::<LOSSY>(input, target),
            Self::Utf16Le => unicode::utf16_decode_into::<false, LOSSY>(input, target),
            Self::Utf16Be => unicode::utf16_decode_into::<true, LOSSY>(input, target),
            Self::Ascii => ascii::decode_into::<LOSSY>(input, target),
            _ => match self.table() {
                Some(table) => table.decode_into::<LOSSY>(input, target),
                None => unicode::utf8_decode_into::<LOSSY>(input, target),
            },
        }
    }

    /// Encode complete text.
    ///
    /// The answer borrows `input` whenever its bytes are already this
    /// charset's: any text under [`Charset::Utf8`], and US-ASCII text under
    /// any ASCII-compatible charset.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] naming the charset, the byte position, and the
    /// scalar it has no byte for.
    pub fn encode(self, input: &str) -> Result<Cow<'_, [u8]>> {
        match self {
            Self::Utf8 => Ok(Cow::Borrowed(input.as_bytes())),
            Self::Utf16Le | Self::Utf16Be => {
                let mut target = Vec::new();
                self.encode_into(input, &mut target)?;
                Ok(Cow::Owned(target))
            }
            Self::Ascii => ascii::encode(input),
            _ => match self.table() {
                Some(table) => table.encode(input),
                None => Ok(Cow::Borrowed(input.as_bytes())),
            },
        }
    }

    /// How many bytes this charset stores `input` in, without storing them.
    ///
    /// A length bound counts stored bytes, and the stored length is a
    /// property of the text and the charset rather than of any buffer - so
    /// asking for it costs a walk of the scalars and no allocation at all,
    /// where [`Charset::encode`] would build the bytes to measure them.
    ///
    /// It counts rather than judges: a scalar this charset has no byte for
    /// still occupies the one byte it would occupy, because that is what a
    /// bound is asking about. Whether the bytes can be written at all is
    /// [`Charset::encode`]'s question, answered where they are written - and
    /// it has to be that way round, because [`Charset::transcribe`] recovers
    /// damage precisely by answering scalars the charset does not assign.
    ///
    /// ```
    /// use yggdryl::Charset;
    ///
    /// // Four scalars: four bytes in windows-1252, five in UTF-8.
    /// assert_eq!(Charset::Cp1252.encoded_len("café"), 4);
    /// assert_eq!(Charset::Utf8.encoded_len("café"), 5);
    /// // A surrogate pair is two UTF-16 units, which is four bytes.
    /// assert_eq!(Charset::Utf16Le.encoded_len("a😀"), 6);
    /// ```
    #[must_use]
    pub fn encoded_len(self, input: &str) -> usize {
        match self {
            Self::Utf8 => input.len(),
            Self::Utf16Le | Self::Utf16Be => input.chars().map(char::len_utf16).sum::<usize>() * 2,
            // Every other charset here is one byte per scalar, so the count
            // is the scalar count and the ASCII prefix is already counted.
            _ => {
                let leading = ascii_len(input.as_bytes());
                leading + input[leading..].chars().count()
            }
        }
    }

    /// Encode complete text onto the end of `target`.
    ///
    /// # Errors
    ///
    /// Returns the same refusal as [`Charset::encode`]; `target` may already
    /// hold the bytes encoded before the failing scalar.
    pub fn encode_into(self, input: &str, target: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::Utf8 => {
                target.extend_from_slice(input.as_bytes());
                Ok(())
            }
            Self::Utf16Le => {
                unicode::utf16_encode_into::<false>(input, target);
                Ok(())
            }
            Self::Utf16Be => {
                unicode::utf16_encode_into::<true>(input, target);
                Ok(())
            }
            Self::Ascii => ascii::encode_into(input, target),
            _ => match self.table() {
                Some(table) => table.encode_into(input, target),
                None => {
                    target.extend_from_slice(input.as_bytes());
                    Ok(())
                }
            },
        }
    }

    /// Begin a chunked decode of this charset.
    ///
    /// A [`Decoder`] holds only the few bytes a chunk boundary can split a
    /// sequence across, so decoding a gigabyte costs one pass and three
    /// retained bytes.
    pub const fn decoder(self) -> Decoder {
        Decoder::new(self)
    }

    /// Wrap a reader so it yields UTF-8 bytes.
    ///
    /// Decoding is streaming: neither the encoded nor the decoded payload is
    /// buffered whole. [`Charset::Utf8`] transcodes nothing, so it returns the
    /// source unchanged and revalidates none of its bytes - the validating
    /// door is [`Charset::decode`], exactly as [`crate::Codec::Identity`]
    /// leaves bytes alone for codings.
    pub fn reader<'source, R: Read + 'source>(self, source: R) -> Box<dyn Read + 'source> {
        if self.is_utf8() {
            return Box::new(source);
        }
        Box::new(Reader::new(self, source))
    }

    /// Wrap a writer so UTF-8 bytes written to it are encoded in this charset.
    ///
    /// The returned writer must be finished with [`Writer::finish`]; dropping
    /// it leaves a scalar split across two writes unwritten.
    pub fn writer<'target, W: Write + 'target>(self, target: W) -> Writer<'target> {
        Writer::new(self, target)
    }

    /// How many trailing bytes of `input` begin a sequence it cuts short.
    ///
    /// A byte that is simply invalid is not pending: it answers zero so the
    /// decode reports it where it is, rather than waiting for a continuation
    /// that cannot fix it.
    pub(crate) fn pending(self, input: &[u8]) -> usize {
        match self {
            Self::Utf8 => unicode::utf8_pending(input),
            Self::Utf16Le => unicode::utf16_pending::<false>(input),
            Self::Utf16Be => unicode::utf16_pending::<true>(input),
            // One byte is one scalar, so a chunk boundary splits nothing.
            _ => 0,
        }
    }

    /// The generated table behind a single-byte charset.
    const fn table(self) -> Option<&'static SingleByte> {
        match self {
            Self::Latin1 => Some(&tables::LATIN1),
            Self::Latin2 => Some(&tables::LATIN2),
            Self::Latin9 => Some(&tables::LATIN9),
            Self::Cp1250 => Some(&tables::CP1250),
            Self::Cp1251 => Some(&tables::CP1251),
            Self::Cp1252 => Some(&tables::CP1252),
            Self::Cp437 => Some(&tables::CP437),
            Self::Cp850 => Some(&tables::CP850),
            Self::MacRoman => Some(&tables::MAC_ROMAN),
            Self::Utf8 | Self::Utf16Le | Self::Utf16Be | Self::Ascii => None,
        }
    }
}

/// The `charset` parameter of a `Content-Type` header value, unquoted.
fn content_type_charset(value: &str) -> Option<&str> {
    value.split(';').skip(1).find_map(|parameter| {
        let (name, declared) = parameter.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case(Charset::PARAMETER) {
            return None;
        }
        let declared = declared.trim();
        Some(
            declared
                .strip_prefix('"')
                .and_then(|inner| inner.strip_suffix('"'))
                .unwrap_or(declared),
        )
    })
}

/// A byte this charset does not decode.
pub(crate) fn undecodable(charset: &'static str, position: usize, byte: u8) -> Error {
    Error::Codec {
        format: charset,
        position,
        reason: format_smolstr!("expected a byte this charset assigns, got {byte:#04x}"),
    }
}

/// A scalar this charset cannot encode.
pub(crate) fn unencodable(charset: &'static str, position: usize, scalar: char) -> Error {
    Error::Codec {
        format: charset,
        position,
        reason: format_smolstr!(
            "expected a scalar this charset holds, got U+{:04X}",
            u32::from(scalar)
        ),
    }
}

/// A surrogate with no partner.
pub(crate) fn unpaired(charset: &'static str, position: usize, unit: u16) -> Error {
    Error::Codec {
        format: charset,
        position,
        reason: format_smolstr!("expected a surrogate pair, got the lone unit {unit:#06x}"),
    }
}

/// Input that stops inside a sequence.
pub(crate) fn truncated(charset: &'static str, position: usize, expected: &str) -> Error {
    Error::Codec {
        format: charset,
        position,
        reason: format_smolstr!("expected {expected}, got the end of the input"),
    }
}

impl FromStr for Charset {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|charset| normalized.eq_ignore_ascii_case(charset.as_str()))
            .or_else(|| {
                ALIASES
                    .iter()
                    .find(|(alias, _)| normalized.eq_ignore_ascii_case(alias))
                    .map(|(_, charset)| *charset)
            })
            .ok_or_else(|| Error::Parse {
                target: "charset",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, or a known alias, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|charset| charset.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

impl fmt::Display for Charset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Charset {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Charset {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        // Owned where the deserializer cannot lend, as a parsed JSON value
        // cannot, so a schema document reads through every door.
        let value = <Cow<'_, str>>::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests;
