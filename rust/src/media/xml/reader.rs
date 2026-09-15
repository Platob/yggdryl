//! The crate's one XML event loop.
//!
//! Every XML surface here - the [`crate::Scalar`] codec and the columnar one -
//! reads through this module, so the facts a document states are decided once:
//! which bytes are a name, what an attribute's value is after normalization,
//! where a text run ends, and what a depth is. A second loop would be a second
//! answer to each of those.
//!
//! The parser is quick-xml's, driven over the document's own bytes through the
//! inherent slice reader rather than the buffered one, so an event's bytes are
//! a borrow out of the document rather than a copy into a scratch `Vec`. A
//! skipped subtree costs the bytes of its own tags and nothing else, because
//! [`Cursor::skip`] consumes it inside the parser's state machine without
//! building one event for it.
//!
//! What the loop refuses, it refuses by name. quick-xml answers end-of-input
//! for a document that stops inside an element rather than failing, so an
//! unclosed element is this module's refusal to raise, not the parser's.

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::{NsReader, XmlVersion};
use smol_str::{SmolStr, format_smolstr};

use crate::text::{ERROR_TEXT_LIMIT, Limits, elide_to};
use crate::{Error, Result};

/// What this codec names itself in a refusal.
pub(crate) const FORMAT: &str = "xml";

/// Nesting past which the reader stops, whatever a caller's limits say.
///
/// The parser itself is iterative, so the stack is never the thing at risk;
/// this bounds the state *this* module keeps per open element. It sits above
/// every caller-facing bound, so a document refused here is refused for being
/// absurd rather than for a caller's budget, and it is checked first and with
/// its own message, exactly as the JSON and YAML readers do.
pub const MAX_PARSER_DEPTH: usize = 384;

/// The XML Schema instance namespace, whose attributes are annotations.
const XSI_NAMESPACE: &[u8] = b"http://www.w3.org/2001/XMLSchema-instance";

/// The namespace of the reserved `xml:` prefix.
const XML_NAMESPACE: &[u8] = b"http://www.w3.org/XML/1998/namespace";

/// Build the codec's refusal at a byte offset.
pub(crate) fn codec_error(position: usize, reason: impl Into<SmolStr>) -> Error {
    Error::Codec {
        format: FORMAT,
        position,
        reason: reason.into(),
    }
}

/// Restate caller text at a bounded width before it reaches a message.
pub(crate) fn quoted(value: &str) -> impl std::fmt::Display + '_ {
    elide_to(value, ERROR_TEXT_LIMIT)
}

/// Whether a name is a namespace binding rather than a column.
///
/// `xmlns="u"` and `xmlns:a="u"` bind namespaces. Their local names are
/// `xmlns` and `a`, so a reader keyed on local names alone would publish a
/// column called `a` holding a URI.
fn is_namespace_declaration(name: QName<'_>) -> bool {
    let name = name.as_ref();
    name == b"xmlns" || name.starts_with(b"xmlns:")
}

/// One element's or attribute's identity.
///
/// A prefix names a document's own vocabulary and never a field, so the local
/// name is the column's - the rule the FIX and CBlock readers already follow.
/// The namespace rides beside it so that two vocabularies folding onto one
/// local name can be refused rather than silently merged into one column.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Name {
    local: SmolStr,
    namespace: Option<SmolStr>,
    /// The name exactly as the document wrote it, prefix included. The parser
    /// matches an end tag against this, and a write spells it back.
    raw: SmolStr,
}

impl Name {
    /// The local name, which is what a column is called.
    pub(crate) fn local(&self) -> &str {
        &self.local
    }

    /// The namespace URI this name was bound in, if any.
    pub(crate) fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }
}

