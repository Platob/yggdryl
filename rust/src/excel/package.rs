//! The Open Packaging Conventions half of a workbook: which member of the
//! archive is which part.
//!
//! An `.xlsx` is a ZIP archive whose members are *parts*, wired together by
//! *relationships*: `_rels/.rels` names the office document, the workbook
//! part's own `.rels` names each worksheet, the shared strings and the
//! styles, and `[Content_Types].xml` states every part's content type. This
//! module owns those documents - reading them into the part names the
//! workbook then asks the archive for, the template a workbook built from
//! nothing starts as, and `rewrite`, the one pass every document the
//! crate writes back goes through - and nothing about what a part holds.
//!
//! The transitional namespace Excel writes and the strict one its "Strict
//! Open XML" save writes are both recognized, including prefixed elements. A
//! relationship's target resolves as the conventions say: absolute from the
//! package root when it starts with `/`, else relative to the folder of the
//! part that states it; an external target is no part and is skipped.

use std::borrow::Cow;

use quick_xml::events::{BytesRef, BytesStart, Event};
use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

/// The namespace pair of generated SpreadsheetML parts. This preserves the
/// package's family; it does not certify every Strict schema constraint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum NamespaceFamily {
    #[default]
    Transitional,
    Strict,
}

impl NamespaceFamily {
    /// Resolve one decoded root namespace at the workbook intake boundary.
    pub(crate) fn from_namespace(namespace: &str) -> Option<Self> {
        match namespace {
            super::NAMESPACE => Some(Self::Transitional),
            super::STRICT_NAMESPACE => Some(Self::Strict),
            _ => None,
        }
    }

    /// The namespace of generated SpreadsheetML elements.
    pub(crate) const fn namespace(self) -> &'static str {
        match self {
            Self::Transitional => super::NAMESPACE,
            Self::Strict => super::STRICT_NAMESPACE,
        }
    }

    /// The namespace of relationship attributes and the base of Type URIs.
    pub(crate) const fn relationships_namespace(self) -> &'static str {
        match self {
            Self::Transitional => super::RELATIONSHIPS_NAMESPACE,
            Self::Strict => super::STRICT_RELATIONSHIPS_NAMESPACE,
        }
    }
}

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

/// The content type of the calculation chain.
pub const CALC_CHAIN_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml";

/// Where Excel writes the calculation chain, when no relationship says.
pub const CALC_CHAIN_PART: &str = "xl/calcChain.xml";

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

/// The relationship type naming a chart sheet, from the workbook.
pub const CHARTSHEET_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet";

/// The relationship type naming a dialog sheet, from the workbook.
pub const DIALOGSHEET_RELATIONSHIP: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/dialogsheet";

/// The part a worksheet is written at, numbered from one.
pub fn worksheet_part(number: usize) -> SmolStr {
    format_smolstr!("xl/worksheets/sheet{number}.xml")
}

/// `[Content_Types].xml` of a workbook built from nothing: the two defaults
/// and the workbook part; a save adds what it writes.
pub(crate) const TEMPLATE_CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
</Types>";

/// `_rels/.rels` of a workbook built from nothing: the office document.
pub(crate) const TEMPLATE_ROOT_RELATIONSHIPS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
</Relationships>";

/// The workbook part of a workbook built from nothing: no sheet yet, and the
/// two elements a save states facts on.
pub(crate) const TEMPLATE_WORKBOOK: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
<workbookPr/><sheets/><calcPr/></workbook>";

/// The workbook's relationships in a workbook built from nothing: none.
pub(crate) const TEMPLATE_WORKBOOK_RELATIONSHIPS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>";

/// The four template documents, by member name, in the order written.
pub(crate) const TEMPLATE: [(&str, &str); 4] = [
    (CONTENT_TYPES_PART, TEMPLATE_CONTENT_TYPES),
    (ROOT_RELATIONSHIPS_PART, TEMPLATE_ROOT_RELATIONSHIPS),
    (WORKBOOK_PART, TEMPLATE_WORKBOOK),
    (WORKBOOK_RELATIONSHIPS_PART, TEMPLATE_WORKBOOK_RELATIONSHIPS),
];

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
    PivotTable,
    PivotCacheDefinition,
    PivotCacheRecords,
    Theme,
    Table,
    Drawing,
    Chart,
    Comments,
    VmlDrawing,
    CalcChain,
    QueryTable,
    ThreadedComment,
    SlicerCache,
    TimelineCache,
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
            "pivotTable" => Self::PivotTable,
            "pivotCacheDefinition" => Self::PivotCacheDefinition,
            "pivotCacheRecords" => Self::PivotCacheRecords,
            "theme" => Self::Theme,
            "table" => Self::Table,
            "drawing" => Self::Drawing,
            "chart" => Self::Chart,
            "comments" => Self::Comments,
            "vmlDrawing" => Self::VmlDrawing,
            "calcChain" => Self::CalcChain,
            "queryTable" => Self::QueryTable,
            "threadedComment" => Self::ThreadedComment,
            "slicerCache" => Self::SlicerCache,
            "timelineCache" => Self::TimelineCache,
            _ => Self::Other,
        }
    }

    /// Whether the relationship names a tab of the workbook.
    pub(crate) const fn is_sheet(self) -> bool {
        matches!(self, Self::Worksheet | Self::Chartsheet | Self::Dialogsheet)
    }
}

