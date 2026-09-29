//! The Open Packaging Conventions half of a workbook: which member of the
//! archive is which part.
//!
//! An `.xlsx` is a ZIP archive whose members are *parts*, wired together by
//! *relationships*: `_rels/.rels` names the office document, the workbook
//! part's own `.rels` names each worksheet, the shared strings and the
//! styles, and `[Content_Types].xml` states every part's content type. This
//! module owns those three documents - reading them into the part names the
//! workbook then asks the archive for, and writing them for the package this
//! crate produces - and nothing about what a part holds.
//!
//! Every reader here matches elements by local name, so the transitional
//! namespaces Excel writes and the strict ones its "Strict Open XML" save
//! writes read alike, as does a producer that prefixes its elements. A
//! relationship's target resolves as the conventions say: absolute from the
//! package root when it starts with `/`, else relative to the folder of the
//! part that states it; an external target is no part and is skipped.

use std::borrow::Cow;
use std::io::Write;

use quick_xml::events::{BytesRef, BytesStart, Event};
use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

/// The part that states every part's content type.
pub const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// The package's own relationships, which name the office document.
pub const ROOT_RELATIONSHIPS_PART: &str = "_rels/.rels";

/// Where this crate writes the workbook part.
pub const WORKBOOK_PART: &str = "xl/workbook.xml";

/// The workbook part's relationships: its sheets, strings and styles.
pub const WORKBOOK_RELATIONSHIPS_PART: &str = "xl/_rels/workbook.xml.rels";

/// Where this crate writes the shared string table.
pub const SHARED_STRINGS_PART: &str = "xl/sharedStrings.xml";

/// Where this crate writes the styles part.
pub const STYLES_PART: &str = "xl/styles.xml";

/// The content type of a relationships part.
pub const RELATIONSHIPS_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-package.relationships+xml";

/// The content type of the workbook part.
pub const WORKBOOK_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";

/// The content type of a worksheet part.
pub const WORKSHEET_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";

/// The content type of the shared string table.
pub const SHARED_STRINGS_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";

/// The content type of the styles part.
pub const STYLES_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";

/// The namespace of a relationships part.
pub const PACKAGE_RELATIONSHIPS_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships";

/// The namespace of `[Content_Types].xml`.
pub const CONTENT_TYPES_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/package/2006/content-types";

/// The relationship type naming the office document, from the package root.
pub const OFFICE_DOCUMENT_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

/// The relationship type naming a worksheet, from the workbook.
pub const WORKSHEET_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";

/// The relationship type naming the shared string table, from the workbook.
pub const SHARED_STRINGS_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";

/// The relationship type naming the styles part, from the workbook.
pub const STYLES_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";

/// The part a worksheet is written at, numbered from one.
pub fn worksheet_part(number: usize) -> SmolStr {
    format_smolstr!("xl/worksheets/sheet{number}.xml")
}

/// What a relationship's type says the target is, by the last segment of
/// the type URI - the same under the transitional and the strict families.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RelationshipKind {
    OfficeDocument,
    Worksheet,
    Chartsheet,
    Dialogsheet,
    SharedStrings,
    Styles,
    Other,
}

impl RelationshipKind {
    fn of(type_uri: &str) -> Self {
        match type_uri.rsplit('/').next().unwrap_or("") {
            "officeDocument" => Self::OfficeDocument,
            "worksheet" => Self::Worksheet,
            "chartsheet" => Self::Chartsheet,
            "dialogsheet" => Self::Dialogsheet,
            "sharedStrings" => Self::SharedStrings,
            "styles" => Self::Styles,
            _ => Self::Other,
        }
    }
}

/// One relationship a `.rels` part states, its target resolved to a member
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Relationship {
    pub(crate) id: SmolStr,
    pub(crate) kind: RelationshipKind,
    /// The member the target names, `None` for an external target.
    pub(crate) target: Option<SmolStr>,
}

/// The relationships one part states.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Relationships {
    entries: Vec<Relationship>,
}