/// Whether a scalar is one XML 1.0 can carry at all.
///
/// XML 1.0's `Char` production excludes most of the C0 controls and the two
/// non-characters at the end of the BMP. A document holding one is not a
/// document, and neither a reference nor an escape can spell it, so this is
/// the one question that has no best-effort answer.
const fn is_xml_char(code: u32) -> bool {
    matches!(code, 0x9 | 0xA | 0xD)
        || matches!(code, 0x20..=0xD7FF)
        || matches!(code, 0xE000..=0xFFFD)
        || matches!(code, 0x10000..=0x10FFFF)
}

/// Refuse a run holding a scalar XML cannot carry.
fn check_chars(run: &str, position: usize, what: &str) -> Result<()> {
    match run
        .chars()
        .find(|character| !is_xml_char(*character as u32))
    {
        None => Ok(()),
        Some(character) => Err(codec_error(
            position,
            format_smolstr!(
                "expected {what} to hold characters XML can carry, got U+{:04X}",
                character as u32
            ),
        )),
    }
}

/// Apply XML 1.0 line-ending normalization to one run.
///
/// A conforming processor translates every `\r\n` and every remaining `\r`
/// into a single `\n` before parsing, CDATA included, so two documents that
/// differ only in line ending are the same document. Borrowing is the common
/// case: a run with no carriage return is returned untouched.
fn normalize_newlines(run: &str) -> std::borrow::Cow<'_, str> {
    if !run.contains('\r') {
        return std::borrow::Cow::Borrowed(run);
    }
    let mut out = String::with_capacity(run.len());
    let mut characters = run.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' {
            if characters.peek() == Some(&'\n') {
                characters.next();
            }
            out.push('\n');
        } else {
            out.push(character);
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Read one borrowed byte run as the text it claims to be.
fn text_of(bytes: &[u8], position: usize, what: &str) -> Result<SmolStr> {
    std::str::from_utf8(bytes).map(SmolStr::new).map_err(|_| {
        codec_error(
            position,
            format_smolstr!("expected {what} in UTF-8, got bytes that are not"),
        )
    })
}

/// Copy a resolved namespace out of the parser's resolver.
///
/// An *unbound* name carries no prefix and is simply in no namespace. An
/// *unknown* one carries a prefix nothing declared, which Namespaces in XML
/// makes a fatal error - and folding it to "no namespace" would merge two
/// different broken vocabularies into one column, which is the very collision
/// this codec refuses when the prefixes are bound.
fn namespace_of(resolved: &ResolveResult<'_>, position: usize) -> Result<Option<SmolStr>> {
    match resolved {
        ResolveResult::Bound(namespace) => {
            text_of(namespace.as_ref(), position, "a namespace").map(Some)
        }
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Unknown(prefix) => Err(codec_error(
            position,
            format_smolstr!(
                "expected every namespace prefix to be declared, got {} bound to nothing",
                quoted(&String::from_utf8_lossy(prefix))
            ),
        )),
    }
}

/// One attribute of an open element, already normalized.
#[derive(Clone, Debug)]
pub(crate) struct Attr {
    /// The attribute's identity.
    pub(crate) name: Name,
    /// The value after XML's own attribute-value normalization.
    pub(crate) value: SmolStr,
}

/// One step of the walk.
#[derive(Debug)]
pub(crate) enum Step {
    /// An element opened, with its attributes already normalized. `empty` is
    /// `<x/>`, which closes without a matching [`Step::Close`].
    Open {
        /// The element's identity.
        name: Name,
        /// The element's attributes, namespace bindings excluded.
        attributes: Vec<Attr>,
        /// Whether the element closed in the same event.
        empty: bool,
        /// Whether `xsi:nil="true"` marks the element as absent.
        nil: bool,
    },
    /// The innermost open element closed, carrying its accumulated text.
    Close {
        /// Every character run the element held, joined in document order.
        text: SmolStr,
    },
    /// The document ended with every element closed.
    End,
}

/// A bounded, borrowing walk over one XML document.
pub(crate) struct Cursor<'a> {
    reader: NsReader<&'a [u8]>,
    limits: Limits,
    nodes: usize,
    /// One accumulated text run per open element. Character data is appended
    /// rather than assigned, because a run splits at every entity boundary:
    /// `a &lt; b` arrives as three events and keeping the last would keep a
    /// fragment.
    text: Vec<String>,
    /// Whether the document element has closed, so trailing content is
    /// content after the root rather than part of it.
    rooted: bool,
}