/// One relationship a `.rels` part states, its target resolved to a member
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Relationship {
    pub(crate) id: SmolStr,
    pub(crate) kind: RelationshipKind,
    /// Normalized authored Type URI; a suffix classification alone cannot
    /// authorize a selected pivot or mistake a vendor relationship for one.
    pub(crate) type_uri: SmolStr,
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
        let mut reader = quick_xml::NsReader::from_reader(bytes);
        reader.config_mut().trim_text(false);
        let mut buffer = Vec::new();
        let mut entries = Vec::new();
        let mut depth = 0usize;
        let mut root = false;
        loop {
            let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| codec_error(position, error.to_string()))?;
            match event {
                Event::Start(ref start) | Event::Empty(ref start) => {
                    let (namespace, name) = reader.resolver().resolve_element(start.name());
                    let opc = match namespace {
                        quick_xml::name::ResolveResult::Bound(uri) => {
                            let uri = std::str::from_utf8(uri.as_ref())
                                .map_err(|error| codec_error(position, error.to_string()))?;
                            quick_xml::escape::unescape(uri)
                                .map_err(|error| codec_error(position, error.to_string()))?
                                == PACKAGE_RELATIONSHIPS_NAMESPACE
                        }
                        _ => false,
                    };
                    if depth == 0 {
                        if root || !opc || name.as_ref() != b"Relationships" {
                            return Err(codec_error(
                                position,
                                "expected one OPC Relationships root",
                            ));
                        }
                        root = true;
                    } else if depth == 1 && opc && name.as_ref() == b"Relationship" {
                        // Only direct OPC entries own parts. Foreign wrappers,
                        // local-name lookalikes and qualified attributes carry
                        // no relationship identity.
                        let (mut id, mut type_uri, mut mode, mut target) = (None, None, None, None);
                        for attribute in start.attributes() {
                            let attribute = attribute
                                .map_err(|error| codec_error(position, error.to_string()))?;
                            let value = match attribute.key.as_ref() {
                                b"Id" => &mut id,
                                b"Type" => &mut type_uri,
                                b"TargetMode" => &mut mode,
                                b"Target" => &mut target,
                                _ => continue,
                            };
                            *value = Some(
                                attribute
                                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                    .map_err(|error| codec_error(position, error.to_string()))?,
                            );
                        }
                        let id = id.unwrap_or_default();
                        let type_uri = type_uri.unwrap_or_default();
                        let kind = RelationshipKind::of(&type_uri);
                        let external =
                            mode.is_some_and(|mode| mode.eq_ignore_ascii_case("External"));
                        let target = match target {
                            Some(target) if !external => Some(resolve_target(source, &target)),
                            _ => None,
                        };
                        entries.push(Relationship {
                            id: SmolStr::new(id),
                            kind,
                            type_uri: SmolStr::new(type_uri),
                            target,
                        });
                    }
                    if matches!(event, Event::Start(_)) {
                        depth += 1;
                    }
                }
                Event::End(_) => depth = depth.saturating_sub(1),
                Event::Eof => {
                    if !root || depth != 0 {
                        return Err(codec_error(
                            position,
                            "expected a complete OPC Relationships document",
                        ));
                    }
                    break;
                }
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

    /// The first relationship naming the member `part`.
    pub(crate) fn to_part(&self, part: &str) -> Option<&Relationship> {
        self.entries
            .iter()
            .find(|entry| entry.target.as_deref() == Some(part))
    }

    /// The number the first relationship added to these takes: `rId` past
    /// every numbered id already stated, `None` when one states the highest
    /// number there is.
    pub(crate) fn next_id(&self) -> Option<usize> {
        self.entries
            .iter()
            .filter_map(|relationship| relationship.id.strip_prefix("rId")?.parse::<usize>().ok())
            .max()
            .unwrap_or(0)
            .checked_add(1)
    }
}

/// The `.rels` part of `part`: `xl/_rels/workbook.xml.rels` for
/// `xl/workbook.xml`.
pub(crate) fn relationships_part_of(part: &str) -> SmolStr {
    match part.rsplit_once('/') {
        Some((folder, name)) => format_smolstr!("{folder}/_rels/{name}.rels"),
        None => format_smolstr!("_rels/{part}.rels"),
    }
}

/// The part whose relationships the `.rels` member `member` states, the
/// empty name for the package's own; `None` for a member that is none.
pub(crate) fn source_of_relationships(member: &str) -> Option<SmolStr> {
    let name = member.strip_suffix(".rels")?;
    let (folder, file) = name.rsplit_once('/')?;
    let folder = folder.strip_suffix("_rels")?.trim_end_matches('/');
    Some(if folder.is_empty() {
        SmolStr::new(file)
    } else {
        format_smolstr!("{folder}/{file}")
    })
}

/// The folder of a part, empty at the package root.
pub(crate) fn folder_of(part: &str) -> &str {
    part.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// A part's name relative to the folder `base`, as a relationship target
/// spells it: `worksheets/sheet1.xml` from `xl`, `../docProps/app.xml` from
/// `xl` too, each segment the two share once.
pub(crate) fn relative_to(base: &str, part: &str) -> String {
    let base: Vec<&str> = base
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let target: Vec<&str> = part
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let shared = base
        .iter()
        .zip(&target)
        .take_while(|(held, wanted)| held == wanted)
        .count()
        .min(target.len().saturating_sub(1));
    let mut segments: Vec<&str> = vec![".."; base.len() - shared.min(base.len())];
    segments.extend(&target[shared..]);
    segments.join("/")
}

/// Escape text for an attribute value.
pub(crate) fn escape_attribute(text: &str) -> String {
    let mut escaped = Vec::with_capacity(text.len());
    if crate::xml::write_attribute_text(&mut escaped, text).is_err() {
        return text.to_owned();
    }
    String::from_utf8(escaped).unwrap_or_else(|_| text.to_owned())
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
    if !decoded.starts_with('/')
        && let Some((folder, _)) = source.rsplit_once('/')
    {
        segments.extend(folder.split('/').filter(|segment| !segment.is_empty()));
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

/// The value of an exact lexical attribute name, unescaped. Metadata
/// URI attributes must not bind a foreign qualified name.
pub(crate) fn exact_attribute<'a>(
    start: &'a BytesStart<'_>,
    name: &[u8],
    position: usize,
) -> Result<Option<Cow<'a, str>>> {
    let mut found = None;
    for held in start.attributes() {
        let held = held.map_err(|error| codec_error(position, error.to_string()))?;
        if held.key.as_ref() == name {
            found = Some(
                held.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|error| codec_error(position, error.to_string()))?,
            );
        }
    }
    Ok(found)
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
/// fragments inserted, attributes set, counts patched - so a part another
/// producer wrote keeps everything this crate does not speak for.
///
/// Every hook names an element by its local name, so a part in the strict
/// namespaces, or one that prefixes its elements, is edited alike: a
/// fragment is written unprefixed, and each element in it that names no
/// prefix of its own takes the prefix of the element it is written beside
/// or inside, the namespace the fragment's elements share with it.
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
    /// The attributes to set (`Some`) or remove (`None`) on the element
    /// named; one it sets that the element lacks is added after the others.
    pub(crate) attributes: Attributes<'a>,
    /// Fragments each written once, where a schema's list order puts them.
    pub(crate) insert: &'a [Insertion<'a>],
    /// The text to write in place of the character data of a leaf element,
    /// asked with its local name and the text it holds, answering `None` to
    /// keep the text as written. With no hook, text is written as it is
    /// read and never held: a rewrite of a large table costs nothing per
    /// `<t>`.
    pub(crate) text: Option<Text<'a>>,
}

/// A fragment a [`Rewrite`] writes once inside `parent`, just before the
/// first of its children named in `before_first_of` - the elements the
/// schema orders after the fragment - or, when it has none of them, just
/// before `parent`'s end tag.
pub(crate) struct Insertion<'a> {
    pub(crate) fragment: String,
    pub(crate) parent: &'a [u8],
    pub(crate) before_first_of: &'a [&'a [u8]],
}

/// A rewrite hook answering the fragment to write at the element named.
pub(crate) type Fragment<'a> = &'a dyn Fn(&[u8]) -> Option<String>;

