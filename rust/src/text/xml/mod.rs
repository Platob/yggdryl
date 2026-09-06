//! Natural XML documents with bounded single-document I/O.
//!
//! The explicit representation forms carry the implementation: `from_utf8`,
//! `from_bytes` and `from_reader` with their `_all`, `_with_field` and
//! `_with_limits` modifiers, and `into_utf8`, `into_bytes` and `into_writer`
//! with `_with_formatting`. [`from_xml_scalar`], [`from_xml_scalar_with_field`]
//! and [`into_xml_scalar`] are the one inferring boundary over them, not
//! aliases: each names the `Scalar` it answers, coerces any byte-like input at
//! the boundary and redirects to its explicit form, holding no parsing,
//! rendering, validation or limits logic of its own.
//!
//! # The mapping
//!
//! XML carries three things a record does not, so three rules cover the whole
//! translation and nothing else is invented:
//!
//! - A document is the one-entry record its document element names, so the
//!   root's name survives a round trip without an envelope around it.
//! - An attribute is a field whose name carries [`ATTRIBUTE_PREFIX`], and an
//!   element's own character data beside child elements is the field
//!   [`TEXT_KEY`]. Neither spelling is a valid XML name, so neither can
//!   collide with a child element.
//! - A child element name that occurs once is that value and a name that
//!   occurs more than once is a sequence in document order, because the wire
//!   spells a list by repeating the element rather than by nesting one.
//!
//! An element holding nothing is [`Scalar::Null`], and an element holding only
//! text is that text. Layout is not content: whitespace-only character data is
//! dropped and an element's own text loses its outer whitespace, so an
//! indented document reads as the values it spells. A CDATA section is the
//! exception it exists to be - an element holding one keeps its character data
//! exactly.
//!
//! A declared [`Field`] types what the document element holds rather than the
//! name the wire gave it, so a row reads as that row whatever its element is
//! called and the field's own names decide the columns.
//!
//! Comments, processing instructions and the document type declaration are
//! annotations rather than content, exactly as a YAML tag is, and are ignored.
//! An entity the declaration would define therefore has no definition to
//! expand: only character references and the five entities XML predefines
//! resolve, and any other reference is an error naming it.
//!
//! ```
//! use yggdryl::{Scalar, from_xml_scalar, into_xml_scalar};
//!
//! let value = from_xml_scalar("<trade id='7'><symbol>AAPL</symbol></trade>")?;
//! assert_eq!(
//!     value,
//!     Scalar::from_record([(
//!         "trade",
//!         Scalar::from_record([
//!             ("@id", Scalar::from("7")),
//!             ("symbol", Scalar::from("AAPL")),
//!         ])?,
//!     )])?,
//! );
//! assert_eq!(
//!     into_xml_scalar(&value)?,
//!     r#"<trade id="7"><symbol>AAPL</symbol></trade>"#,
//! );
//! # Ok::<(), yggdryl::Error>(())
//! ```

use std::borrow::Borrow;
use std::io::{Read, Write};

pub(crate) mod parser;
pub(crate) mod wire;

use crate::text::{Formatting, Limits, Scalar, ScalarIter, check_input_size};
use crate::types::Nested;
use crate::{Error, Field, Result};

/// The name this codec reports in its errors.
pub(crate) const FORMAT: &str = "xml";

/// The prefix an attribute's field name carries.
///
/// `@` cannot start an XML name, so an attribute field and a child element
/// field can never collide.
pub const ATTRIBUTE_PREFIX: char = '@';

/// The field name an element's own character data is stored under.
///
/// `#text` is not a valid XML name, so this field and a child element field
/// can never collide.
pub const TEXT_KEY: &str = "#text";

/// Maximum element nesting accepted by the XML parser.
///
/// Caller limits may choose any smaller depth. The reader keeps its open
/// elements on its own stack rather than on the native one, so this ceiling
/// bounds memory rather than recursion.
pub const MAX_PARSER_DEPTH: usize = 1_024;

/// Decode one XML document from byte-like content into the shared `Scalar`, a
/// `Record` naming the document element.
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
/// assert_eq!(from_xml_scalar(into_xml_scalar(&value)?)?, value);
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn from_xml_scalar(input: impl AsRef<[u8]>) -> Result<Scalar> {
    from_bytes(input.as_ref())
}

/// Decode XML content and interpret the natural value under `field`,
/// answering the shared `Scalar`.
///
/// This is the inferring entry point over [`from_bytes_with_field`]: it
/// coerces `input` exactly as [`from_xml_scalar`] does - content, never a path
/// - and redirects, so `field` types the character data XML carries, orders
/// the element into the field's declaration order and validates there under
/// default [`Limits`]. Explicit limits go through
/// [`from_bytes_with_field_and_limits`].
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

/// Encode one value as an XML document, naming the shared `Scalar` it takes.
///
/// This is the named entry point over [`into_utf8`], which it redirects to
/// unchanged; an indented layout goes through [`into_utf8_with_formatting`].
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
    check_input_size(input.as_bytes(), limits, FORMAT)?;
    parser::parse(input, limits)
}

