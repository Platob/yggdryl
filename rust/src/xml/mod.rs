//! Natural XML values with bounded single-document I/O.
//!
//! An XML document proves no type: every leaf is text, and what it does
//! state is the element tree. The natural value is that tree, read the way
//! the widely used XML-to-object conventions read it:
//!
//! | XML | `Scalar` |
//! | --- | --- |
//! | the document | a record with one entry: the root element's name, and its value |
//! | `<a>text</a>` | `"text"` |
//! | `<a/>`, `<a xsi:nil="true"/>` | null |
//! | `<a></a>` | `""` - present, and empty |
//! | `<a k="v">text</a>` | `{"@k": "v", "#text": "text"}` |
//! | `<a><b>1</b><c>2</c></a>` | `{"b": "1", "c": "2"}` |
//! | `<a><b>1</b><b>2</b></a>` | `{"b": ["1", "2"]}` |
//!
//! An attribute is an [`ATTRIBUTE_PREFIX`]ed entry, an element's own text
//! beside attributes or children is the [`TEXT_KEY`] entry, and child
//! elements sharing one name are a sequence in document order. Names are
//! kept as the document spells them, prefix included, and a namespace
//! declaration is the `@xmlns` attribute it is written as. Whitespace-only
//! text between child elements is indentation and is dropped; every other
//! text is kept exactly, CDATA and character references resolved. Comments,
//! processing instructions and the document type declaration are skipped, and
//! an entity the declaration would have defined is refused by name, so no
//! document reaches anything outside its own bytes. A [`Field`] types the
//! natural strings ([`from_xml_scalar_with_field`]): text is trimmed and
//! empty text is null under any column that is not text, a child element
//! read once is the one item of a column declared as a sequence, and a child
//! element occurring no time is the empty sequence.
//!
//! Writing is the exact inverse: a record with one entry is the document
//! element, its `@` entries are attributes, its `#text` the element's own
//! text, a sequence under a key repeats that element, null is the self-closed
//! element and the empty text an element with an empty body. A nested
//! sequence, a sequence holding one null, a `#` key other than `#text`, a key
//! that is not an XML name, a temporal count with no ISO 8601 spelling and a
//! character XML 1.0 cannot carry are each refused by name. No declaration is
//! written: the transport writes one exactly where a reader would need it,
//! for a charset other than UTF-8.
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

use std::borrow::Borrow;
use std::io::{Read, Write};

pub mod element;
mod parser;
#[cfg(feature = "aws")]
pub(crate) mod scanner;
pub mod soap;
mod wire;

pub use element::{Element, Scope, XSD_NAMESPACE, XSI_NAMESPACE};
pub(crate) use wire::{
    is_name_char, is_name_start, natural, shaped, write_attribute_text, write_element_text,
    write_fragment, write_leaf_text,
};

use crate::text::{Formatting, Limits, Scalar, ScalarIter, check_input_size};
use crate::{Charset, Error, Field, Result};

/// The key an attribute is read into and written from: `<a id="7"/>` is
/// `{"@id": "7"}`.
pub const ATTRIBUTE_PREFIX: &str = "@";

/// The key an element's own text takes beside its attributes or children:
/// `<a id="7">text</a>` is `{"@id": "7", "#text": "text"}`.
pub const TEXT_KEY: &str = "#text";

/// The document element a stream of record columns is written under, each
/// row one child element named after the root field.
pub const DOCUMENT_ELEMENT: &str = "data";

/// Maximum structural nesting accepted by the XML parser and writer.
///
/// Caller limits may choose any smaller depth. The parser is iterative, so
/// this ceiling is what keeps a hostile document from costing memory rather
/// than stack; the writer recurses, and it is what keeps adversarial explicit
/// limits from turning nesting into stack exhaustion there.
pub const MAX_PARSER_DEPTH: usize = 384;