impl Relationships {
    /// Read a `.rels` part stated by the part `source` - `""` for the
    /// package root - resolving every target against it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] when the bytes are not the part.
    pub(crate) fn from_xml(bytes: &[u8], source: &str) -> Result<Self> {
        let mut reader = super::styles::reader(bytes);
        let mut buffer = Vec::new();
        let mut entries = Vec::new();
        loop {
            let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| codec_error(position, error.to_string()))?;
            match event {
                Event::Start(ref start) | Event::Empty(ref start) => {
                    if local_name(start.name().as_ref()) == b"Relationship" {
                        let id = attribute(start, b"Id", position)?.unwrap_or_default();
                        let kind = attribute(start, b"Type", position)?
                            .map_or(RelationshipKind::Other, |uri| RelationshipKind::of(&uri));
                        let external = attribute(start, b"TargetMode", position)?
                            .is_some_and(|mode| mode.eq_ignore_ascii_case("External"));
                        let target = attribute(start, b"Target", position)?;
                        let target = match target {
                            Some(target) if !external => Some(resolve_target(source, &target)),
                            _ => None,
                        };
                        entries.push(Relationship {
                            id: SmolStr::new(id),
                            kind,
                            target,
                        });
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
        Ok(Self { entries })
    }

    /// The relationship with `id`.
    pub(crate) fn by_id(&self, id: &str) -> Option<&Relationship> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// The first relationship of `kind` naming a part; an external target
    /// is no part and is passed over.
    pub(crate) fn first_of(&self, kind: RelationshipKind) -> Option<&Relationship> {
        self.entries
            .iter()
            .find(|entry| entry.kind == kind && entry.target.is_some())
    }

    /// Every relationship, in document order.
    pub(crate) fn entries(&self) -> &[Relationship] {
        &self.entries
    }
}

/// Resolve a relationship target against the part that states it.
///
/// An absolute target starts with `/` and names the member from the package
/// root; a relative one is joined onto the folder of `source`, `.` and `..`
/// segments folded. Percent escapes in the target are decoded, since a
/// member name is the archive's own text.
pub(crate) fn resolve_target(source: &str, target: &str) -> SmolStr {
    let decoded = percent_decode(target);
    let mut segments: Vec<&str> = Vec::new();
    if !decoded.starts_with('/') {
        if let Some((folder, _)) = source.rsplit_once('/') {
            segments.extend(folder.split('/').filter(|segment| !segment.is_empty()));
        }
    }
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    SmolStr::new(segments.join("/"))
}

/// Decode `%HH` escapes, leaving anything else as it stands.
fn percent_decode(text: &str) -> Cow<'_, str> {
    if !text.contains('%') {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        // Two hex digits after the `%`, checked as bytes: a `%` before a
        // multi-byte character is kept as it stands, never sliced through.
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && bytes[at + 1].is_ascii_hexdigit()
            && bytes[at + 2].is_ascii_hexdigit()
        {
            let high = char::from(bytes[at + 1]).to_digit(16).unwrap_or(0);
            let low = char::from(bytes[at + 2]).to_digit(16).unwrap_or(0);
            decoded.push(u8::try_from(high * 16 + low).unwrap_or(0));
            at += 3;
            continue;
        }
        decoded.push(bytes[at]);
        at += 1;
    }
    Cow::Owned(String::from_utf8_lossy(&decoded).into_owned())
}

/// Write `_rels/.rels`: the one relationship to the workbook part.
///
/// # Errors
///
/// Returns the sink's failure.
pub(crate) fn write_root_relationships<W: Write>(writer: &mut W) -> Result<()> {
    write!(
        writer,
        "<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">\
         <Relationship Id=\"rId1\" Type=\"{OFFICE_DOCUMENT_RELATIONSHIP}\" Target=\"{WORKBOOK_PART}\"/>\
         </Relationships>"
    )?;
    Ok(())
}

/// Write the workbook's relationships: sheets `rId1..=n`, the styles at
/// `rId{n + 1}`, the shared strings at `rId{n + 2}`.
///
/// # Errors
///
/// Returns the sink's failure.
pub(crate) fn write_workbook_relationships<W: Write>(writer: &mut W, sheets: usize) -> Result<()> {
    write!(
        writer,
        "<Relationships xmlns=\"{PACKAGE_RELATIONSHIPS_NAMESPACE}\">"
    )?;
    for number in 1..=sheets {
        write!(
            writer,
            "<Relationship Id=\"rId{number}\" Type=\"{WORKSHEET_RELATIONSHIP}\" \
             Target=\"worksheets/sheet{number}.xml\"/>"
        )?;
    }
    write!(
        writer,
        "<Relationship Id=\"rId{}\" Type=\"{STYLES_RELATIONSHIP}\" Target=\"styles.xml\"/>\
         <Relationship Id=\"rId{}\" Type=\"{SHARED_STRINGS_RELATIONSHIP}\" Target=\"sharedStrings.xml\"/>\
         </Relationships>",
        sheets + 1,
        sheets + 2
    )?;
    Ok(())
}

/// Write `[Content_Types].xml` for a package of `sheets` worksheets.
///
/// # Errors
///
/// Returns the sink's failure.
pub(crate) fn write_content_types<W: Write>(writer: &mut W, sheets: usize) -> Result<()> {
    write!(
        writer,
        "<Types xmlns=\"{CONTENT_TYPES_NAMESPACE}\">\
         <Default Extension=\"rels\" ContentType=\"{RELATIONSHIPS_CONTENT_TYPE}\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/{WORKBOOK_PART}\" ContentType=\"{WORKBOOK_CONTENT_TYPE}\"/>"
    )?;
    for number in 1..=sheets {
        write!(
            writer,
            "<Override PartName=\"/xl/worksheets/sheet{number}.xml\" ContentType=\"{WORKSHEET_CONTENT_TYPE}\"/>"
        )?;
    }
    write!(
        writer,
        "<Override PartName=\"/{STYLES_PART}\" ContentType=\"{STYLES_CONTENT_TYPE}\"/>\
         <Override PartName=\"/{SHARED_STRINGS_PART}\" ContentType=\"{SHARED_STRINGS_CONTENT_TYPE}\"/>\
         </Types>"
    )?;
    Ok(())
}

/// The local name of a qualified element or attribute name: what follows
/// the prefix, or the whole name.
pub(crate) fn local_name(qualified: &[u8]) -> &[u8] {
    match qualified.iter().rposition(|byte| *byte == b':') {
        Some(colon) => &qualified[colon + 1..],
        None => qualified,
    }
}

/// The value of the attribute whose local name is `name`, unescaped.
///
/// # Errors
///
/// Returns [`Error::Codec`] when an attribute is malformed.
pub(crate) fn attribute<'a>(
    start: &'a BytesStart<'_>,
    name: &[u8],
    position: usize,
) -> Result<Option<Cow<'a, str>>> {
    // The duplicate check keeps every key seen so far, which is one
    // allocation per element read - three per cell - for a fault no part
    // Excel or this crate writes carries; the first attribute of a name is
    // the one read.
    for held in start.attributes().with_checks(false) {
        let held = held.map_err(|error| codec_error(position, error.to_string()))?;
        if local_name(held.key.as_ref()) == name {
            // Borrowed from the event's buffer unless an entity had to be
            // resolved or a newline normalized, so a cell's `r`, `t` and `s`
            // cost nothing to read.
            return held
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(Some)
                .map_err(|error| codec_error(position, error.to_string()));
        }
    }
    Ok(None)
}