/// Decode one XML document from bytes.
pub fn from_bytes(input: &[u8]) -> Result<Scalar> {
    from_bytes_with_limits(input, Limits::default())
}

/// Decode one XML document from bytes with explicit limits.
///
/// The bytes are UTF-8: an XML declaration naming another encoding is an
/// error rather than a transcoding this codec performs silently.
pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Scalar> {
    check_input_size(input, limits, FORMAT)?;
    let input = std::str::from_utf8(input).map_err(|error| Error::Codec {
        format: FORMAT,
        position: error.valid_up_to(),
        reason: "input is not valid UTF-8".into(),
    })?;
    parser::parse(input.strip_prefix('\u{feff}').unwrap_or(input), limits)
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
    from_bytes_with_field_and_limits(input.as_bytes(), field, limits)
}

/// Decode XML bytes and interpret the natural value under `field`.
pub fn from_bytes_with_field(input: &[u8], field: &Field) -> Result<Scalar> {
    from_bytes_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed XML bytes with explicit limits.
pub fn from_bytes_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    field.from_natural_value(content(from_bytes_with_limits(input, limits)?))
}

/// Return the value the document element holds.
///
/// Every value this codec decodes is the one-entry record naming the document
/// element, so a declared field types what that element *holds* rather than
/// the name the wire gave it: `<trade><id>7</id></trade>` read under a row
/// field is that row, and `trade` is layout the field replaces. A fieldless
/// read keeps the name, because nothing else would.
pub(crate) fn content(document: Scalar) -> Scalar {
    let Scalar::Nested(Nested::Record(entries)) = &document else {
        return document;
    };
    let entries = entries.as_map();
    if entries.len() != 1 {
        return document;
    }
    entries
        .values()
        .next()
        .cloned()
        .unwrap_or(document)
}

/// Decode one XML document from a byte reader.
pub fn from_reader<R: Read>(reader: R) -> Result<Scalar> {
    from_reader_with_limits(reader, Limits::default())
}

/// Decode one XML document from a reader with explicit limits.
pub fn from_reader_with_limits<R: Read>(reader: R, limits: Limits) -> Result<Scalar> {
    let mut reader = Reader::with_limits(reader, limits);
    reader.next().unwrap_or_else(|| {
        Err(Error::Codec {
            format: FORMAT,
            position: 0,
            reason: "XML reader did not yield its document".into(),
        })
    })
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
    field.from_natural_value(content(from_reader_with_limits(reader, limits)?))
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
    ScalarIter::new(
        Reader::with_limits(reader, limits)
            .map(move |value| field.from_natural_value(content(value?))),
    )
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
            return Some(Err(Error::Codec {
                format: FORMAT,
                position: 0,
                reason: "document limit exceeded".into(),
            }));
        }
        let maximum = self.limits.max_input_bytes();
        let mut reader = reader.take(u64::try_from(maximum.saturating_add(1)).unwrap_or(u64::MAX));
        let mut input = Vec::with_capacity(maximum.min(8 * 1024));
        let result = reader.read_to_end(&mut input).map_err(Error::from);
        self.byte_offset = input.len();
        Some(result.and_then(|_| {
            if input.len() > maximum {
                return Err(crate::text::input_too_large(FORMAT, maximum));
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
        Error::Codec {
            format: FORMAT,
            position: error.utf8_error().valid_up_to(),
            reason: "encoded XML is not valid UTF-8".into(),
        }
    })
}

/// Validate the natural XML projection before opening a destination.
pub fn validate_for_write(value: &Scalar) -> Result<()> {
    validate_for_write_with_limits(value, Limits::default())
}

/// Validate the natural XML projection against explicit limits.
pub fn validate_for_write_with_limits(value: &Scalar, limits: Limits) -> Result<()> {
    wire::check_depth(value, limits.max_depth())
}

/// Encode one value to a byte writer.
pub fn into_writer<W: Write>(value: &Scalar, writer: W) -> Result<()> {
    into_writer_with_formatting(value, writer, Formatting::default())
}

/// Encode one value to a byte writer with explicit formatting.
///
/// The document element is written on its own: no declaration, no document
/// type, and no trailing newline, so the bytes are exactly the value and
/// nothing around it.
pub fn into_writer_with_formatting<W: Write>(
    value: &Scalar,
    mut writer: W,
    formatting: Formatting,
) -> Result<()> {
    validate_for_write(value)?;
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
        Error::Codec {
            format: FORMAT,
            position: error.utf8_error().valid_up_to(),
            reason: "encoded XML is not valid UTF-8".into(),
        }
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
    let value = values.next().ok_or_else(|| Error::Codec {
        format: FORMAT,
        position: 0,
        reason: "expected exactly one value for an XML document".into(),
    })?;
    if values.next().is_some() {
        return Err(Error::Codec {
            format: FORMAT,
            position: 0,
            reason: "XML has one document element, so it holds one value".into(),
        });
    }
    into_writer_with_formatting(value.borrow(), writer, formatting)
}