impl<'a> Cursor<'a> {
    /// Open a walk over one document's bytes.
    pub(crate) fn new(document: &'a [u8], limits: Limits) -> Self {
        let mut reader = NsReader::from_reader(document);
        let config = reader.config_mut();
        // Trimming is per event, and a text run splits at every entity
        // boundary, so trimming here would eat the space in front of `&lt;`
        // and join two words. What a column needs trimmed is trimmed once,
        // where the leaf closes and its datatype is known.
        config.trim_text(false);
        // An empty element stays its own event, so the depth this module keeps
        // and the events it sees cannot disagree.
        config.expand_empty_elements = false;
        config.check_end_names = true;
        Self {
            reader,
            limits,
            nodes: 0,
            text: Vec::new(),
            rooted: false,
        }
    }

    /// The byte the reader has consumed up to.
    pub(crate) fn position(&self) -> usize {
        usize::try_from(self.reader.buffer_position()).unwrap_or(usize::MAX)
    }

    /// How many elements are currently open.
    pub(crate) fn depth(&self) -> usize {
        self.text.len()
    }

    /// Charge one decoded node against the budget.
    fn observe_node(&mut self, position: usize) -> Result<()> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes() {
            Err(codec_error(position, "decoded node limit exceeded"))
        } else {
            Ok(())
        }
    }

    /// Charge one newly opened element against the depth budget.
    fn observe_depth(&self, position: usize) -> Result<()> {
        let depth = self.text.len().saturating_add(1);
        if depth > MAX_PARSER_DEPTH {
            return Err(codec_error(
                position,
                format_smolstr!("XML nesting exceeds the parser hard limit of {MAX_PARSER_DEPTH}"),
            ));
        }
        if depth > self.limits.max_depth() {
            return Err(codec_error(position, "nesting depth limit exceeded"));
        }
        Ok(())
    }

    /// Append one character run to the innermost open element.
    fn push_text(&mut self, run: &str, position: usize) -> Result<()> {
        match self.text.last_mut() {
            Some(held) => {
                held.push_str(run);
                Ok(())
            }
            None if run.trim().is_empty() => Ok(()),
            None => Err(codec_error(
                position,
                format_smolstr!(
                    "expected one document element, got content after it: the text {}",
                    quoted(run.trim())
                ),
            )),
        }
    }

    /// Read the attributes of an element the parser just opened.
    ///
    /// The list is walked once. A value arrives through XML's own
    /// attribute-value normalization, which resolves the five predefined
    /// entities and character references; the raw bytes are still escaped and
    /// are never what a column holds.
    fn attributes(
        &mut self,
        element: &BytesStart<'_>,
        position: usize,
    ) -> Result<(Vec<Attr>, bool)> {
        let mut attributes = Vec::new();
        let mut nil = false;
        for attribute in element.attributes() {
            let attribute =
                attribute.map_err(|error| codec_error(position, format_smolstr!("{error}")))?;
            if is_namespace_declaration(attribute.key) {
                continue;
            }
            let raw = text_of(attribute.key.as_ref(), position, "an attribute name")?;
            let resolved = self.reader.resolver_mut().resolve_attribute(attribute.key);
            let namespace = namespace_of(&resolved.0, position)?;
            let local = text_of(resolved.1.as_ref(), position, "an attribute name")?;
            let value = attribute
                .normalized_value(XmlVersion::Explicit1_0)
                .map_err(|error| codec_error(position, format_smolstr!("{error}")))?;
            let value = SmolStr::new(value.as_ref());
            match namespace.as_deref().map(str::as_bytes) {
                // The reserved namespaces carry annotations about the
                // document, never a column of it. `xsi:nil` is the one whose
                // answer a column needs, and it answers absence.
                Some(XSI_NAMESPACE) => {
                    if local == "nil" {
                        nil = value == "true" || value == "1";
                    }
                }
                Some(XML_NAMESPACE) => {}
                _ => {
                    self.observe_node(position)?;
                    attributes.push(Attr {
                        name: Name {
                            local,
                            namespace,
                            raw,
                        },
                        value,
                    });
                }
            }
        }
        Ok((attributes, nil))
    }

    /// Skip the innermost open element's subtree without decoding it.
    ///
    /// This is the projection primitive: an unselected column costs the bytes
    /// of its own tags and nothing else, because the parser consumes the run
    /// inside its own state machine and builds no event for it.
    pub(crate) fn skip(&mut self, name: &Name) -> Result<()> {
        let position = self.position();
        // The parser matches the end tag as it was written, prefix and all.
        // A local name would never find `</p:meta>`, and the read would run
        // past it to the end of the document.
        let raw = name.raw.clone();
        self.reader
            .read_to_end(QName(raw.as_bytes()))
            .map_err(|error| codec_error(position, format_smolstr!("{error}")))?;
        self.text.pop();
        Ok(())
    }

    /// Advance to the next step the row model cares about.
    ///
    /// Comments, processing instructions and the declaration are skipped;
    /// character data is accumulated rather than reported run by run.
    pub(crate) fn next(&mut self) -> Result<Step> {
        loop {
            let position = self.position();
            let (resolved, event) = self
                .reader
                .read_resolved_event()
                .map_err(|error| codec_error(position, format_smolstr!("{error}")))?;
            let namespace = namespace_of(&resolved, position)?;
            drop(resolved);
            match event {
                // `<x>` and `<x/>` stay two events, so the depth this
                // module keeps and the events it sees cannot disagree: an
                // empty element never gets a text frame, because no close
                // will arrive to take one off.
                Event::Start(element) => return self.open(element, namespace, position, false),
                Event::Empty(element) => return self.open(element, namespace, position, true),
                Event::End(_) => {
                    let text = self.text.pop().ok_or_else(|| {
                        codec_error(position, "expected an open element to close")
                    })?;
                    if self.text.is_empty() {
                        self.rooted = true;
                    }
                    return Ok(Step::Close {
                        text: SmolStr::new(text),
                    });
                }
                Event::Text(run) => {
                    let run = text_of(run.as_ref(), position, "character data")?;
                    check_chars(&run, position, "character data")?;
                    self.push_text(&normalize_newlines(&run), position)?;
                }
                Event::CData(run) => {
                    // A CDATA section is content already: nothing in it is
                    // markup and nothing in it is unescaped. Line endings are
                    // normalized there too - that happens before parsing, so
                    // the section's own framing does not exempt it.
                    let run = text_of(run.as_ref(), position, "a CDATA section")?;
                    check_chars(&run, position, "a CDATA section")?;
                    self.push_text(&normalize_newlines(&run), position)?;
                }
                Event::GeneralRef(reference) => {
                    // quick-xml reports an entity rather than folding it into
                    // the text, so a reader that ignores this arm silently
                    // drops every entity a document spells.
                    let name = text_of(reference.as_ref(), position, "an entity name")?;
                    let resolved = resolve_entity(&name).ok_or_else(|| {
                        codec_error(
                            position,
                            format_smolstr!(
                                "expected one of the predefined entities or a character \
                                 reference, got &{};",
                                quoted(&name)
                            ),
                        )
                    })?;
                    self.push_text(&resolved, position)?;
                }
                Event::DocType(declaration) => {
                    // The internal subset is where a document declares its own
                    // entities. Nothing resolves them here, and reading the
                    // document as though the declarations were not there would
                    // be a wrong answer rather than a refusal.
                    let text = text_of(declaration.as_ref(), position, "a doctype")?;
                    if text.contains('[') {
                        return Err(codec_error(
                            position,
                            "expected a document without an internal DTD subset, \
                             got one declaring entities",
                        ));
                    }
                }
                Event::Decl(declaration) => {
                    // Text crosses the boundary once, and it crossed it before
                    // this parser ran: these bytes are already UTF-8, decoded
                    // from whatever charset the handle's media type declared.
                    // A prolog naming a different charset is not a third
                    // opinion to weigh - it is a document saying it is not the
                    // one that was decoded, and reading it anyway would be
                    // mojibake with no error.
                    if let Some(Ok(encoding)) = declaration.encoding() {
                        let encoding = String::from_utf8_lossy(&encoding).into_owned();
                        if !is_utf8_name(&encoding) {
                            return Err(codec_error(
                                position,
                                format_smolstr!(
                                    "expected a document already decoded to UTF-8, got one \
                                     declaring {}; wrap the handle in that charset instead",
                                    quoted(&encoding)
                                ),
                            ));
                        }
                    }
                }
                Event::Comment(_) | Event::PI(_) => {}
                // A document that stops inside an element reads as a
                // shorter document to the parser, so the refusal is this
                // module's to raise.
                Event::Eof if !self.text.is_empty() => {
                    return Err(codec_error(
                        position,
                        "expected every element to close, got the end of the document",
                    ));
                }
                Event::Eof => return Ok(Step::End),
            }
        }
    }

    /// Open one element, charging it and reading its attributes.
    fn open(
        &mut self,
        element: BytesStart<'_>,
        namespace: Option<SmolStr>,
        position: usize,
        empty: bool,
    ) -> Result<Step> {
        if self.rooted {
            return Err(codec_error(
                position,
                "expected one document element, got content after it",
            ));
        }
        self.observe_depth(position)?;
        self.observe_node(position)?;
        let raw = text_of(element.name().as_ref(), position, "an element name")?;
        let local = text_of(element.local_name().as_ref(), position, "an element name")?;
        let (attributes, nil) = self.attributes(&element, position)?;
        if empty {
            if self.text.is_empty() {
                self.rooted = true;
            }
        } else {
            self.text.push(String::new());
        }
        Ok(Step::Open {
            name: Name {
                local,
                namespace,
                raw,
            },
            attributes,
            empty,
            nil,
        })
    }
}

