//! Bounded XML 1.0 decoding: quick-xml's pull events, folded into the natural
//! value with no element tree in between.
//!
//! One frame per open element holds its attributes, the text pieces it has
//! met and the children closed under it; closing the element makes its value
//! and hands it to the frame beneath. A leaf's text stays borrowed from the
//! input until it becomes the `Scalar`, so a leaf costs its value and no copy
//! of its text; the frames, a record's map and a repeated element's sequence
//! are what a document's shape costs.

use std::borrow::Cow;

use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::{Decoder, Reader, XmlVersion};
use smol_str::SmolStr;

use crate::text::Limits;
use crate::{Error, Result, Scalar};

use super::wire::{illegal_character, is_name};
use super::{ATTRIBUTE_PREFIX, TEXT_KEY};

/// The attribute XML Schema Instance marks an absent value with. The prefix
/// is the conventional one and is matched as spelled: this reader resolves no
/// namespace, and every producer that writes the attribute writes it so.
const NIL_ATTRIBUTE: &str = "xsi:nil";

pub(super) fn parse(input: &str, limits: Limits) -> Result<Scalar> {
    if limits.max_documents() == 0 {
        return Err(codec_error(0, "document limit exceeded".into()));
    }
    // A byte order mark is framing: quick-xml would skip it too, but without
    // counting it, and every position reported here counts from the input.
    let (input, base) = match input.strip_prefix('\u{FEFF}') {
        Some(rest) => (rest, '\u{FEFF}'.len_utf8()),
        None => (input, 0),
    };
    let mut reader = Reader::from_str(input);
    let decoder = reader.decoder();
    let mut state = State {
        limits,
        base,
        nodes: 0,
        stack: Vec::new(),
        root: None,
    };
    loop {
        let position = state.position(reader.buffer_position());
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(error) => {
                return Err(codec_error(
                    state.position(reader.error_position()),
                    error.to_string(),
                ));
            }
        };
        match event {
            Event::Start(start) => state.open(&start, decoder, position, false)?,
            Event::Empty(start) => {
                state.open(&start, decoder, position, true)?;
                state.close(position)?;
            }
            // quick-xml has already matched the closing name to the open one.
            Event::End(_) => state.close(position)?,
            Event::Text(text) => {
                let piece = text
                    .xml10_content()
                    .map_err(|error| codec_error(position, error.to_string()))?;
                state.text(piece, position)?;
            }
            Event::CData(data) => {
                let piece = data
                    .xml10_content()
                    .map_err(|error| codec_error(position, error.to_string()))?;
                state.text(piece, position)?;
            }
            Event::GeneralRef(reference) => state.reference(&reference, position)?,
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_) => {}
            Event::Eof => return state.finish(position),
        }
    }
}

/// One open element: what it has said so far.
struct Frame<'a> {
    /// The name as the document spells it, prefix included.
    name: SmolStr,
    /// The attributes, each already under its `@` key, in document order.
    attributes: Vec<(SmolStr, Scalar)>,
    /// The character data met directly under this element, borrowed while
    /// it is one piece.
    text: Option<Cow<'a, str>>,
    /// The closed child elements, in document order.
    children: Vec<(SmolStr, Scalar)>,
    /// Whether `xsi:nil` declared the element's value absent.
    nil: bool,
    /// Whether the element was the empty-element tag `<a/>`, which is null,
    /// rather than `<a></a>`, which is present and empty.
    empty: bool,
}

struct State<'a> {
    limits: Limits,
    /// The bytes of the mark stripped before the reader, so a position counts
    /// from the start of the input.
    base: usize,
    nodes: usize,
    stack: Vec<Frame<'a>>,
    /// The document element once it has closed.
    root: Option<(SmolStr, Scalar)>,
}

impl<'a> State<'a> {
    fn position(&self, offset: u64) -> usize {
        usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .saturating_add(self.base)
    }

