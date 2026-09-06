//! One XML document read into the shared `Scalar`.
//!
//! The reader is a flat loop over the protocol's events with its own element
//! stack: nesting is bounded by [`Limits::max_depth`] rather than by the native
//! stack, so a document that nests a million elements is a bounded error and
//! never an abort.

use std::collections::BTreeMap;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use smol_str::{SmolStr, format_smolstr};

use crate::text::{Limits, Scalar};
use crate::{Error, Result};

use super::{ATTRIBUTE_PREFIX, FORMAT, TEXT_KEY};

/// Read exactly one document element into its value.
pub(crate) fn parse(input: &str, limits: Limits) -> Result<Scalar> {
    let mut reader = quick_xml::Reader::from_str(input);
    let config = reader.config_mut();
    // Every well-formedness constraint the tokenizer can answer is on: an
    // unmatched or mismatched end tag is a document error here rather than a
    // silently different value downstream.
    config.check_end_names = true;
    config.allow_unmatched_ends = false;
    config.allow_dangling_amp = false;
    config.check_comments = true;
    // Empty elements keep their own event: `<a/>` and `<a></a>` are the same
    // value, and the reader says which one it read without a second pass.
    config.expand_empty_elements = false;

    let mut state = State::new(limits);
    let mut stack: Vec<Element> = Vec::new();
    let mut document: Option<(SmolStr, Scalar)> = None;
    let mut version = XmlVersion::Implicit1_0;

    loop {
        let position = position(&reader);
        match reader
            .read_event()
            .map_err(|error| protocol(error, position))?
        {
            Event::Decl(declaration) => {
                version = declaration
                    .xml_version()
                    .map_err(|error| protocol(error, position))?;
            }
            Event::Start(start) => {
                if stack.is_empty() && document.is_some() {
                    return Err(second_root(position));
                }
                state.enter(stack.len(), position)?;
                stack.push(Element::open(&start, position, version, &mut state)?);
            }
            Event::Empty(start) => {
                if stack.is_empty() && document.is_some() {
                    return Err(second_root(position));
                }
                state.enter(stack.len(), position)?;
                let element = Element::open(&start, position, version, &mut state)?;
                let (name, value) = element.close()?;
                place(&mut stack, &mut document, name, value);
            }
            Event::End(_) => {
                // `check_end_names` already proved this end tag closes the open
                // element, so the stack top is the element it names.
                let element = stack.pop().ok_or_else(|| unmatched_end(position))?;
                let (name, value) = element.close()?;
                place(&mut stack, &mut document, name, value);
            }
            Event::Text(text) => {
                let text = text
                    .xml_content(version)
                    .map_err(|error| protocol(error, position))?;
                match stack.last_mut() {
                    Some(element) => element.push_text(&text, &mut state)?,
                    None if text.trim_matches(is_xml_space).is_empty() => {}
                    None => return Err(text_outside_root(position)),
                }
            }
            Event::CData(section) => {
                let text = section
                    .decode()
                    .map_err(|error| protocol(error, position))?;
                match stack.last_mut() {
                    Some(element) => {
                        element.literal = true;
                        element.push_text(&text, &mut state)?;
                    }
                    None => return Err(text_outside_root(position)),
                }
            }
            Event::GeneralRef(reference) => {
                let reference = reference
                    .decode()
                    .map_err(|error| protocol(error, position))?;
                let resolved = resolve(&reference, position)?;
                match stack.last_mut() {
                    Some(element) => element.push_text(resolved.as_str(), &mut state)?,
                    None => return Err(text_outside_root(position)),
                }
            }
            // A comment, a processing instruction, and a document type
            // declaration are annotations about the document rather than
            // content in it, exactly as a YAML tag is.
            Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
            Event::Eof => break,
        }
    }

    let (name, value) = document.ok_or_else(|| Error::Codec {
        format: FORMAT,
        position: input.len(),
        reason: "expected one document element, got no element".into(),
    })?;
    Scalar::from_record([(name, value)])
}

/// The character data and children one open element has read so far.
pub(crate) struct Element {
    name: SmolStr,
    position: usize,
    entries: BTreeMap<SmolStr, Vec<Scalar>>,
    text: String,
    /// Whether a CDATA section contributed, which keeps the text exact.
    pub(crate) literal: bool,
}

impl Element {
    pub(crate) fn open(
        start: &BytesStart<'_>,
        position: usize,
        version: XmlVersion,
        state: &mut State,
    ) -> Result<Self> {
        let mut element = Self {
            name: SmolStr::new(name_text(start.name().as_ref(), position)?),
            position,
            entries: BTreeMap::new(),
            text: String::new(),
            literal: false,
        };
        for attribute in start.attributes() {
            let attribute = attribute.map_err(|error| protocol(error, position))?;
            state.node(position)?;
            let value = attribute
                .normalized_value(version)
                .map_err(|error| protocol(error, position))?;
            let key = name_text(attribute.key.as_ref(), position)?;
            let mut name = String::with_capacity(key.len() + 1);
            name.push(ATTRIBUTE_PREFIX);
            name.push_str(key);
            element
                .entries
                .insert(SmolStr::new(name), vec![Scalar::from(value.into_owned())]);
        }
        Ok(element)
    }

    pub(crate) fn push_text(&mut self, text: &str, state: &mut State) -> Result<()> {
        state.text(text.len(), self.position)?;
        self.text.push_str(text);
        Ok(())
    }

    pub(crate) fn push_child(&mut self, name: SmolStr, value: Scalar) {
        self.entries.entry(name).or_default().push(value);
    }