/// One attribute a rewrite states: its name and its value, `None` to remove
/// it.
pub(crate) type AttributeEdit = (&'static str, Option<String>);

/// A rewrite hook answering the attributes to set or remove on the element
/// named.
pub(crate) type Attributes<'a> = &'a dyn Fn(&[u8]) -> Vec<AttributeEdit>;

/// A rewrite hook answering the text a leaf element named holds instead of
/// the text it holds.
pub(crate) type Text<'a> = &'a dyn Fn(&[u8], &str) -> Option<String>;

/// Keep every element.
fn keep(_: &BytesStart<'_>) -> bool {
    false
}

/// Patch no count.
fn no_count(_: &[u8]) -> Option<u32> {
    None
}

/// Write no fragment.
fn no_fragment(_: &[u8]) -> Option<String> {
    None
}

/// Set no attribute.
fn no_attributes(_: &[u8]) -> Vec<AttributeEdit> {
    Vec::new()
}

impl Default for Rewrite<'_> {
    /// The rewrite that changes nothing: every hook a no-op.
    fn default() -> Self {
        Self {
            skip: &keep,
            patch_count: &no_count,
            before_end: &no_fragment,
            after_start: &no_fragment,
            before_start: &no_fragment,
            attributes: &no_attributes,
            insert: &[],
            text: None,
        }
    }
}

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
    // The elements written and still open, so an insertion knows the parent
    // of the element it may go before, and the prefix its fragment takes.
    let mut open: Vec<Opened> = Vec::new();
    let mut inserted = vec![false; edits.insert.len()];
    // The character data of the innermost open element, held until it is
    // known to be a leaf's: as written, and as text.
    let mut pending: Vec<Event<'static>> = Vec::new();
    let mut pending_text = String::new();
    // Whether the innermost open element has held no child element yet.
    let mut leaf = false;
    let failed =
        |position: usize| move |error: std::io::Error| codec_error(position, error.to_string());
    let flush = |writer: &mut quick_xml::Writer<Vec<u8>>,
                 pending: &mut Vec<Event<'static>>,
                 pending_text: &mut String,
                 position: usize|
     -> Result<()> {
        for event in pending.drain(..) {
            writer.write_event(event).map_err(failed(position))?;
        }
        pending_text.clear();
        Ok(())
    };
    loop {
        let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| codec_error(position, error.to_string()))?;
        match event {
            Event::Start(ref start) => {
                if skipping > 0 {
                    skipping += 1;
                } else {
                    flush(&mut writer, &mut pending, &mut pending_text, position)?;
                    leaf = false;
                    let element = Opened::of(start.name().as_ref());
                    due(
                        &mut writer,
                        edits.insert,
                        &mut inserted,
                        open.last(),
                        Some(element.name()),
                    )?;
                    if (edits.skip)(start) {
                        skipping = 1;
                    } else {
                        if let Some(fragment) = (edits.before_start)(element.name()) {
                            put(&mut writer, &fragment, element.prefix())?;
                        }
                        let patched = patch_start(
                            start,
                            (edits.patch_count)(element.name()),
                            &(edits.attributes)(element.name()),
                            position,
                        )?;
                        writer
                            .write_event(Event::Start(patched))
                            .map_err(failed(position))?;
                        if let Some(fragment) = (edits.after_start)(element.name()) {
                            put(&mut writer, &fragment, element.prefix())?;
                        }
                        open.push(element);
                        leaf = true;
                    }
                }
            }
            Event::Empty(ref start) => {
                if skipping > 0 {
                    buffer.clear();
                    continue;
                }
                flush(&mut writer, &mut pending, &mut pending_text, position)?;
                leaf = false;
                let element = Opened::of(start.name().as_ref());
                due(
                    &mut writer,
                    edits.insert,
                    &mut inserted,
                    open.last(),
                    Some(element.name()),
                )?;
                if (edits.skip)(start) {
                    buffer.clear();
                    continue;
                }
                if let Some(fragment) = (edits.before_start)(element.name()) {
                    put(&mut writer, &fragment, element.prefix())?;
                }
                let patched = patch_start(
                    start,
                    (edits.patch_count)(element.name()),
                    &(edits.attributes)(element.name()),
                    position,
                )?;
                // An empty element the edits add children to opens.
                let after = (edits.after_start)(element.name());
                let before = (edits.before_end)(element.name());
                let children = edits
                    .insert
                    .iter()
                    .zip(&inserted)
                    .any(|(insertion, done)| !done && insertion.parent == element.name());
                if after.is_some() || before.is_some() || children {
                    let end = patched.to_end().into_owned();
                    writer
                        .write_event(Event::Start(patched))
                        .map_err(failed(position))?;
                    if let Some(fragment) = after {
                        put(&mut writer, &fragment, element.prefix())?;
                    }
                    due(
                        &mut writer,
                        edits.insert,
                        &mut inserted,
                        Some(&element),
                        None,
                    )?;
                    if let Some(fragment) = before {
                        put(&mut writer, &fragment, element.prefix())?;
                    }
                    writer
                        .write_event(Event::End(end))
                        .map_err(failed(position))?;
                } else {
                    writer
                        .write_event(Event::Empty(patched))
                        .map_err(failed(position))?;
                }
            }
            Event::End(ref end) => {
                if skipping > 0 {
                    skipping -= 1;
                } else {
                    let element = Opened::of(end.name().as_ref());
                    // A leaf's text, replaced where the edits say.
                    let replaced = match edits.text {
                        Some(text) if leaf => text(element.name(), &pending_text),
                        _ => None,
                    };
                    leaf = false;
                    match replaced {
                        Some(replacement) => {
                            pending.clear();
                            pending_text.clear();
                            // Escaped as a producer escapes text: `<`, `>` and
                            // `&`, quotes as they stand.
                            writer
                                .write_event(Event::Text(
                                    quick_xml::events::BytesText::from_escaped(
                                        quick_xml::escape::partial_escape(&replacement),
                                    ),
                                ))
                                .map_err(failed(position))?;
                        }
                        None => flush(&mut writer, &mut pending, &mut pending_text, position)?,
                    }
                    due(
                        &mut writer,
                        edits.insert,
                        &mut inserted,
                        Some(&element),
                        None,
                    )?;
                    if let Some(fragment) = (edits.before_end)(element.name()) {
                        put(&mut writer, &fragment, element.prefix())?;
                    }
                    writer
                        .write_event(Event::End(end.borrow()))
                        .map_err(failed(position))?;
                    open.pop();
                }
            }
            Event::Eof => {
                flush(&mut writer, &mut pending, &mut pending_text, position)?;
                // A part cut short inside an element would lose every edit
                // due at an end tag it never reaches.
                if let Some(element) = open.last() {
                    return Err(codec_error(
                        position,
                        format_smolstr!(
                            "expected the end tag of <{}>, got the end of the part",
                            element.qualified
                        ),
                    ));
                }
                break;
            }
            other => {
                if skipping == 0 && edits.text.is_none() {
                    writer.write_event(other).map_err(failed(position))?;
                } else if skipping == 0 {
                    match &other {
                        Event::Text(text) => {
                            pending_text.push_str(&text_piece(text.xml10_content(), position)?);
                        }
                        Event::CData(text) => {
                            pending_text.push_str(&text_piece(text.xml10_content(), position)?);
                        }
                        Event::GeneralRef(_) => {
                            pending_text.push_str(&reference_text(&other, position)?);
                        }
                        _ => {}
                    }
                    pending.push(other.into_owned());
                }
            }
        }
        buffer.clear();
    }
    Ok(writer.into_inner())
}

/// An element a [`rewrite`] has written the start tag of: its qualified
/// name, and the local name every hook is asked by.
struct Opened {
    /// Inline for a name of up to 23 bytes, so a rewrite costs nothing per
    /// element.
    qualified: SmolStr,
}

impl Opened {
    fn of(qualified: &[u8]) -> Self {
        Self {
            qualified: SmolStr::new(String::from_utf8_lossy(qualified)),
        }
    }

    /// The local name every hook is asked by.
    fn name(&self) -> &[u8] {
        local_name(self.qualified.as_bytes())
    }

    /// The prefix the element is written under, `None` for an unprefixed
    /// one.
    fn prefix(&self) -> Option<&[u8]> {
        let qualified = self.qualified.as_bytes();
        qualified
            .iter()
            .rposition(|byte| *byte == b':')
            .map(|colon| &qualified[..colon])
    }
}

