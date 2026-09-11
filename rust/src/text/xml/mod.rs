//! Natural XML values with bounded single-document I/O.
//!
//! The explicit representation forms carry the implementation: `from_utf8`,
//! `from_bytes` and `from_reader` with their `_all`, `_with_field` and
//! `_with_limits` modifiers, and `into_utf8`, `into_bytes` and `into_writer`
//! with `_with_formatting`. [`from_xml_scalar`], [`from_xml_scalar_with_field`]
//! and [`into_xml_scalar`] are the one inferring boundary over them, not
//! aliases: each names the `Scalar` it answers, coerces any byte-like input at
//! the boundary and redirects to its explicit form, holding no parsing,
//! rendering, validation or limits logic of its own. Deleting them as
//! duplicates would remove the only entry point that names the `Scalar`.
//!
//! # The shape a document has
//!
//! A document is one root element, so a decoded document is a one-entry record
//! keyed by that element's qualified name. Inside it:
//!
//! | XML | `Scalar` |
//! | --- | --- |
//! | `<a>text</a>` | the text, under `a` |
//! | `<a/>`, `<a></a>` | [`Scalar::Null`], under `a` |
//! | `<a id="1">text</a>` | a record of `@id` and `#text` |
//! | `<a><b/><c/></a>` | a record of `b` and `c` |
//! | `<a><b/><b/></a>` | a sequence of two, under `b` |
//! | `<a>text<b/></a>` | refused: mixed content orders nothing |
//!
//! An attribute is keyed by its name behind an `@`, and character data beside
//! attributes is keyed `#text`; neither collides with an element name, because
//! no XML name may start with either character. A prefixed name - `ns:total`,
//! `xmlns:ns` - is kept exactly as written, so namespaces survive a round trip
//! without this layer resolving any of them.
//!
//! Every leaf is character data, so a schema-free read answers text and
//! nothing else: XML has no number, boolean or date grammar to prove one with.
//! A [`Field`] is what restores exact types, and it reads the same spellings
//! every other structured format writes.
//!
//! ```
//! use yggdryl::{DataType, Field, Scalar, from_xml_scalar, from_xml_scalar_with_field};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let document = "<trade><symbol>AAPL</symbol><size>100</size></trade>";
//!
//! // Without a field, a document proves text.
//! let natural = from_xml_scalar(document)?;
//! assert_eq!(
//!     natural.get_key_str("trade").and_then(|trade| trade.get_key_str("size")),
//!     Some(&Scalar::from("100")),
//! );
//!
//! // With one, the field names the root and types every leaf under it.
//! let field = Field::from_str("trade: struct<symbol: utf8, size: int64> not null")?;
//! let typed = from_xml_scalar_with_field(document, &field)?;
//! assert_eq!(typed, Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]));
//! # Ok(())
//! # }
//! ```
//!
//! # What XML cannot say
//!
//! `<a/>` and `<a></a>` are one document, so an element with no character data
//! is absence, and a schema-free read of an empty string answers
//! [`Scalar::Null`]. A document also has no framing around a repeated element,
//! so one occurrence is one value, and no spelling at all for an empty list.
//!
//! A declared [`Field`] is what reads those back, and the only thing a shape
//! is ever read from here: it names the root element, takes one occurrence of
//! a repeated element as a one-item list, takes no occurrence as the empty
//! list where a list may not be absent, and takes an empty element as the
//! empty text or byte value where that field may not be absent either. An
//! element the document leaves out entirely is still absence.

use std::borrow::Borrow;
use std::io::{Read, Write};

mod parser;
mod schema;
mod wire;

use smol_str::SmolStr;

use crate::text::wire::from_raw;
use crate::text::{Formatting, Limits, Scalar, ScalarIter, check_input_size};
use crate::{Error, Field, Result};

/// Maximum element nesting accepted by the XML parser.
///
/// Caller limits may choose any smaller depth. This implementation ceiling
/// keeps adversarial explicit limits from turning nesting into stack
/// exhaustion while the parsed document is decoded.
pub const MAX_PARSER_DEPTH: usize = 384;