/// Whether a declared encoding names the one these bytes are already in.
fn is_utf8_name(encoding: &str) -> bool {
    encoding.eq_ignore_ascii_case("utf-8") || encoding.eq_ignore_ascii_case("utf8")
}

/// Resolve one entity name to the text it stands for.
///
/// Only the five XML predefines and character references. There is no entity
/// table and there never is one: a document's own declarations are refused at
/// the doctype, so no input can grow this set and no expansion can nest.
fn resolve_entity(name: &str) -> Option<SmolStr> {
    match name {
        "lt" => return Some(SmolStr::new_static("<")),
        "gt" => return Some(SmolStr::new_static(">")),
        "amp" => return Some(SmolStr::new_static("&")),
        "apos" => return Some(SmolStr::new_static("'")),
        "quot" => return Some(SmolStr::new_static("\"")),
        _ => {}
    }
    let digits = name.strip_prefix('#')?;
    // `&#x41;` is a reference and `&#X41;` is not: the grammar spells the
    // marker in lower case, and a sign is not part of it either.
    let code = match digits.strip_prefix('x') {
        Some(hexadecimal) => {
            if !hexadecimal.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            u32::from_str_radix(hexadecimal, 16).ok()?
        }
        None => {
            if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            digits.parse::<u32>().ok()?
        }
    };
    // A reference to a scalar XML cannot carry is a fatal error, not a
    // character - the same rule the attribute path already applies.
    if !is_xml_char(code) {
        return None;
    }
    char::from_u32(code).map(|character| SmolStr::new(character.to_string()))
}