    fn observe_node(&mut self, position: usize) -> Result<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes() {
            Err(codec_error(position, "decoded node limit exceeded".into()))
        } else {
            Ok(())
        }
    }

    fn observe_depth(&self, position: usize) -> Result<()> {
        let depth = self.stack.len().saturating_add(1);
        if depth > super::MAX_PARSER_DEPTH {
            Err(codec_error(
                position,
                format!(
                    "XML nesting exceeds the parser hard limit of {}",
                    super::MAX_PARSER_DEPTH
                ),
            ))
        } else if depth > self.limits.max_depth() {
            Err(codec_error(position, "nesting depth limit exceeded".into()))
        } else {
            Ok(())
        }
    }

    /// Open the element `start` names under the current frame.
    fn open(
        &mut self,
        start: &BytesStart<'a>,
        decoder: Decoder,
        position: usize,
        empty: bool,
    ) -> Result<()> {
        if self.root.is_some() {
            return Err(codec_error(
                position,
                "content after the root element".into(),
            ));
        }
        self.observe_depth(position)?;
        self.observe_node(position)?;
        let name = SmolStr::new(name_text(start.name().into_inner(), position)?);
        let mut attributes = Vec::new();
        let mut nil = false;
        for attribute in start.attributes() {
            let attribute = attribute.map_err(|error| codec_error(position, error.to_string()))?;
            self.observe_node(position)?;
            let key = name_text(attribute.key.into_inner(), position)?;
            let value = attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
                .map_err(|error| codec_error(position, error.to_string()))?;
            refuse_illegal(&value, position)?;
            if key == NIL_ATTRIBUTE && matches!(value.as_ref(), "true" | "1") {
                nil = true;
            }
            let mut prefixed = String::with_capacity(ATTRIBUTE_PREFIX.len() + key.len());
            prefixed.push_str(ATTRIBUTE_PREFIX);
            prefixed.push_str(key);
            attributes.push((SmolStr::from(prefixed), text_scalar(value)));
        }
        self.stack.push(Frame {
            name,
            attributes,
            text: None,
            children: Vec::new(),
            nil,
            empty,
        });
        Ok(())
    }

    /// Add character data to the open element, or refuse it outside one.
    fn text(&mut self, piece: Cow<'a, str>, position: usize) -> Result<()> {
        refuse_illegal(&piece, position)?;
        let Some(frame) = self.stack.last_mut() else {
            if is_blank(&piece) {
                return Ok(());
            }
            return Err(codec_error(
                position,
                "character data outside the root element".into(),
            ));
        };
        match &mut frame.text {
            None => frame.text = Some(piece),
            Some(existing) => existing.to_mut().push_str(&piece),
        }
        Ok(())
    }

    /// Resolve a character or predefined entity reference into the open
    /// element's text; anything else is a reference the document did not
    /// define for this reader.
    fn reference(&mut self, reference: &BytesRef<'a>, position: usize) -> Result<()> {
        let resolved = match reference
            .resolve_char_ref()
            .map_err(|error| codec_error(position, error.to_string()))?
        {
            Some(character) => Cow::Owned(character.to_string()),
            None => {
                let name = reference
                    .decode()
                    .map_err(|error| codec_error(position, error.to_string()))?;
                match quick_xml::escape::resolve_predefined_entity(&name) {
                    Some(text) => Cow::Borrowed(text),
                    None => {
                        return Err(codec_error(
                            position,
                            format!("unknown entity reference `&{name};`"),
                        ));
                    }
                }
            }
        };
        if self.stack.is_empty() {
            return Err(codec_error(
                position,
                "character data outside the root element".into(),
            ));
        }
        self.text(resolved, position)
    }

    /// Close the innermost element and hand its value to the one beneath.
    fn close(&mut self, position: usize) -> Result<()> {
        let Some(frame) = self.stack.pop() else {
            return Err(codec_error(position, "unmatched closing tag".into()));
        };
        let Frame {
            name,
            attributes,
            text,
            children,
            nil,
            empty,
        } = frame;
        let blank = text.as_deref().is_none_or(is_blank);
        // `xsi:nil` on an element that says nothing else is the absence it
        // declares; beside other attributes or content it is one attribute.
        let value = if nil && attributes.len() == 1 && children.is_empty() && blank {
            Scalar::Null
        } else if attributes.is_empty() && children.is_empty() {
            match text {
                Some(text) => text_scalar(text),
                None if empty => Scalar::Null,
                None => Scalar::from(""),
            }
        } else {
            let mut entries = attributes;
            entries.reserve(children.len() + 1);
            let had_children = !children.is_empty();
            merge_children(children, &mut entries);
            match text {
                Some(text) if !had_children || !is_blank(&text) => {
                    self.observe_node(position)?;
                    entries.push((SmolStr::new_static(TEXT_KEY), text_scalar(text)));
                }
                // `<a k="v"></a>`: present, with empty text of its own.
                None if !had_children && !empty => {
                    self.observe_node(position)?;
                    entries.push((SmolStr::new_static(TEXT_KEY), Scalar::from("")));
                }
                _ => {}
            }
            Scalar::from_struct(entries).map_err(|error| match error {
                Error::Codec { reason, .. } => codec_error(position, reason.to_string()),
                other => other,
            })?
        };
        match self.stack.last_mut() {
            Some(parent) => parent.children.push((name, value)),
            None => self.root = Some((name, value)),
        }
        Ok(())
    }

    fn finish(self, position: usize) -> Result<Scalar> {
        if let Some(open) = self.stack.last() {
            return Err(codec_error(
                position,
                format!("<{}> never closed", open.name),
            ));
        }
        let Some((name, value)) = self.root else {
            return Err(codec_error(position, "expected the root element".into()));
        };
        Scalar::from_struct([(name, value)])
    }
}