/// The character an attribute's name is keyed behind.
///
/// No XML name may start with it, so an attribute can never collide with a
/// child element of the same name.
pub const ATTRIBUTE_PREFIX: char = '@';

/// The key an element's own character data is stored under.
///
/// No XML name may start with `#`, so this can never collide with a child
/// element's name.
pub const TEXT_KEY: &str = "#text";

/// Decode one XML document from byte-like content into the shared `Scalar`,
/// a `Record` of one entry because a document is one root element.
///
/// This is the inferring entry point over [`from_bytes`]: `input` may be
/// `&str`, `String`, `&[u8]`, `Vec<u8>` or any other byte-like value, and its
/// bytes are decoded there under default [`Limits`]. Inference is
/// deterministic - the input is always content, never a path - so text that
/// happens to name an existing file fails as the markup it is not rather than
/// being read. A caller who needs explicit limits calls
/// [`from_bytes_with_limits`].
///
/// ```
/// use yggdryl::{Scalar, from_xml_scalar, into_xml_scalar};
///
/// let value = from_xml_scalar("<row><id>1</id></row>")?;
/// assert_eq!(
///     value,
///     Scalar::from_record([("row", Scalar::from_record([("id", Scalar::from("1"))])?)])?,
/// );
/// assert_eq!(into_xml_scalar(&value)?, "<row><id>1</id></row>");
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn from_xml_scalar(input: impl AsRef<[u8]>) -> Result<Scalar> {
    from_bytes(input.as_ref())
}

/// Decode XML content and interpret the natural value under `field`,
/// answering the shared `Scalar`.
///
/// This is the inferring entry point over [`from_bytes_with_field`]: it
/// coerces `input` exactly as [`from_xml_scalar`] does, as content rather than
/// as a path, and redirects, so `field` names the root element, types every
/// leaf's character data and orders the record there under default [`Limits`].
/// Explicit limits go through [`from_bytes_with_field_and_limits`].
///
/// ```
/// use yggdryl::{DataType, Field, Scalar, from_xml_scalar_with_field};
///
/// let amount = Field::new("amount", DataType::decimal128(10, 2)?, false);
/// let field = Field::new("row", DataType::from_fields([amount])?, false);
/// let value = from_xml_scalar_with_field("<row><amount>12.50</amount></row>", &field)?;
/// assert_eq!(value, Scalar::from_sequence([Scalar::d128(1250, 2)]));
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn from_xml_scalar_with_field(input: impl AsRef<[u8]>, field: &Field) -> Result<Scalar> {
    from_bytes_with_field(input.as_ref(), field)
}

/// Encode one root-named value as XML UTF-8, naming the shared `Scalar` it
/// takes.
///
/// This is the named entry point over [`into_utf8`], which it redirects to
/// unchanged; another layout goes through [`into_utf8_with_formatting`].
///
/// ```
/// use yggdryl::{Scalar, into_xml_scalar};
///
/// let value = Scalar::from_record([("row", Scalar::from_record([("id", Scalar::from(1))])?)])?;
/// assert_eq!(into_xml_scalar(&value)?, "<row><id>1</id></row>");
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn into_xml_scalar(value: &Scalar) -> Result<String> {
    into_utf8(value)
}

/// Decode one XML document from UTF-8.
pub fn from_utf8(input: &str) -> Result<Scalar> {
    from_utf8_with_limits(input, Limits::default())
}

/// Decode one XML document from UTF-8 with explicit limits.
pub fn from_utf8_with_limits(input: &str, limits: Limits) -> Result<Scalar> {
    check_input_size(input.as_bytes(), limits, "xml")?;
    from_raw(parser::parse(input, limits)?, limits, "xml")
}

/// Decode one XML document from bytes.
pub fn from_bytes(input: &[u8]) -> Result<Scalar> {
    from_bytes_with_limits(input, Limits::default())
}