/// The text of a `Text` or `CData` event, decoded.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the bytes are not UTF-8.
pub(crate) fn text_piece<'a>(
    content: std::result::Result<Cow<'a, str>, quick_xml::encoding::EncodingError>,
    position: usize,
) -> Result<Cow<'a, str>> {
    content.map_err(|error| codec_error(position, error.to_string()))
}

/// The text a `GeneralRef` event spells: a character reference or one of
/// the five predefined entities.
///
/// # Errors
///
/// Returns [`Error::Codec`] for an entity the document did not define.
pub(crate) fn reference_text(event: &Event<'_>, position: usize) -> Result<String> {
    let Event::GeneralRef(reference) = event else {
        return Ok(String::new());
    };
    resolve_reference(reference, position)
}

fn resolve_reference(reference: &BytesRef<'_>, position: usize) -> Result<String> {
    if let Some(character) = reference
        .resolve_char_ref()
        .map_err(|error| codec_error(position, error.to_string()))?
    {
        return Ok(character.to_string());
    }
    let name = reference
        .decode()
        .map_err(|error| codec_error(position, error.to_string()))?;
    quick_xml::escape::resolve_predefined_entity(&name)
        .map(str::to_owned)
        .ok_or_else(|| codec_error(position, format!("unknown entity reference `&{name};`")))
}

