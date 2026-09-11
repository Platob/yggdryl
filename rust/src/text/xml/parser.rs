//! One XML document, read as the parser-neutral syntax nodes.
//!
//! The pull parser is quick-xml's; everything below it is the shape decision
//! this module owns. An element is read exactly once, into the group it
//! belongs to, and nothing is re-read or re-decided afterwards.

use quick_xml::Reader;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesDecl, BytesStart, Event};

use smol_str::format_smolstr;

use crate::text::wire::RawValue;
use crate::text::{ERROR_TEXT_LIMIT, Limits, elide_to};
use crate::{Error, Result};

use super::{ATTRIBUTE_PREFIX, MAX_PARSER_DEPTH, TEXT_KEY, codec_error};

/// One element being read, holding only what its own events proved.
struct Element {
    /// The element's qualified name, which keys it in its parent.
    name: String,
    /// Attributes in document order, already keyed with [`ATTRIBUTE_PREFIX`].
    attributes: Vec<(String, String)>,
    /// Child elements grouped by qualified name, in first-appearance order.
    children: Vec<(String, Vec<RawValue>)>,
    /// The character data read directly inside this element.
    text: String,
    /// Whether any character data was read at all, which `<a/>` has not.
    has_text: bool,
    /// Where the element started, so its own refusals are located there.
    position: usize,
}

impl Element {
    fn new(name: String, position: usize) -> Self {
        Self {
            name,
            attributes: Vec::new(),
            children: Vec::new(),
            text: String::new(),
            has_text: false,
            position,
        }
    }

    /// File one finished child under its name, keeping document order.
    fn push_child(&mut self, name: String, value: RawValue) {
        if let Some((_, values)) = self
            .children
            .iter_mut()
            .find(|(existing, _)| *existing == name)
        {
            values.push(value);
            return;
        }
        self.children.push((name, vec![value]));
    }

    /// Answer the value this element's own events proved.
    ///
    /// A leaf is its character data, an empty element is absence, and anything
    /// carrying attributes or children is the mapping of both. Character data
    /// beside child elements is mixed content, which no natural value orders,
    /// so it is refused rather than silently dropped.
    fn finish(self) -> Result<(String, RawValue)> {
        let has_children = !self.children.is_empty();
        if has_children && !is_xml_space(&self.text) {
            return Err(codec_error(
                self.position,
                "an element holds character data or child elements, not both",
            ));
        }
        if !has_children && self.attributes.is_empty() {
            let value = if self.has_text {
                RawValue::String(self.text)
            } else {
                RawValue::Null
            };
            return Ok((self.name, value));
        }

        let mut entries = Vec::with_capacity(
            self.attributes.len() + self.children.len() + usize::from(self.has_text),
        );
        for (name, value) in self.attributes {
            entries.push((RawValue::String(name), RawValue::String(value)));
        }
        if !has_children && self.has_text {
            entries.push((
                RawValue::String(TEXT_KEY.to_owned()),
                RawValue::String(self.text),
            ));
        }
        for (name, mut values) in self.children {
            let value = if values.len() == 1 {
                values.pop().unwrap_or(RawValue::Null)
            } else {
                RawValue::Sequence(values)
            };
            entries.push((RawValue::String(name), value));
        }
        Ok((self.name, RawValue::Mapping(entries)))
    }
}