/// Write `fragment`, each element in it that names no prefix taking
/// `prefix`.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the fragment is not well-formed XML.
fn put(
    writer: &mut quick_xml::Writer<Vec<u8>>,
    fragment: &str,
    prefix: Option<&[u8]>,
) -> Result<()> {
    let Some(prefix) = prefix else {
        writer.get_mut().extend_from_slice(fragment.as_bytes());
        return Ok(());
    };
    let named = |name: &[u8]| -> Vec<u8> {
        if name.contains(&b':') {
            return name.to_vec();
        }
        let mut qualified = Vec::with_capacity(prefix.len() + 1 + name.len());
        qualified.extend_from_slice(prefix);
        qualified.push(b':');
        qualified.extend_from_slice(name);
        qualified
    };
    let mut reader = super::styles::reader(fragment.as_bytes());
    let mut buffer = Vec::new();
    loop {
        let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| codec_error(position, error.to_string()))?;
        let written = match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let qualified = named(start.name().as_ref());
                let mut renamed = BytesStart::new(String::from_utf8_lossy(&qualified).into_owned());
                for attribute in start.attributes() {
                    renamed.push_attribute(
                        attribute.map_err(|error| codec_error(position, error.to_string()))?,
                    );
                }
                if matches!(event, Event::Start(_)) {
                    writer.write_event(Event::Start(renamed))
                } else {
                    writer.write_event(Event::Empty(renamed))
                }
            }
            Event::End(ref end) => {
                let qualified = named(end.name().as_ref());
                writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
                    String::from_utf8_lossy(&qualified).into_owned(),
                )))
            }
            Event::Eof => return Ok(()),
            other => writer.write_event(other),
        };
        written.map_err(|error| codec_error(position, error.to_string()))?;
        buffer.clear();
    }
}