/// Decode one XML document from bytes with explicit limits.
pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Scalar> {
    check_input_size(input, limits, "xml")?;
    from_utf8_with_limits(decoded(input)?, limits)
}

/// Decode XML UTF-8 and interpret its natural value under `field`.
pub fn from_utf8_with_field(input: &str, field: &Field) -> Result<Scalar> {
    from_utf8_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed XML UTF-8 with explicit limits.
pub fn from_utf8_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    with_field(from_utf8_with_limits(input, limits)?, field)
}

/// Decode XML bytes and interpret their natural value under `field`.
pub fn from_bytes_with_field(input: &[u8], field: &Field) -> Result<Scalar> {
    from_bytes_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed XML bytes with explicit limits.
pub fn from_bytes_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    with_field(from_bytes_with_limits(input, limits)?, field)
}

/// Decode one XML document from a byte reader.
pub fn from_reader<R: Read>(reader: R) -> Result<Scalar> {
    from_reader_with_limits(reader, Limits::default())
}

/// Decode one XML document from a reader with explicit limits.
pub fn from_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Scalar> {
    let mut reader = Reader::with_limits(reader, limits);
    reader
        .next()
        .unwrap_or_else(|| Err(codec_error(0, "XML reader did not yield its document")))
}

/// Decode XML from a reader under `field`.
pub fn from_reader_with_field<R: Read>(reader: R, field: &Field) -> Result<Scalar> {
    from_reader_with_field_and_limits(reader, field, Limits::default())
}

/// Decode schema-directed XML from a reader with explicit limits.
pub fn from_reader_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    with_field(from_reader_with_limits(reader, limits)?, field)
}

/// Decode the single XML document as a one-element collection.
pub fn from_utf8_all(input: &str) -> Result<Vec<Scalar>> {
    from_utf8_all_with_limits(input, Limits::default())
}

/// Decode the single XML UTF-8 document with explicit limits.
pub fn from_utf8_all_with_limits(input: &str, limits: Limits) -> Result<Vec<Scalar>> {
    from_utf8_with_limits(input, limits).map(|value| vec![value])
}

/// Decode the single XML byte document as a one-element collection.
pub fn from_bytes_all(input: &[u8]) -> Result<Vec<Scalar>> {
    from_bytes_all_with_limits(input, Limits::default())
}

/// Decode the single XML byte document with explicit limits.
pub fn from_bytes_all_with_limits(input: &[u8], limits: Limits) -> Result<Vec<Scalar>> {
    from_bytes_with_limits(input, limits).map(|value| vec![value])
}

/// Decode the XML UTF-8 document under `field`.
pub fn from_utf8_all_with_field(input: &str, field: &Field) -> Result<Vec<Scalar>> {
    from_utf8_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed XML UTF-8 with explicit limits.
pub fn from_utf8_all_with_field_and_limits(
    input: &str,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    from_utf8_with_field_and_limits(input, field, limits).map(|value| vec![value])
}

/// Decode the XML byte document under `field`.
pub fn from_bytes_all_with_field(input: &[u8], field: &Field) -> Result<Vec<Scalar>> {
    from_bytes_all_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed XML bytes with explicit limits.
pub fn from_bytes_all_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    from_bytes_with_field_and_limits(input, field, limits).map(|value| vec![value])
}

/// Decode the reader's XML document as a one-element collection.
pub fn from_reader_all<R: Read>(reader: R) -> Result<Vec<Scalar>> {
    from_reader_all_with_limits(reader, Limits::default())
}

/// Decode the reader's XML document with explicit limits.
pub fn from_reader_all_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Vec<Scalar>> {
    from_reader_with_limits(reader, limits).map(|value| vec![value])
}

/// Decode the reader's XML document under `field`.
pub fn from_reader_all_with_field<R: Read>(reader: R, field: &Field) -> Result<Vec<Scalar>> {
    from_reader_all_with_field_and_limits(reader, field, Limits::default())
}

/// Decode schema-directed XML from a reader with explicit limits.
pub fn from_reader_all_with_field_and_limits<R: Read>(
    reader: R,
    field: &Field,
    limits: Limits,
) -> Result<Vec<Scalar>> {
    from_reader_with_field_and_limits(reader, field, limits).map(|value| vec![value])
}

/// Lazily decode the one XML document from a borrowed reader.
pub fn from_reader_iter<'a, R: Read + 'a>(reader: &'a mut R) -> ScalarIter<'a> {
    from_reader_iter_with_limits(reader, Limits::default())
}

/// Lazily decode the one XML document with explicit limits.
pub fn from_reader_iter_with_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    limits: Limits,
) -> ScalarIter<'a> {
    ScalarIter::new(Reader::with_limits(reader, limits))
}