/// Read one XML document into the parser-neutral syntax nodes.
///
/// The answer is always a one-entry mapping keyed by the root element's
/// qualified name, because an XML document is exactly one root element and its
/// name is part of what the document says.
pub(super) fn parse(input: &str, limits: Limits) -> Result<RawValue> {
    if limits.max_documents() == 0 {
        return Err(codec_error(0, "document limit exceeded"));
    }
    let depth_limit = limits.max_depth().min(MAX_PARSER_DEPTH);
    let mut reader = Reader::from_str(input);
    // Character data is kept exactly as written: what is inside an element is
    // the element's value, and the trimming other readers apply is a reading
    // this one leaves to the field that types the text.
    let config = reader.config_mut();
    config.trim_text(false);
    config.expand_empty_elements = false;
    config.check_comments = true;

    let mut stack: Vec<Element> = Vec::new();
    let mut document: Option<(String, RawValue)> = None;
    loop {
        let position = position_of(&reader);
        let event = reader
            .read_event()
            .map_err(|error| malformed(position, &error.to_string()))?;
        match event {
            Event::Decl(declaration) => check_declaration(&declaration, position)?,
            Event::Start(start) => {
                let element = open(&start, position, &stack, depth_limit, document.is_some())?;
                stack.push(element);
            }
            Event::Empty(start) => {
                let element = open(&start, position, &stack, depth_limit, document.is_some())?;
                close(element, &mut stack, &mut document)?;
            }
            Event::End(_) => {
                // quick-xml checks that the name matches its start tag, so an
                // end event here always closes the element on top.
                let Some(element) = stack.pop() else {
                    return Err(malformed(position, "end tag without a start tag"));
                };
                let (name, value) = element.finish()?;
                match stack.last_mut() {
                    Some(parent) => parent.push_child(name, value),
                    None => document = Some((name, value)),
                }
            }
            Event::Text(text) => {
                let text = text
                    .xml10_content()
                    .map_err(|error| malformed(position, &error.to_string()))?;
                push_text(&mut stack, &text, position)?;
            }
            // A CDATA section is how a document holds a `<` or an `&` without
            // escaping one, so it is content and never unescaped.
            Event::CData(data) => {
                let data = data
                    .decode()
                    .map_err(|error| malformed(position, &error.to_string()))?;
                push_text(&mut stack, &data, position)?;
            }
            // Only the five predefined entities and character references
            // resolve. No document-declared entity is expanded, so no external
            // or recursive definition can be reached through this parser.
            Event::GeneralRef(reference) => {
                let raw = String::from_utf8_lossy(reference.as_ref()).into_owned();
                let spelled = format!("&{raw};");
                let resolved = quick_xml::escape::unescape(&spelled)
                    .map_err(|error| malformed(position, &error.to_string()))?;
                push_text(&mut stack, &resolved, position)?;
            }
            // Comments, processing instructions and the document type
            // declaration annotate a document without valuing it.
            Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }

    if let Some(element) = stack.last() {
        return Err(malformed(
            element.position,
            "an element was left open at the end of the document",
        ));
    }
    let (name, value) = document.ok_or_else(|| codec_error(0, "expected one XML root element"))?;
    Ok(RawValue::Mapping(vec![(RawValue::String(name), value)]))
}

/// Start one element, bounding the nesting and the number of roots.
fn open(
    start: &BytesStart<'_>,
    position: usize,
    stack: &[Element],
    depth_limit: usize,
    closed_root: bool,
) -> Result<Element> {
    if stack.is_empty() && closed_root {
        return Err(malformed(
            position,
            "an XML document has exactly one root element",
        ));
    }
    if stack.len() >= depth_limit {
        return Err(codec_error(position, "nesting depth limit exceeded"));
    }
    let name = name_of(start.name().as_ref(), position)?;
    let mut element = Element::new(name, position);
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| malformed(position, &error.to_string()))?;
        element
            .attributes
            .push(read_attribute(&attribute, position)?);
    }
    Ok(element)
}

/// Close one element that never had an end tag of its own.
fn close(
    element: Element,
    stack: &mut Vec<Element>,
    document: &mut Option<(String, RawValue)>,
) -> Result<()> {
    let (name, value) = element.finish()?;
    match stack.last_mut() {
        Some(parent) => parent.push_child(name, value),
        None => *document = Some((name, value)),
    }
    Ok(())
}

/// Read one attribute as the `@`-keyed entry it becomes.
fn read_attribute(attribute: &Attribute<'_>, position: usize) -> Result<(String, String)> {
    let name = name_of(attribute.key.as_ref(), position)?;
    let value = attribute
        .normalized_value(quick_xml::XmlVersion::Explicit1_0)
        .map_err(|error| malformed(position, &error.to_string()))?;
    let mut key = String::with_capacity(name.len() + ATTRIBUTE_PREFIX.len_utf8());
    key.push(ATTRIBUTE_PREFIX);
    key.push_str(&name);
    Ok((key, value.into_owned()))
}

/// Add character data to the element it was read inside.
fn push_text(stack: &mut [Element], text: &str, position: usize) -> Result<()> {
    let Some(element) = stack.last_mut() else {
        if is_xml_space(text) {
            return Ok(());
        }
        return Err(malformed(
            position,
            "character data outside the root element",
        ));
    };
    element.text.push_str(text);
    element.has_text = true;
    Ok(())
}

/// Refuse a declaration this reader cannot honour.
///
/// The bytes are read as UTF-8, so a document that declares another encoding
/// disagrees with what is being read rather than describing it.
fn check_declaration(declaration: &BytesDecl<'_>, position: usize) -> Result<()> {
    let version = declaration
        .version()
        .map_err(|error| malformed(position, &error.to_string()))?;
    if !matches!(version.as_ref(), b"1.0" | b"1.1") {
        return Err(codec_error(
            position,
            "expected an XML version of 1.0 or 1.1",
        ));
    }
    if let Some(encoding) = declaration.encoding() {
        let encoding = encoding.map_err(|error| malformed(position, &error.to_string()))?;
        if !encoding.eq_ignore_ascii_case(b"utf-8") && !encoding.eq_ignore_ascii_case(b"utf8") {
            return Err(codec_error(position, "expected a UTF-8 encoding declaration"));
        }
    }
    Ok(())
}

/// One element or attribute name, as UTF-8 the rest of the read can key with.
fn name_of(name: &[u8], position: usize) -> Result<String> {
    std::str::from_utf8(name)
        .map(str::to_owned)
        .map_err(|_| malformed(position, "name is not valid UTF-8"))
}

/// Whether character data is only the whitespace that lays a document out.
fn is_xml_space(text: &str) -> bool {
    text.bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
}

fn malformed(position: usize, reason: &str) -> Error {
    Error::Codec {
        format: "xml",
        position,
        reason: format_smolstr!("{}", elide_to(reason, ERROR_TEXT_LIMIT)),
    }
}

/// The byte the reader has reached, which every refusal is located at.
fn position_of(reader: &Reader<&[u8]>) -> usize {
    usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
}