/// Write every insertion due inside `parent` before its child `name`, or -
/// `name` absent - every one still due at `parent`'s end.
fn due(
    writer: &mut quick_xml::Writer<Vec<u8>>,
    insert: &[Insertion<'_>],
    inserted: &mut [bool],
    parent: Option<&Opened>,
    name: Option<&[u8]>,
) -> Result<()> {
    let Some(parent) = parent else {
        return Ok(());
    };
    for (insertion, done) in insert.iter().zip(inserted.iter_mut()) {
        if *done || insertion.parent != parent.name() {
            continue;
        }
        if name.is_none_or(|name| insertion.before_first_of.contains(&name)) {
            put(writer, &insertion.fragment, parent.prefix())?;
            *done = true;
        }
    }
    Ok(())
}

/// What an [`edit_document`] pass does with one start tag.
pub(crate) enum Tag {
    /// Kept as written.
    Keep,
    /// Left out, with everything inside it.
    Drop,
    /// Kept, the attributes named by exact QName (`s` distinct from `x:s`)
    /// set to the value given or, for `None`, removed.
    Set(Vec<(SmolStr, Option<String>)>),
}

/// The hooks of an [`edit_document`] pass. Each is asked with `path`, the
/// local names of the elements open from the document's root down to the
/// element asked about, and each answers what changes; a pass whose hooks
/// change nothing answers no document.
pub(crate) trait Edits {
    /// A start tag, with its attributes by exact QName and normalized
    /// values, and its resolved element namespace. Namespace declaration
    /// values have already been normalized, including character references.
    fn start(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<Tag> {
        let _ = (path, attributes, namespace);
        Ok(Tag::Keep)
    }

    /// The text of an element holding no element, unescaped: its new text,
    /// `None` to keep it as written.
    fn text(&mut self, path: &[SmolStr], text: &str) -> Result<Option<String>> {
        let _ = (path, text);
        Ok(None)
    }

    /// An element's end, `dropped` of its `children` elements left out,
    /// `attributes` what its start tag stated: [`Tag::Drop`] to leave the
    /// element out after all, else the attributes to set on that tag.
    fn end(
        &mut self,
        path: &[SmolStr],
        children: usize,
        dropped: usize,
        attributes: &[(SmolStr, String)],
    ) -> Tag {
        let _ = (path, children, dropped, attributes);
        Tag::Keep
    }

    /// Replace one complete element after its children have been inspected.
    /// A caller can refuse an unsupported descendant before supplying bytes.
    /// Everything outside that element keeps its original lexical bytes.
    fn replacement(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        qualified: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        let _ = (path, attributes, qualified);
        Ok(None)
    }

    /// Keep an untouched subtree as raw bytes without visiting its children.
    /// The start and end hooks are not called for this element.
    fn keep_subtree(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        namespace: &quick_xml::name::ResolveResult<'_>,
    ) -> Result<bool> {
        let _ = (path, attributes, namespace);
        Ok(false)
    }

    /// A borrowed-tag fast path for an unchanged subtree. `path` is its
    /// parent's path. A caller may return `Some(bytes)` only when the tag has
    /// no namespace declarations, so `namespace` is its complete resolution.
    /// The returned bytes are inserted before the raw subtree.
    fn keep_raw_subtree(
        &mut self,
        path: &[SmolStr],
        tag: &BytesStart<'_>,
        namespace: &quick_xml::name::ResolveResult<'_>,
        position: usize,
    ) -> Result<Option<Vec<u8>>> {
        let _ = (path, tag, namespace, position);
        Ok(None)
    }

    /// Inspect a subtree selected by `keep_raw_subtree` after its complete
    /// byte range is known. The ordinary fast path does no work here.
    fn raw_subtree(&mut self, path: &[SmolStr], tag: &BytesStart<'_>, raw: &[u8]) -> Result<()> {
        let _ = (path, tag, raw);
        Ok(())
    }

    /// Bytes inserted immediately before an element's start tag.
    fn before(
        &mut self,
        path: &[SmolStr],
        attributes: &[(SmolStr, String)],
        namespace: &quick_xml::name::ResolveResult<'_>,
    ) -> Result<Option<Vec<u8>>> {
        let _ = (path, attributes, namespace);
        Ok(None)
    }

    /// Bytes inserted before a nonempty element's end tag.
    fn before_end(&mut self, path: &[SmolStr]) -> Result<Option<Vec<u8>>> {
        let _ = path;
        Ok(None)
    }

    /// Bytes inserted into a self-closing element, turning it into a pair.
    fn inside_empty(&mut self, path: &[SmolStr]) -> Result<Option<Vec<u8>>> {
        let _ = path;
        Ok(None)
    }
}

/// An element an [`edit_document`] pass has open.
struct Held {
    tag: BytesStart<'static>,
    attributes: Vec<(SmolStr, String)>,
    /// Where its start tag lies.
    tag_range: std::ops::Range<usize>,
    /// First splice inside this element, after its before-start insertion.
    splice_start: usize,
    set: Vec<(SmolStr, Option<String>)>,
    children: usize,
    text: String,
    dropped: usize,
}

/// `bytes` with `edits` applied, every byte they do not change as it was
/// written - `None` when they change nothing. Only a changed attribute's
/// value bytes are replaced, preserving its quotes and every other lexical
/// byte; a text replaced is escaped as a producer escapes one.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the bytes are not well-formed XML, or what
/// a hook returns.
pub(crate) fn edit_document(bytes: &[u8], edits: &mut dyn Edits) -> Result<Option<Vec<u8>>> {
    let splices = document_splices(bytes, edits)?;
    if splices.is_empty() {
        return Ok(None);
    }
    // The plan already owns every replacement. Reserve its exact output once,
    // including only nonoverlapping splices the writer below will apply.
    let mut length = bytes.len();
    let mut after = 0;
    for (range, replacement) in &splices {
        if range.start < after {
            continue;
        }
        length = length
            .checked_sub(range.len())
            .and_then(|size| size.checked_add(replacement.len()))
            .ok_or_else(|| codec_error(range.start, "edited XML exceeds addressable size"))?;
        after = range.end;
    }
    let mut edited = Vec::with_capacity(length);
    let mut at = 0;
    for (range, replacement) in splices {
        if range.start < at {
            continue;
        }
        edited.extend_from_slice(&bytes[at..range.start]);
        edited.extend_from_slice(&replacement);
        at = range.end;
    }
    edited.extend_from_slice(&bytes[at..]);
    Ok(Some(edited))
}

/// Plan edits eagerly, then read retained source bytes and replacement bytes
/// directly into the package encoder. Retains source plus all changed payloads,
/// not a second complete worksheet. Each consumed payload is released;
/// the splice-index capacity remains until the reader is dropped.
pub(crate) fn edit_reader(
    bytes: std::sync::Arc<[u8]>,
    edits: &mut dyn Edits,
) -> Result<DocumentReader> {
    let splices = document_splices(&bytes, edits)?;
    Ok(DocumentReader {
        source: bytes,
        splices: splices.into_iter(),
        pending: None,
        at: 0,
    })
}

/// Replay of a validated edit plan. Parsing and all fallible edits complete
/// before the reader is returned; read() only copies already-owned bytes.
pub(crate) struct DocumentReader {
    source: std::sync::Arc<[u8]>,
    splices: std::vec::IntoIter<(std::ops::Range<usize>, Vec<u8>)>,
    pending: Option<(std::ops::Range<usize>, std::io::Cursor<Vec<u8>>)>,
    at: usize,
}

impl std::io::Read for DocumentReader {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        let mut written = 0;
        while written < target.len() {
            if self.pending.is_none() {
                for (range, replacement) in self.splices.by_ref() {
                    if range.start >= self.at {
                        self.pending = Some((range, std::io::Cursor::new(replacement)));
                        break;
                    }
                }
            }
            if let Some((range, replacement)) = self.pending.as_mut() {
                if self.at < range.start {
                    let size = (range.start - self.at).min(target.len() - written);
                    target[written..written + size]
                        .copy_from_slice(&self.source[self.at..self.at + size]);
                    self.at += size;
                    written += size;
                } else {
                    let size = std::io::Read::read(replacement, &mut target[written..])?;
                    written += size;
                    if replacement.position() == replacement.get_ref().len() as u64 {
                        self.at = range.end;
                        // Drop the consumed Vec now, not when the ZIP is done.
                        self.pending = None;
                    }
                }
            } else {
                let size = (self.source.len() - self.at).min(target.len() - written);
                if size == 0 {
                    break;
                }
                target[written..written + size]
                    .copy_from_slice(&self.source[self.at..self.at + size]);
                self.at += size;
                written += size;
            }
        }
        Ok(written)
    }
}

fn document_splices(
    bytes: &[u8],
    edits: &mut dyn Edits,
) -> Result<Vec<(std::ops::Range<usize>, Vec<u8>)>> {
    let mut reader = super::styles::reader(bytes);
    let mut namespaces = quick_xml::name::NamespaceResolver::default();
    let mut buffer = Vec::new();
    let mut skipped = Vec::new();
    let mut path: Vec<SmolStr> = Vec::new();
    let mut open: Vec<Held> = Vec::new();
    let mut splices: Vec<(std::ops::Range<usize>, Vec<u8>)> = Vec::new();
    let position = |reader: &quick_xml::Reader<&[u8]>| {
        usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
    };
    loop {
        let start = position(&reader);
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| codec_error(start, error.to_string()))?;
        let end = position(&reader);
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                let empty = matches!(event, Event::Empty(_));
                if let Some(parent) = open.last_mut() {
                    parent.children += 1;
                }
                // A raw subtree can be copied before building the normalized
                // attribute vector. The hook only accepts tags with no local
                // namespace declarations; inherited bindings are resolved here.
                let inherited = namespaces.resolve_element(tag.name()).0;
                if let Some(inserted) = edits.keep_raw_subtree(&path, tag, &inherited, start)? {
                    if !inserted.is_empty() {
                        splices.push((start..start, inserted));
                    }
                    if !empty {
                        let mut depth = 1_usize;
                        while depth > 0 {
                            let at = position(&reader);
                            match reader
                                .read_event_into(&mut skipped)
                                .map_err(|error| codec_error(at, error.to_string()))?
                            {
                                Event::Start(_) => depth += 1,
                                Event::End(_) => depth -= 1,
                                Event::Eof => {
                                    return Err(codec_error(
                                        at,
                                        "expected an element's end tag, got the end of the part",
                                    ));
                                }
                                _ => {}
                            }
                            skipped.clear();
                        }
                    }
                    edits.raw_subtree(&path, tag, &bytes[start..position(&reader)])?;
                    buffer.clear();
                    continue;
                }
                let attributes = attributes_of(tag, start)?;
                // NamespaceResolver receives the already-decoded declarations
                // from this one attribute boundary; it never reparses a tag.
                let level = namespaces.level().checked_add(1).ok_or_else(|| {
                    codec_error(
                        start,
                        "expected XML nesting within the namespace resolver's limit",
                    )
                })?;
                namespaces.set_level(level);
                let mut declarations = 0;
                for (name, value) in &attributes {
                    let Some(prefix) =
                        quick_xml::name::QName(name.as_bytes()).as_namespace_binding()
                    else {
                        continue;
                    };
                    declarations += 1;
                    if declarations > namespaces.max_declarations_per_element() {
                        return Err(codec_error(
                            start,
                            "too many namespace declarations on one element",
                        ));
                    }
                    namespaces
                        .add(prefix, quick_xml::name::Namespace(value.as_bytes()))
                        .map_err(|error| codec_error(start, error.to_string()))?;
                }
                let namespace = namespaces.resolve_element(tag.name()).0;
                path.push(SmolStr::new(String::from_utf8_lossy(local_name(
                    tag.name().as_ref(),
                ))));
                if let Some(inserted) = edits.before(&path, &attributes, &namespace)? {
                    splices.push((start..start, inserted));
                }
                if edits.keep_subtree(&path, &attributes, &namespace)? {
                    if !empty {
                        let mut depth = 1_usize;
                        while depth > 0 {
                            let at = position(&reader);
                            match reader
                                .read_event_into(&mut skipped)
                                .map_err(|error| codec_error(at, error.to_string()))?
                            {
                                Event::Start(_) => depth += 1,
                                Event::End(_) => depth -= 1,
                                Event::Eof => {
                                    return Err(codec_error(
                                        at,
                                        "expected an element's end tag, got the end of the part",
                                    ));
                                }
                                _ => {}
                            }
                            skipped.clear();
                        }
                    }
                    namespaces.pop();
                    path.pop();
                    buffer.clear();
                    continue;
                }
                match edits.start(&path, &attributes, namespace)? {
                    Tag::Drop => {
                        let mut close = end;
                        if !empty {
                            // Past the element's end tag.
                            let mut depth = 1_usize;
                            while depth > 0 {
                                let at = position(&reader);
                                match reader
                                    .read_event_into(&mut skipped)
                                    .map_err(|error| codec_error(at, error.to_string()))?
                                {
                                    Event::Start(_) => depth += 1,
                                    Event::End(_) => depth -= 1,
                                    Event::Eof => {
                                        return Err(codec_error(
                                            at,
                                            "expected an element's end tag, got the end of the part",
                                        ));
                                    }
                                    _ => {}
                                }
                                skipped.clear();
                            }
                            close = position(&reader);
                        }
                        splices.push((start..close, Vec::new()));
                        if let Some(parent) = open.last_mut() {
                            parent.dropped += 1;
                        }
                        namespaces.pop();
                        path.pop();
                    }
                    tag_edit => {
                        let set = match tag_edit {
                            Tag::Set(set) => set,
                            Tag::Keep | Tag::Drop => Vec::new(),
                        };
                        if empty {
                            let mut all = set;
                            match edits.end(&path, 0, 0, &attributes) {
                                Tag::Drop => {
                                    splices.push((start..end, Vec::new()));
                                    if let Some(parent) = open.last_mut() {
                                        parent.dropped += 1;
                                    }
                                    namespaces.pop();
                                    path.pop();
                                    buffer.clear();
                                    continue;
                                }
                                Tag::Set(more) => all.extend(more),
                                Tag::Keep => {}
                            }
                            let inner = edits.inside_empty(&path)?;
                            if let Some(replacement) =
                                edits.replacement(&path, &attributes, tag.name().as_ref())?
                            {
                                splices.push((start..end, replacement));
                            } else if let Some(inner) = inner {
                                let start_tag = retagged(tag, &all, true, start)?
                                    .unwrap_or_else(|| bytes[start..end].to_vec());
                                let closing = start_tag
                                    .iter()
                                    .rposition(|byte| *byte == b'/')
                                    .ok_or_else(|| {
                                        codec_error(
                                            start,
                                            "expected a self-closing element for an insertion",
                                        )
                                    })?;
                                let mut expanded = start_tag[..closing].to_vec();
                                expanded.push(b'>');
                                expanded.extend(inner);
                                expanded.extend_from_slice(b"</");
                                expanded.extend_from_slice(tag.name().as_ref());
                                expanded.push(b'>');
                                splices.push((start..end, expanded));
                            } else if !all.is_empty()
                                && let Some(tag) = retagged(tag, &all, true, start)?
                            {
                                splices.push((start..end, tag));
                            }
                            namespaces.pop();
                            path.pop();
                        } else {
                            open.push(Held {
                                tag: tag.to_owned(),
                                attributes,
                                tag_range: start..end,
                                splice_start: splices.len(),
                                set,
                                children: 0,
                                text: String::new(),
                                dropped: 0,
                            });
                        }
                    }
                }
            }
            Event::End(_) => {
                let Some(held) = open.pop() else {
                    return Err(codec_error(start, "expected no end tag here"));
                };
                match edits.end(&path, held.children, held.dropped, &held.attributes) {
                    Tag::Drop => {
                        // What was edited inside goes with it.
                        let from = held.tag_range.start;
                        splices.truncate(held.splice_start);
                        splices.push((from..end, Vec::new()));
                        if let Some(parent) = open.last_mut() {
                            parent.dropped += 1;
                        }
                    }
                    more => {
                        if let Some(inserted) = edits.before_end(&path)? {
                            splices.push((start..start, inserted));
                        }
                        if let Some(replacement) =
                            edits.replacement(&path, &held.attributes, held.tag.name().as_ref())?
                        {
                            let from = held.tag_range.start;
                            splices.truncate(held.splice_start);
                            splices.push((from..end, replacement));
                            namespaces.pop();
                            path.pop();
                            buffer.clear();
                            continue;
                        }
                        if held.children == 0
                            && let Some(text) = edits.text(&path, &held.text)?
                        {
                            let escaped = quick_xml::escape::partial_escape(&text);
                            splices.push((held.tag_range.end..start, escaped.as_bytes().to_vec()));
                        }
                        let mut all = held.set;
                        if let Tag::Set(more) = more {
                            all.extend(more);
                        }
                        if !all.is_empty()
                            && let Some(tag) =
                                retagged(&held.tag, &all, false, held.tag_range.start)?
                        {
                            splices.push((held.tag_range.clone(), tag));
                        }
                    }
                }
                namespaces.pop();
                path.pop();
            }
            Event::Text(ref text) => {
                if let Some(held) = open.last_mut() {
                    held.text
                        .push_str(&text_piece(text.xml10_content(), start)?);
                }
            }
            Event::CData(ref text) => {
                if let Some(held) = open.last_mut() {
                    held.text
                        .push_str(&text_piece(text.xml10_content(), start)?);
                }
            }
            Event::GeneralRef(_) => {
                if let Some(held) = open.last_mut() {
                    held.text.push_str(&reference_text(&event, start)?);
                }
            }
            Event::Eof => {
                if !open.is_empty() {
                    return Err(codec_error(
                        start,
                        "expected every element closed, got the end of the part",
                    ));
                }
                break;
            }
            _ => {}
        }
        buffer.clear();
    }
    splices.sort_by_key(|(range, _)| range.start);
    Ok(splices)
}