/// Lazily decode the XML document under `field`.
pub fn from_reader_iter_with_field<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
) -> ScalarIter<'a> {
    from_reader_iter_with_field_and_limits(reader, field, Limits::default())
}

/// Lazily decode schema-directed XML with explicit limits.
pub fn from_reader_iter_with_field_and_limits<'a, R: Read + 'a>(
    reader: &'a mut R,
    field: &'a Field,
    limits: Limits,
) -> ScalarIter<'a> {
    ScalarIter::new(Reader::with_limits(reader, limits)).with_field(field, crate::text::Format::Xml)
}

/// An owning lazy iterator that yields exactly one XML document.
pub struct Reader<R: Read> {
    reader: Option<R>,
    limits: Limits,
    byte_offset: usize,
}

impl<R: Read> Reader<R> {
    /// Construct a single-document reader with default limits.
    pub fn new(reader: R) -> Self {
        Self::with_limits(reader, Limits::default())
    }

    /// Construct a single-document reader with explicit limits.
    pub const fn with_limits(reader: R, limits: Limits) -> Self {
        Self {
            reader: Some(reader),
            limits,
            byte_offset: 0,
        }
    }

    /// Return the number of bytes pulled from the source.
    pub const fn byte_offset(&self) -> usize {
        self.byte_offset
    }
}

impl<R: Read> Iterator for Reader<R> {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        let reader = self.reader.take()?;
        if self.limits.max_documents() == 0 {
            return Some(Err(codec_error(0, "document limit exceeded")));
        }
        let maximum = self.limits.max_input_bytes();
        let mut reader = reader.take(u64::try_from(maximum.saturating_add(1)).unwrap_or(u64::MAX));
        let mut input = Vec::with_capacity(maximum.min(8 * 1024));
        let result = reader.read_to_end(&mut input).map_err(Error::from);
        self.byte_offset = input.len();
        Some(result.and_then(|_| {
            if input.len() > maximum {
                return Err(crate::text::input_too_large("xml", maximum));
            }
            from_bytes_with_limits(&input, self.limits)
        }))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::from(self.reader.is_some());
        (remaining, Some(remaining))
    }
}

impl<R: Read> ExactSizeIterator for Reader<R> {}

/// Encode one value as XML bytes.
pub fn into_bytes(value: &Scalar) -> Result<Vec<u8>> {
    into_bytes_with_formatting(value, Formatting::default())
}

/// Encode one value as XML bytes with explicit formatting.
pub fn into_bytes_with_formatting(value: &Scalar, formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_with_formatting(value, &mut output, formatting)?;
    Ok(output)
}

/// Encode one value as XML UTF-8.
pub fn into_utf8(value: &Scalar) -> Result<String> {
    into_utf8_with_formatting(value, Formatting::default())
}

/// Encode one value as XML UTF-8 with explicit formatting.
pub fn into_utf8_with_formatting(value: &Scalar, formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_with_formatting(value, formatting)?).map_err(|error| {
        codec_error_at(
            error.utf8_error().valid_up_to(),
            "encoded XML is not valid UTF-8",
        )
    })
}