/// Report a malformed part at a byte position.
pub(crate) fn codec_error(position: usize, reason: impl Into<SmolStr>) -> Error {
    Error::Codec {
        format: "xlsx",
        position,
        reason: reason.into(),
    }
}

/// The edits one pass over an OPC document applies: subtrees dropped,
/// fragments inserted, counts patched - so a part another producer wrote
/// keeps everything this crate does not speak for.
pub(crate) struct Rewrite<'a> {
    /// Whether an element - and, for a start tag, its whole subtree - is
    /// left out.
    pub(crate) skip: &'a dyn Fn(&BytesStart<'_>) -> bool,
    /// How much to add to the `count` attribute of the element named.
    pub(crate) patch_count: &'a dyn Fn(&[u8]) -> Option<u32>,
    /// The fragment to write just before the end tag of the element named.
    pub(crate) before_end: Fragment<'a>,
    /// The fragment to write just after the start tag of the element named.
    pub(crate) after_start: Fragment<'a>,
    /// The fragment to write just before the start tag of the element named.
    pub(crate) before_start: Fragment<'a>,
    /// An attribute to set on the element named, replacing one it states.
    pub(crate) set_attribute: Attribute<'a>,
}

/// A rewrite hook answering the fragment to write at the element named.
pub(crate) type Fragment<'a> = &'a dyn Fn(&[u8]) -> Option<String>;

/// A rewrite hook answering the attribute to set on the element named.
pub(crate) type Attribute<'a> = &'a dyn Fn(&[u8]) -> Option<(&'static str, String)>;

/// Apply `edits` to the document `bytes`, event by event.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the bytes are not well-formed XML.
pub(crate) fn rewrite(bytes: &[u8], edits: &Rewrite<'_>) -> Result<Vec<u8>> {
    use quick_xml::Writer;

    let mut reader = super::styles::reader(bytes);
    let mut buffer = Vec::new();
    let mut writer = Writer::new(Vec::with_capacity(bytes.len() + 512));
    let mut skipping = 0_usize;
    loop {
        let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| codec_error(position, error.to_string()))?;
        match event {
            Event::Start(ref start) => {
                if skipping > 0 || (edits.skip)(start) {
                    skipping += 1;
                } else {
                    let qualified = start.name();
                    let name = local_name(qualified.as_ref()).to_vec();
                    if let Some(fragment) = (edits.before_start)(&name) {
                        writer.get_mut().extend_from_slice(fragment.as_bytes());
                    }
                    let patched = patch_start(
                        start,
                        (edits.patch_count)(&name),
                        (edits.set_attribute)(&name),
                        position,
                    )?;
                    writer
                        .write_event(Event::Start(patched))
                        .map_err(|error| codec_error(position, error.to_string()))?;
                    if let Some(fragment) = (edits.after_start)(&name) {
                        writer.get_mut().extend_from_slice(fragment.as_bytes());
                    }
                }
            }
            Event::Empty(ref start) => {
                if skipping == 0 && !(edits.skip)(start) {
                    let qualified = start.name();
                    let name = local_name(qualified.as_ref()).to_vec();
                    if let Some(fragment) = (edits.before_start)(&name) {
                        writer.get_mut().extend_from_slice(fragment.as_bytes());
                    }
                    let set = (edits.set_attribute)(&name);
                    // An empty element the edits add children to opens.
                    let fragment = (edits.after_start)(&name).or_else(|| (edits.before_end)(&name));
                    if let Some(fragment) = fragment {
                        let patched =
                            patch_start(start, (edits.patch_count)(&name), set, position)?;
                        let end = patched.to_end().into_owned();
                        writer
                            .write_event(Event::Start(patched))
                            .map_err(|error| codec_error(position, error.to_string()))?;
                        writer.get_mut().extend_from_slice(fragment.as_bytes());
                        writer
                            .write_event(Event::End(end))
                            .map_err(|error| codec_error(position, error.to_string()))?;
                    } else if set.is_some() {
                        let patched = patch_start(start, None, set, position)?;
                        writer
                            .write_event(Event::Empty(patched))
                            .map_err(|error| codec_error(position, error.to_string()))?;
                    } else {
                        writer
                            .write_event(Event::Empty(start.borrow()))
                            .map_err(|error| codec_error(position, error.to_string()))?;
                    }
                }
            }
            Event::End(ref end) => {
                if skipping > 0 {
                    skipping -= 1;
                } else {
                    let qualified = end.name();
                    if let Some(fragment) = (edits.before_end)(local_name(qualified.as_ref())) {
                        writer.get_mut().extend_from_slice(fragment.as_bytes());
                    }
                    writer
                        .write_event(Event::End(end.borrow()))
                        .map_err(|error| codec_error(position, error.to_string()))?;
                }
            }
            Event::Eof => break,
            other => {
                if skipping == 0 {
                    writer
                        .write_event(other)
                        .map_err(|error| codec_error(position, error.to_string()))?;
                }
            }
        }
        buffer.clear();
    }
    Ok(writer.into_inner())
}