/// The attributes of `tag` by exact QName, their values normalized and unescaped.
fn attributes_of(tag: &BytesStart<'_>, position: usize) -> Result<Vec<(SmolStr, String)>> {
    let mut attributes = Vec::new();
    for held in tag.attributes().with_checks(false) {
        let held = held.map_err(|error| codec_error(position, error.to_string()))?;
        let value = held
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|error| codec_error(position, error.to_string()))?;
        attributes.push((
            SmolStr::new(String::from_utf8_lossy(held.key.as_ref())),
            value.into_owned(),
        ));
    }
    Ok(attributes)
}

/// `tag` with only the selected exact QName attributes changed: existing
/// values keep their quotes, names and spacing; removal takes the preceding
/// separator with the attribute; missing names are appended. The last
/// assignment to a name wins. Equal values keep their original lexemes.
fn retagged(
    tag: &BytesStart<'_>,
    set: &[(SmolStr, Option<String>)],
    empty: bool,
    position: usize,
) -> Result<Option<Vec<u8>>> {
    let last = |name: &[u8]| {
        set.iter()
            .rev()
            .find(|(key, _)| key.as_bytes() == name)
            .map(|(_, value)| value)
    };
    let raw: &[u8] = tag.as_ref();
    // A self-closing tag has raw.len() + 3 bytes. Its first fully escaped
    // replacement already states the exact growth to reserve.
    let mut written: Option<Vec<u8>> = None;
    let mut copied = 0;
    let mut seen: Vec<&[u8]> = Vec::new();
    // quick-xml's attribute iterator lends both slices from this exact
    // tag. Their offsets locate the raw spelling without a second parser;
    // they are byte offsets, so non-ASCII names and values remain intact.
    let offset = |part: &[u8]| part.as_ptr() as usize - raw.as_ptr() as usize;
    let mut splice = |range: std::ops::Range<usize>, replacement: &[u8]| {
        let output = written.get_or_insert_with(|| {
            let growth = replacement.len().saturating_sub(range.len());
            let mut bytes = Vec::with_capacity(raw.len().saturating_add(3).saturating_add(growth));
            bytes.push(b'<');
            bytes
        });
        output.extend_from_slice(&raw[copied..range.start]);
        output.extend_from_slice(replacement);
        copied = range.end;
    };
    for attribute in tag.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| codec_error(position, error.to_string()))?;
        let key = attribute.key.into_inner();
        let Some(value) = last(key) else {
            continue;
        };
        seen.push(key);
        let start = offset(&attribute.value);
        let end = start + attribute.value.len();
        match value {
            Some(value) => {
                let before = attribute
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|error| codec_error(position, error.to_string()))?;
                if before == value.as_str() {
                    continue;
                }
                let mut escaped = escape_attribute(value);
                if raw[start - 1] == b'\'' && escaped.contains('\'') {
                    escaped = escaped.replace('\'', "&apos;");
                }
                splice(start..end, escaped.as_bytes());
            }
            None => {
                let mut from = offset(key);
                while from > tag.name().as_ref().len()
                    && matches!(raw[from - 1], b' ' | b'\t' | b'\r' | b'\n')
                {
                    from -= 1;
                }
                splice(from..end + 1, &[]);
            }
        }
    }
    // Keep whitespace immediately before `>` or `/>` after any additions.
    let insert = raw
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        .map_or(raw.len(), |at| at + 1);
    for (index, (key, value)) in set.iter().enumerate() {
        if seen.iter().any(|held| *held == key.as_bytes())
            || set[index + 1..].iter().any(|(later, _)| later == key)
        {
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        let escaped = escape_attribute(value);
        let output = written.get_or_insert_with(|| {
            let growth = key.len().saturating_add(escaped.len()).saturating_add(4);
            let mut bytes = Vec::with_capacity(raw.len().saturating_add(3).saturating_add(growth));
            bytes.push(b'<');
            bytes
        });
        output.extend_from_slice(&raw[copied..insert]);
        copied = insert;
        output.push(b' ');
        output.extend_from_slice(key.as_bytes());
        output.extend_from_slice(b"=\"");
        output.extend_from_slice(escaped.as_bytes());
        output.push(b'"');
    }
    let Some(mut written) = written else {
        return Ok(None);
    };
    written.extend_from_slice(&raw[copied..]);
    written.extend_from_slice(if empty { b"/>" } else { b">" });
    Ok(Some(written))
}