/// Validate the natural XML projection before opening a destination.
pub fn validate_for_write(value: &Scalar) -> Result<()> {
    validate_for_write_with_limits(value, Limits::default())
}

/// Validate the natural XML projection against explicit limits.
///
/// The document is rendered into a sink, so every refusal is the writer's own
/// rather than a second opinion about what XML can carry.
pub fn validate_for_write_with_limits(value: &Scalar, limits: Limits) -> Result<()> {
    crate::text::check_encode_depth_to(value, "xml", limits.max_depth())?;
    wire::write_document(&mut std::io::sink(), value, Formatting::default().into())
}

/// Encode one value to a byte writer.
pub fn into_writer<W: Write>(value: &Scalar, writer: W) -> Result<()> {
    into_writer_with_formatting(value, writer, Formatting::default())
}

/// Encode one value to a byte writer with explicit formatting.
pub fn into_writer_with_formatting<W: Write>(
    value: &Scalar,
    mut writer: W,
    formatting: Formatting,
) -> Result<()> {
    crate::text::check_encode_depth(value, "xml")?;
    wire::write_document(&mut writer, value, formatting.into())
}

/// Encode exactly one value as XML bytes.
pub fn into_bytes_all(values: &[Scalar]) -> Result<Vec<u8>> {
    into_bytes_all_with_formatting(values, Formatting::default())
}

/// Encode exactly one value as XML bytes with explicit formatting.
pub fn into_bytes_all_with_formatting(
    values: &[Scalar],
    formatting: Formatting,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_all_with_formatting(values, &mut output, formatting)?;
    Ok(output)
}

/// Encode exactly one value as XML UTF-8.
pub fn into_utf8_all(values: &[Scalar]) -> Result<String> {
    into_utf8_all_with_formatting(values, Formatting::default())
}

/// Encode exactly one value as XML UTF-8 with explicit formatting.
pub fn into_utf8_all_with_formatting(values: &[Scalar], formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_all_with_formatting(values, formatting)?).map_err(|error| {
        codec_error_at(
            error.utf8_error().valid_up_to(),
            "encoded XML is not valid UTF-8",
        )
    })
}

/// Encode exactly one value to an XML writer.
pub fn into_writer_all<W, I, V>(values: I, writer: W) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Scalar>,
{
    into_writer_all_with_formatting(values, writer, Formatting::default())
}

/// Encode exactly one value to an XML writer with explicit formatting.
pub fn into_writer_all_with_formatting<W, I, V>(
    values: I,
    writer: W,
    formatting: Formatting,
) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Scalar>,
{
    let mut values = values.into_iter();
    let value = values
        .next()
        .ok_or_else(|| codec_error(0, "expected exactly one value for an XML document"))?;
    if values.next().is_some() {
        return Err(codec_error(0, "XML does not support multiple documents"));
    }
    into_writer_with_formatting(value.borrow(), writer, formatting)
}

/// Interpret one decoded document under `field`.
pub(crate) fn with_field(document: Scalar, field: &Field) -> Result<Scalar> {
    field.from_natural_value(schema::shaped_document(document, field)?)
}

/// Interpret one decoded value that is already a row under `field`.
pub(crate) fn with_field_row(value: Scalar, field: &Field) -> Result<Scalar> {
    field.from_natural_value(schema::shaped(value, field)?)
}

/// The UTF-8 an XML document must be, named where it stops being one.
fn decoded(input: &[u8]) -> Result<&str> {
    std::str::from_utf8(input)
        .map_err(|error| codec_error_at(error.valid_up_to(), "input is not valid UTF-8"))
}

fn codec_error(position: usize, reason: impl Into<SmolStr>) -> Error {
    Error::Codec {
        format: "xml",
        position,
        reason: reason.into(),
    }
}

fn codec_error_at(position: usize, reason: &'static str) -> Error {
    codec_error(position, reason)
}