/// Fold the closed children into entries: one entry per name, a sequence in
/// document order where a name repeats.
///
/// The sort is stable, so repeated elements keep their order, and it costs
/// what the record's own sorted map will cost anyway.
fn merge_children(mut children: Vec<(SmolStr, Scalar)>, entries: &mut Vec<(SmolStr, Scalar)>) {
    if children.len() > 1 {
        children.sort_by(|left, right| left.0.cmp(&right.0));
    }
    let mut children = children.into_iter().peekable();
    while let Some((name, value)) = children.next() {
        if children.peek().is_some_and(|(next, _)| *next == name) {
            let mut items = vec![value];
            while let Some((_, value)) = children.next_if(|(next, _)| *next == name) {
                items.push(value);
            }
            entries.push((name, Scalar::from_sequence(items)));
        } else {
            entries.push((name, value));
        }
    }
}

/// The text a piece of the document is: a borrowed piece becomes the value
/// straight from the input, an assembled one moves.
fn text_scalar(text: Cow<'_, str>) -> Scalar {
    match text {
        Cow::Borrowed(text) => Scalar::from(text),
        Cow::Owned(text) => Scalar::from(text),
    }
}

/// XML's own whitespace set, narrower than Unicode's.
fn is_blank(text: &str) -> bool {
    text.bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
}

/// A name the reader sliced out of the UTF-8 input, checked against XML's
/// own `Name`: quick-xml checks neither, and a name this reader accepted is
/// one the writer must accept back.
fn name_text(name: &[u8], position: usize) -> Result<&str> {
    let name = std::str::from_utf8(name)
        .map_err(|_| codec_error(position, "element name is not valid UTF-8".into()))?;
    if is_name(name) {
        Ok(name)
    } else {
        Err(codec_error(
            position,
            format!(
                "expected an XML name, got {:?}",
                crate::text::elide_to(name, crate::text::ERROR_TEXT_LIMIT)
            ),
        ))
    }
}

/// Refuse a character XML 1.0 cannot carry, whichever way it was spelled.
fn refuse_illegal(text: &str, position: usize) -> Result<()> {
    match illegal_character(text) {
        Some(character) => Err(codec_error(
            position,
            format!("text carries U+{character:04X}, which XML 1.0 cannot carry"),
        )),
        None => Ok(()),
    }
}

fn codec_error(position: usize, reason: String) -> Error {
    Error::Codec {
        format: "xml",
        position,
        reason: reason.into(),
    }
}