/// The prefix of the first of `namespaces` bound on the root or its child
/// `element`, the inner declaration winning, for an attribute written there.
///
/// # Errors
///
/// Returns [`Error::Codec`] when the bytes are not well-formed XML.
pub(crate) fn prefix_of(
    bytes: &[u8],
    element: &[u8],
    namespaces: &[&str],
) -> Result<Option<String>> {
    let mut reader = super::styles::reader(bytes);
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut prefix = None;
    loop {
        let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| codec_error(position, error.to_string()))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let qualified = start.name();
                let at_root = depth == 0;
                if at_root || (depth == 1 && local_name(qualified.as_ref()) == element) {
                    for held in start.attributes().with_checks(false) {
                        let held =
                            held.map_err(|error| codec_error(position, error.to_string()))?;
                        let key = held.key.as_ref();
                        let value = held
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|error| codec_error(position, error.to_string()))?
                            .into_owned();
                        let declared = match key.strip_prefix(b"xmlns") {
                            Some([]) => None,
                            Some([b':', declared @ ..]) => Some(declared),
                            _ => continue,
                        };
                        let Some(declared) = declared else {
                            continue;
                        };
                        if namespaces.contains(&value.as_str()) {
                            prefix = Some(String::from_utf8_lossy(declared).into_owned());
                        } else if prefix.as_deref().map(str::as_bytes) == Some(declared) {
                            // The inner element binds the root's prefix to
                            // another namespace: it names none of them here.
                            prefix = None;
                        }
                    }
                    if !at_root {
                        return Ok(prefix);
                    }
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => return Ok(prefix),
            _ => {}
        }
        buffer.clear();
    }
}