/// A start tag with its `count` attribute raised by `delta` and the
/// attribute `set` stated, or as it stands when neither is asked.
fn patch_start(
    start: &BytesStart<'_>,
    delta: Option<u32>,
    set: Option<(&'static str, String)>,
    position: usize,
) -> Result<BytesStart<'static>> {
    if delta.is_none() && set.is_none() {
        return Ok(start.to_owned());
    }
    let mut patched = BytesStart::new(String::from_utf8_lossy(start.name().as_ref()).into_owned());
    let mut counted = false;
    let mut stated = false;
    for held in start.attributes() {
        let held = held.map_err(|error| codec_error(position, error.to_string()))?;
        let key = local_name(held.key.as_ref());
        if let Some(delta) = delta.filter(|_| key == b"count") {
            let count: u32 = String::from_utf8_lossy(&held.value)
                .trim()
                .parse()
                .unwrap_or(0);
            patched.push_attribute(("count", (count + delta).to_string().as_str()));
            counted = true;
        } else if let Some((name, value)) = set.as_ref().filter(|(name, _)| key == name.as_bytes())
        {
            patched.push_attribute((*name, value.as_str()));
            stated = true;
        } else {
            let key = String::from_utf8_lossy(held.key.as_ref()).into_owned();
            let value = String::from_utf8_lossy(&held.value).into_owned();
            patched.push_attribute((key.as_str(), value.as_str()));
        }
    }
    if let Some(delta) = delta.filter(|_| !counted) {
        patched.push_attribute(("count", delta.to_string().as_str()));
    }
    if let Some((name, value)) = set.filter(|_| !stated) {
        patched.push_attribute((name, value.as_str()));
    }
    Ok(patched)
}

/// Whether the document holds an element of local name `name`.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the bytes are not well-formed XML.
pub(crate) fn has_element(bytes: &[u8], name: &[u8]) -> Result<bool> {
    let mut reader = super::styles::reader(bytes);
    let mut buffer = Vec::new();
    loop {
        let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| codec_error(position, error.to_string()))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                if local_name(start.name().as_ref()) == name {
                    return Ok(true);
                }
            }
            Event::Eof => return Ok(false),
            _ => {}
        }
        buffer.clear();
    }
}

/// One `<Override>` for `[Content_Types].xml`.
pub(crate) fn override_element(part: &str, content_type: &str) -> String {
    format!("<Override PartName=\"/{part}\" ContentType=\"{content_type}\"/>")
}

/// One `<Relationship>` for a `.rels` part, its target relative to `xl/`.
pub(crate) fn relationship_element(id: &str, kind: &str, target: &str) -> String {
    format!("<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>")
}