/// Decode one XML document from byte-like content into the shared `Scalar`:
/// a record with one entry, the root element's name and its value.
///
/// This is the inferring entry point over [`from_bytes`]: `input` may be
/// `&str`, `String`, `&[u8]`, `Vec<u8>` or any other byte-like value, and its
/// bytes are decoded there under default [`Limits`]. Inference is
/// deterministic - the input is always content, never a path - so text that
/// happens to name an existing file fails as the markup it is not rather
/// than being read. A caller who needs explicit limits calls
/// [`from_bytes_with_limits`].
///
/// ```
/// use yggdryl::{Scalar, from_xml_scalar, into_xml_scalar};
///
/// let value = from_xml_scalar("<order id=\"7\"><symbol>AAPL</symbol></order>")?;
/// let order = Scalar::from_struct([
///     ("@id", Scalar::from("7")),
///     ("symbol", Scalar::from("AAPL")),
/// ])?;
/// assert_eq!(value, Scalar::from_struct([("order", order)])?);
/// assert_eq!(into_xml_scalar(&value)?, "<order id=\"7\"><symbol>AAPL</symbol></order>");
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn from_xml_scalar(input: impl AsRef<[u8]>) -> Result<Scalar> {
    from_bytes(input.as_ref())
}

/// Decode XML content and interpret the document element's value under
/// `field`, answering the shared `Scalar`.
///
/// This is the inferring entry point over [`from_bytes_with_field`]: it
/// coerces `input` exactly as [`from_xml_scalar`] does - content, never a
/// path - and redirects, so `field` types the natural strings, orders the
/// record and validates it there under default [`Limits`]. The document
/// element's name is not read: a field names the value it holds, not the
/// document. Explicit limits go through [`from_bytes_with_field_and_limits`].
///
/// ```
/// use yggdryl::{DataType, Field, Scalar, StructType, from_xml_scalar_with_field};
///
/// let amount = Field::new("amount", DataType::decimal128(10, 2)?, false);
/// let field = Field::new("row", DataType::from(StructType::from_fields([amount])?), false);
/// let value = from_xml_scalar_with_field("<row><amount> 12.50 </amount></row>", &field)?;
/// assert_eq!(value, Scalar::from_sequence([Scalar::d128(1250, 2)]));
/// # Ok::<(), yggdryl::Error>(())
/// ```
pub fn from_xml_scalar_with_field(input: impl AsRef<[u8]>, field: &Field) -> Result<Scalar> {
    from_bytes_with_field(input.as_ref(), field)
}

/// Encode one document as XML UTF-8, naming the shared `Scalar` it takes.
///
/// This is the named entry point over [`into_utf8`], which it redirects to
/// unchanged; an indented layout goes through [`into_utf8_with_formatting`].
///
/// ```
/// use yggdryl::{Scalar, into_xml_scalar};
///
/// let row = Scalar::from_struct([("id", Scalar::from(1)), ("tags", Scalar::from_sequence([
///     Scalar::from("a"),
///     Scalar::from("b"),
/// ]))])?;
/// let value = Scalar::from_struct([("row", row)])?;
/// assert_eq!(into_xml_scalar(&value)?, "<row><id>1</id><tags>a</tags><tags>b</tags></row>");
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
    parser::parse(input, limits)
}

/// Decode one XML document from bytes.
///
/// The bytes are UTF-8, as every codec here reads them: the charset a
/// document declares is the transport's to honour, where
/// [`crate::text::from_io`] reads it off the declaration when neither the
/// handle's media type nor a byte order mark says otherwise.
pub fn from_bytes(input: &[u8]) -> Result<Scalar> {
    from_bytes_with_limits(input, Limits::default())
}

/// Decode one XML document from bytes with explicit limits.
pub fn from_bytes_with_limits(input: &[u8], limits: Limits) -> Result<Scalar> {
    check_input_size(input, limits, "xml")?;
    let input = std::str::from_utf8(input).map_err(|error| Error::Codec {
        format: "xml",
        position: error.valid_up_to(),
        reason: "input is not valid UTF-8".into(),
    })?;
    parser::parse(input, limits)
}

/// Decode XML UTF-8 and interpret the document element's value under
/// `field`.
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

/// Decode XML bytes and interpret the document element's value under
/// `field`.
pub fn from_bytes_with_field(input: &[u8], field: &Field) -> Result<Scalar> {
    from_bytes_with_field_and_limits(input, field, Limits::default())
}

/// Decode schema-directed XML bytes with explicit limits.
pub fn from_bytes_with_field_and_limits(
    input: &[u8],
    field: &Field,
    limits: Limits,
) -> Result<Scalar> {
    typed(from_bytes_with_limits(input, limits)?, field)
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
            format: "xml",
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
    typed(from_reader_with_limits(reader, limits)?, field)
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
    let root = Reader::with_limits(reader, limits)
        .map(move |document| document.map(|document| shaped(root_value(document), field)));
    ScalarIter::new(root).with_field(field)
}