    /// Answer this element's name and the value it holds.
    pub(crate) fn close(mut self) -> Result<(SmolStr, Scalar)> {
        let text = self.retained_text();
        if self.entries.is_empty() {
            let value = text.map_or(Scalar::Null, Scalar::from);
            return Ok((self.name, value));
        }
        if let Some(text) = text {
            self.entries
                .insert(SmolStr::new_static(TEXT_KEY), vec![Scalar::from(text)]);
        }
        let entries = self.entries.into_iter().map(|(name, mut values)| {
            let value = match values.len() {
                1 => values.pop().unwrap_or(Scalar::Null),
                _ => Scalar::from_sequence(values),
            };
            (name, value)
        });
        Ok((self.name, Scalar::from_record(entries)?))
    }

    /// Return the character data this element keeps, if any.
    ///
    /// A CDATA section exists to carry exact bytes, so an element that holds
    /// one keeps its character data unchanged. Everywhere else layout is not
    /// content: whitespace-only data is dropped and an element's own text
    /// loses its outer whitespace, so an indented document reads as the values
    /// it spells rather than as the indentation around them.
    fn retained_text(&mut self) -> Option<String> {
        let text = std::mem::take(&mut self.text);
        if self.literal {
            return (!text.is_empty()).then_some(text);
        }
        let trimmed = text.trim_matches(is_xml_space);
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.len() == text.len() {
            return Some(text);
        }
        Some(trimmed.to_owned())
    }
}

/// Give one closed element to its parent, or to the document.
fn place(
    stack: &mut [Element],
    document: &mut Option<(SmolStr, Scalar)>,
    name: SmolStr,
    value: Scalar,
) {
    match stack.last_mut() {
        Some(parent) => parent.push_child(name, value),
        None => *document = Some((name, value)),
    }
}

/// The bounds a caller-controlled document is decoded under.
pub(crate) struct State {
    limits: Limits,
    nodes: usize,
    bytes: usize,
}

impl State {
    pub(crate) const fn new(limits: Limits) -> Self {
        Self {
            limits,
            nodes: 0,
            bytes: 0,
        }
    }

    /// Account for one element opening at `depth` levels of open ancestors.
    pub(crate) fn enter(&mut self, depth: usize, position: usize) -> Result<()> {
        if depth.saturating_add(1) > self.limits.max_depth() {
            return Err(Error::Codec {
                format: FORMAT,
                position,
                reason: "nesting depth limit exceeded".into(),
            });
        }
        self.node(position)
    }

    pub(crate) fn node(&mut self, position: usize) -> Result<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes() {
            return Err(Error::Codec {
                format: FORMAT,
                position,
                reason: "decoded node limit exceeded".into(),
            });
        }
        Ok(())
    }

    /// Account for character data, which grows a value without adding a node.
    pub(crate) fn text(&mut self, len: usize, position: usize) -> Result<()> {
        self.bytes = self.bytes.saturating_add(len);
        if self.bytes > self.limits.max_input_bytes() {
            return Err(crate::text::input_too_large(
                FORMAT,
                self.limits.max_input_bytes(),
            ));
        }
        self.node(position)
    }
}

/// Resolve one reference into the text it stands for.
///
/// Character references and the five entities XML predefines resolve. Nothing
/// else does: a document type declaration is an annotation here, so an entity
/// it declares has no definition to expand and is reported by name rather than
/// read from wherever the declaration points.
pub(crate) fn resolve(reference: &str, position: usize) -> Result<SmolStr> {
    if let Some(resolved) = quick_xml::events::BytesRef::new(reference)
        .resolve_char_ref()
        .map_err(|error| protocol(error, position))?
    {
        let mut buffer = [0_u8; 4];
        return Ok(SmolStr::new(resolved.encode_utf8(&mut buffer)));
    }
    match reference {
        "amp" => Ok(SmolStr::new_static("&")),
        "lt" => Ok(SmolStr::new_static("<")),
        "gt" => Ok(SmolStr::new_static(">")),
        "apos" => Ok(SmolStr::new_static("'")),
        "quot" => Ok(SmolStr::new_static("\"")),
        other => Err(Error::Codec {
            format: FORMAT,
            position,
            reason: crate::text::expected_got(
                "a character reference or one of &amp; &lt; &gt; &apos; &quot;",
                format_args!("&{};", crate::text::elide_display(&other)),
            ),
        }),
    }
}

/// Borrow one name the tokenizer answers in the source's own bytes.
///
/// The source was proven UTF-8 before a byte of it was tokenized, so this
/// only restates that proof where the protocol hands back a byte slice.
pub(crate) fn name_text(name: &[u8], position: usize) -> Result<&str> {
    std::str::from_utf8(name).map_err(|error| Error::Codec {
        format: FORMAT,
        position: position.saturating_add(error.valid_up_to()),
        reason: "element and attribute names are UTF-8".into(),
    })
}

/// Return whether one character is XML whitespace.
pub(crate) const fn is_xml_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\r' | '\n')
}

pub(crate) fn position<R>(reader: &quick_xml::Reader<R>) -> usize {
    usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
}

pub(crate) fn protocol(error: impl std::fmt::Display, position: usize) -> Error {
    Error::Codec {
        format: FORMAT,
        position,
        reason: format_smolstr!("{}", crate::text::elide_display(&error)),
    }
}

fn second_root(position: usize) -> Error {
    Error::Codec {
        format: FORMAT,
        position,
        reason: "expected one document element, got a second one".into(),
    }
}

pub(crate) fn text_outside_root(position: usize) -> Error {
    Error::Codec {
        format: FORMAT,
        position,
        reason: "expected character data inside the document element".into(),
    }
}

pub(crate) fn unmatched_end(position: usize) -> Error {
    Error::Codec {
        format: FORMAT,
        position,
        reason: "expected an open element for this end tag".into(),
    }
}