/// A start tag with its `count` attribute raised by `delta` and the
/// attributes `set` stated or removed, or as it stands when neither is
/// asked.
fn patch_start<'a>(
    start: &'a BytesStart<'_>,
    delta: Option<u32>,
    set: &[AttributeEdit],
    position: usize,
) -> Result<BytesStart<'a>> {
    // Unedited, the tag is written from the reader's buffer.
    if delta.is_none() && set.is_empty() {
        return Ok(start.borrow());
    }
    let mut patched = BytesStart::new(String::from_utf8_lossy(start.name().as_ref()).into_owned());
    let mut counted = false;
    let mut stated = vec![false; set.len()];
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
        } else if let Some(at) = set.iter().position(|(name, _)| key == name.as_bytes()) {
            stated[at] = true;
            if let (name, Some(value)) = &set[at] {
                patched.push_attribute((*name, value.as_str()));
            }
        } else {
            patched.push_attribute(held);
        }
    }
    if let Some(delta) = delta.filter(|_| !counted) {
        patched.push_attribute(("count", delta.to_string().as_str()));
    }
    for ((name, value), stated) in set.iter().zip(stated) {
        if let (Some(value), false) = (value, stated) {
            patched.push_attribute((*name, value.as_str()));
        }
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
pub(crate) fn relationship_element(
    id: &str,
    family: NamespaceFamily,
    kind: &str,
    target: &str,
) -> String {
    format!(
        "<Relationship Id=\"{id}\" Type=\"{}/{kind}\" Target=\"{target}\"/>",
        family.relationships_namespace()
    )
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/excel/package.rs` pins and a caller cannot reach:
    //! the byte-preserving rewrite every part an edit moves references in
    //! goes through.

    use smol_str::SmolStr;

    use super::{Edits, Tag};

    /// Test-only intake through the package's exact-QName attribute decoder.
    pub fn exact_attribute(start: &str, name: &str) -> crate::Result<Option<String>> {
        let mut reader = quick_xml::Reader::from_str(start);
        let event = reader
            .read_event()
            .map_err(|error| super::codec_error(0, error.to_string()))?;
        match event {
            quick_xml::events::Event::Start(start) | quick_xml::events::Event::Empty(start) => {
                super::exact_attribute(&start, name.as_bytes(), 0)
                    .map(|value| value.map(|text| text.into_owned()))
            }
            _ => Err(super::codec_error(0, "expected a metadata start tag")),
        }
    }

    /// Test-only raw capture records followed by the completely planned output.
    /// A selected subtree stays byte-exact, with an insertion before it; a
    /// later capture refusal must prevent returning a readable document.
    pub fn raw_subtree_capture(bytes: &[u8], fail_at: Option<usize>) -> crate::Result<Vec<String>> {
        struct Capture {
            records: Vec<String>,
            fail_at: Option<usize>,
        }
        impl Edits for Capture {
            fn keep_raw_subtree(
                &mut self,
                _: &[SmolStr],
                tag: &quick_xml::events::BytesStart<'_>,
                namespace: &quick_xml::name::ResolveResult<'_>,
                _: usize,
            ) -> crate::Result<Option<Vec<u8>>> {
                if tag.attributes().any(|attribute| {
                    attribute.is_ok_and(|attribute| attribute.key.as_namespace_binding().is_some())
                }) {
                    return Ok(None);
                }
                Ok((super::local_name(tag.name().as_ref()) == b"keep"
                    && matches!(namespace, quick_xml::name::ResolveResult::Bound(uri)
                        if uri.as_ref() == b"urn:main"))
                .then(|| b"<marker/>".to_vec()))
            }

            fn raw_subtree(
                &mut self,
                path: &[SmolStr],
                tag: &quick_xml::events::BytesStart<'_>,
                raw: &[u8],
            ) -> crate::Result<()> {
                if self.fail_at == Some(self.records.len()) {
                    return Err(crate::Error::InvalidRecord {
                        path: "raw-capture".into(),
                        reason: "injected late capture refusal".into(),
                    });
                }
                let parent = path
                    .iter()
                    .map(SmolStr::as_str)
                    .collect::<Vec<_>>()
                    .join("/");
                self.records.push(format!(
                    "{parent}|{}|{}",
                    String::from_utf8_lossy(tag.name().as_ref()),
                    String::from_utf8_lossy(raw)
                ));
                Ok(())
            }

            fn start(
                &mut self,
                path: &[SmolStr],
                _: &[(SmolStr, String)],
                namespace: quick_xml::name::ResolveResult<'_>,
            ) -> crate::Result<Tag> {
                Ok(
                    if path.last().is_some_and(|name| name == "after")
                        && matches!(namespace, quick_xml::name::ResolveResult::Bound(uri)
                        if uri.as_ref() == b"urn:main")
                    {
                        Tag::Set(vec![("value".into(), Some("new".into()))])
                    } else {
                        Tag::Keep
                    },
                )
            }
        }
        let mut capture = Capture {
            records: Vec::new(),
            fail_at,
        };
        let mut reader = super::edit_reader(std::sync::Arc::from(bytes), &mut capture)?;
        let mut result = String::new();
        std::io::Read::read_to_string(&mut reader, &mut result)?;
        capture.records.push(result);
        Ok(capture.records)
    }

    /// Edits named by element: attributes set, texts replaced, elements
    /// left out.
    struct Named<'a> {
        set: &'a [(&'a str, &'a str, Option<&'a str>)],
        texts: &'a [(&'a str, &'a str)],
        dropped: &'a [&'a str],
        namespace: Option<&'a str>,
        inserted: &'a [(&'a str, &'a str)],
    }

    impl Edits for Named<'_> {
        fn before(
            &mut self,
            path: &[SmolStr],
            _: &[(SmolStr, String)],
            _: &quick_xml::name::ResolveResult<'_>,
        ) -> crate::Result<Option<Vec<u8>>> {
            let name = path.last().map_or("", SmolStr::as_str);
            Ok(self
                .inserted
                .iter()
                .find(|(element, _)| *element == name)
                .map(|(_, bytes)| bytes.as_bytes().to_vec()))
        }

        fn start(
            &mut self,
            path: &[SmolStr],
            _: &[(SmolStr, String)],
            namespace: quick_xml::name::ResolveResult<'_>,
        ) -> crate::Result<Tag> {
            if let Some(expected) = self.namespace
                && !matches!(namespace, quick_xml::name::ResolveResult::Bound(namespace) if namespace.as_ref() == expected.as_bytes())
            {
                return Ok(Tag::Keep);
            }
            let name = path.last().map_or("", SmolStr::as_str);
            if self.dropped.contains(&name) {
                return Ok(Tag::Drop);
            }
            let set: Vec<(SmolStr, Option<String>)> = self
                .set
                .iter()
                .filter(|(element, _, _)| *element == name)
                .map(|(_, attribute, value)| (SmolStr::new(attribute), value.map(str::to_owned)))
                .collect();
            Ok(if set.is_empty() {
                Tag::Keep
            } else {
                Tag::Set(set)
            })
        }

        fn text(&mut self, path: &[SmolStr], _: &str) -> crate::Result<Option<String>> {
            let name = path.last().map_or("", SmolStr::as_str);
            Ok(self
                .texts
                .iter()
                .find(|(element, _)| *element == name)
                .map(|(_, text)| (*text).to_owned()))
        }
    }

    /// A planned document read in bounded chunks; inserted bytes precede each
    /// named element, including elements replaced or dropped by the same plan.
    ///
    /// # Errors
    ///
    /// Returns the same XML refusal as edit_document before returning a reader.
    pub fn document_reader(
        bytes: std::sync::Arc<[u8]>,
        set: &[(&str, &str, Option<&str>)],
        texts: &[(&str, &str)],
        dropped: &[&str],
        inserted: &[(&str, &str)],
    ) -> crate::Result<Box<dyn std::io::Read + Send>> {
        Ok(Box::new(super::edit_reader(
            bytes,
            &mut Named {
                set,
                texts,
                dropped,
                namespace: None,
                inserted,
            },
        )?))
    }

    /// `bytes` rewritten: each `(element, attribute, value)` of `set`, the
    /// element by local name and attribute by exact QName, `None` removing
    /// the attribute; the text of each element `texts` names replaced, every
    /// element `dropped` names left out; `namespace`, when stated, selects
    /// start-tag edits and drops only in that element namespace. `None` is
    /// answered when nothing changes.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Codec`] for bytes that are not well-formed.
    pub fn edit_document(
        bytes: &[u8],
        set: &[(&str, &str, Option<&str>)],
        texts: &[(&str, &str)],
        dropped: &[&str],
        namespace: Option<&str>,
    ) -> crate::Result<Option<Vec<u8>>> {
        super::edit_document(
            bytes,
            &mut Named {
                set,
                texts,
                dropped,
                namespace,
                inserted: &[],
            },
        )
    }
}

/// Attributes at one exact SpreadsheetML path, by exact attribute QName.
/// The root establishes its namespace family; foreign descendants cannot
/// impersonate a member of that path by sharing its local name.
pub(crate) fn element_attributes(
    bytes: &[u8],
    expected: &[&str],
    part: &str,
) -> Result<Vec<(SmolStr, String)>> {
    struct First<'e> {
        expected: &'e [&'e str],
        part: &'e str,
        family: Option<NamespaceFamily>,
        matched: usize,
        found: Option<Vec<(SmolStr, String)>>,
    }
    impl Edits for First<'_> {
        fn start(
            &mut self,
            path: &[SmolStr],
            attributes: &[(SmolStr, String)],
            namespace: quick_xml::name::ResolveResult<'_>,
        ) -> Result<Tag> {
            let family = match &namespace {
                quick_xml::name::ResolveResult::Bound(uri) => std::str::from_utf8(uri.as_ref())
                    .ok()
                    .and_then(NamespaceFamily::from_namespace),
                _ => None,
            };
            if path.len() == 1 {
                if family.is_none() || self.expected.first().is_none_or(|name| path[0] != *name) {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new(self.part),
                        reason: format_smolstr!(
                            "expected a SpreadsheetML {} root, got {} in {namespace:?}",
                            self.expected[0],
                            path[0]
                        ),
                    });
                }
                self.family = family;
            }
            if path.len() == self.matched + 1
                && family.is_some()
                && family == self.family
                && self
                    .expected
                    .get(self.matched)
                    .is_some_and(|name| path[self.matched] == *name)
            {
                self.matched += 1;
                if self.matched == self.expected.len() && self.found.is_none() {
                    self.found = Some(attributes.to_vec());
                }
            }
            Ok(Tag::Keep)
        }

        fn end(&mut self, path: &[SmolStr], _: usize, _: usize, _: &[(SmolStr, String)]) -> Tag {
            if path.len() == self.matched {
                self.matched -= 1;
            }
            Tag::Keep
        }
    }
    let mut first = First {
        expected,
        part,
        family: None,
        matched: 0,
        found: None,
    };
    edit_document(bytes, &mut first)?;
    Ok(first.found.unwrap_or_default())
}