/// The document element's value, typed by `field`.
///
/// This is what every field-directed door here does after parsing, and what
/// the format dispatch in [`crate::text`] does for XML in place of the bare
/// value contract, because a document is a record naming its root and states
/// less than a field does.
pub(crate) fn typed(document: Scalar, field: &Field) -> Result<Scalar> {
    field.from_natural_value(shaped(root_value(document), field))
}

/// The value of the one document element a parsed document holds.
///
/// A parsed document is a record with exactly one entry by construction, so
/// this never fails; a caller-built value goes through the writer, which
/// checks the shape for itself.
fn root_value(document: Scalar) -> Scalar {
    match document {
        Scalar::Struct(entries) => entries
            .as_map()
            .values()
            .next()
            .cloned()
            .unwrap_or(Scalar::Null),
        other => other,
    }
}

/// The charset an XML declaration names, when the document opens with one.
///
/// This is the transport's one bounded content read for XML: the declaration
/// is framing, and the charset it names is read where the handle's media type
/// and a byte order mark were read - and after both, because either of them
/// already answers. `head` is the first bytes of the document, at least the
/// declaration when there is one; a declaration cut short by the probe is
/// read up to the cut. A charset this crate does not have is refused by name.
///
/// # Errors
///
/// Returns [`Error::Parse`] naming the charset vocabulary when the
/// declaration names one the crate has no table for.
pub(crate) fn declared_charset(head: &[u8]) -> Result<Option<Charset>> {
    let Some(declaration) = head.strip_prefix(b"<?xml") else {
        return Ok(None);
    };
    let end = declaration
        .windows(2)
        .position(|pair| pair == b"?>")
        .unwrap_or(declaration.len());
    let declaration = &declaration[..end];
    let Some(at) = declaration
        .windows(8)
        .position(|window| window == b"encoding")
    else {
        return Ok(None);
    };
    let rest = &declaration[at + 8..];
    let rest = rest
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .map_or(&rest[rest.len()..], |skip| &rest[skip..]);
    let Some(rest) = rest.strip_prefix(b"=") else {
        return Ok(None);
    };
    let rest = rest
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .map_or(&rest[rest.len()..], |skip| &rest[skip..]);
    let Some((quote, rest)) = rest.split_first() else {
        return Ok(None);
    };
    if !matches!(quote, b'"' | b'\'') {
        return Ok(None);
    }
    let close = rest
        .iter()
        .position(|byte| byte == quote)
        .unwrap_or(rest.len());
    let name = std::str::from_utf8(&rest[..close]).map_err(|_| Error::Parse {
        target: "charset",
        position: at + b"<?xml".len(),
        reason: "the XML declaration names an encoding that is not ASCII".into(),
    })?;
    Charset::from_str(name).map(Some)
}

/// An owning lazy iterator that yields exactly one XML document.
pub struct Reader<R: Read> {
    inner: crate::text::DocumentReader<R>,
}

impl<R: Read> Reader<R> {
    /// Construct a single-document reader with default limits.
    pub fn new(reader: R) -> Self {
        Self::with_limits(reader, Limits::default())
    }

    /// Construct a single-document reader with explicit limits.
    pub const fn with_limits(reader: R, limits: Limits) -> Self {
        Self {
            inner: crate::text::DocumentReader::new(reader, limits, "xml", from_bytes_with_limits),
        }
    }

    /// Return the number of bytes pulled from the source.
    pub const fn byte_offset(&self) -> usize {
        self.inner.byte_offset()
    }
}

impl<R: Read> Iterator for Reader<R> {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<R: Read> ExactSizeIterator for Reader<R> {}

/// Encode one document as XML bytes.
pub fn into_bytes(value: &Scalar) -> Result<Vec<u8>> {
    into_bytes_with_formatting(value, Formatting::default())
}

/// Encode one document as XML bytes with explicit formatting.
///
/// [`Indent::Default`](crate::text::Indent::Default) and
/// [`Indent::None`](crate::text::Indent::None) are one line;
/// [`Indent::Spaces`](crate::text::Indent::Spaces) and
/// [`Indent::Tabs`](crate::text::Indent::Tabs) put each element on its own
/// line at its depth, except inside an element that carries `#text` beside
/// children, whose layout would become that text.
///
/// ```
/// use yggdryl::Scalar;
/// use yggdryl::text::Formatting;
///
/// # fn main() -> yggdryl::Result<()> {
/// let value = Scalar::from_struct([("row", Scalar::from_struct([("id", Scalar::from(1))])?)])?;
/// assert_eq!(yggdryl::xml::into_bytes(&value)?, b"<row><id>1</id></row>");
/// assert_eq!(
///     yggdryl::xml::into_bytes_with_formatting(&value, Formatting::indented(2))?,
///     b"<row>\n  <id>1</id>\n</row>",
/// );
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns the encoder's failure, including the published depth cap.
pub fn into_bytes_with_formatting(value: &Scalar, formatting: Formatting) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_with_formatting(value, &mut output, formatting)?;
    Ok(output)
}

/// Encode one document as XML UTF-8.
pub fn into_utf8(value: &Scalar) -> Result<String> {
    into_utf8_with_formatting(value, Formatting::default())
}

/// Encode one document as XML UTF-8 with explicit formatting.
pub fn into_utf8_with_formatting(value: &Scalar, formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_with_formatting(value, formatting)?).map_err(|error| {
        Error::Codec {
            format: "xml",
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
///
/// The one walk the writer makes, into no output: every refusal the writer
/// has - the document element, a nested sequence, a name that is not one, a
/// character XML cannot carry, the depth - is found here first, so a
/// destination is never opened for a value that cannot be written whole.
pub fn validate_for_write_with_limits(value: &Scalar, limits: Limits) -> Result<()> {
    wire::write_document(
        &mut std::io::sink(),
        value,
        Formatting::default().into(),
        limits.max_depth(),
    )
}

/// Encode one document to a byte writer.
pub fn into_writer<W: Write>(value: &Scalar, writer: W) -> Result<()> {
    into_writer_with_formatting(value, writer, Formatting::default())
}

/// Encode one document to a byte writer with explicit formatting.
///
/// # Errors
///
/// Returns the encoder's or the sink's failure.
pub fn into_writer_with_formatting<W: Write>(
    value: &Scalar,
    mut writer: W,
    formatting: Formatting,
) -> Result<()> {
    wire::write_document(
        &mut writer,
        value,
        formatting.into(),
        Limits::default().max_depth(),
    )
}

/// Encode exactly one document as XML bytes.
pub fn into_bytes_all(values: &[Scalar]) -> Result<Vec<u8>> {
    into_bytes_all_with_formatting(values, Formatting::default())
}

/// Encode exactly one document as XML bytes with explicit formatting.
pub fn into_bytes_all_with_formatting(
    values: &[Scalar],
    formatting: Formatting,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    into_writer_all_with_formatting(values, &mut output, formatting)?;
    Ok(output)
}

/// Encode exactly one document as XML UTF-8.
pub fn into_utf8_all(values: &[Scalar]) -> Result<String> {
    into_utf8_all_with_formatting(values, Formatting::default())
}

/// Encode exactly one document as XML UTF-8 with explicit formatting.
pub fn into_utf8_all_with_formatting(values: &[Scalar], formatting: Formatting) -> Result<String> {
    String::from_utf8(into_bytes_all_with_formatting(values, formatting)?).map_err(|error| {
        Error::Codec {
            format: "xml",
            position: error.utf8_error().valid_up_to(),
            reason: "encoded XML is not valid UTF-8".into(),
        }
    })
}

/// Encode exactly one document to an XML writer.
pub fn into_writer_all<W, I, V>(values: I, writer: W) -> Result<()>
where
    W: Write,
    I: IntoIterator<Item = V>,
    V: Borrow<Scalar>,
{
    into_writer_all_with_formatting(values, writer, Formatting::default())
}

/// Encode exactly one document to an XML writer with explicit formatting.
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
        format: "xml",
        position: 0,
        reason: "expected exactly one value for an XML document".into(),
    })?;
    if values.next().is_some() {
        return Err(Error::Codec {
            format: "xml",
            position: 0,
            reason: "XML does not support multiple documents".into(),
        });
    }
    into_writer_with_formatting(value.borrow(), writer, formatting)
}
