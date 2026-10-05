//! Everything a worksheet part states outside its cells, carried as it was
//! written: the frame the model's own elements are written back into.
//!
//! One streamed pass reads a worksheet part (the parser in
//! [`parser`](super::parser)); outside `<sheetData>` it captures each child
//! of the root, and each run of text between them, re-serialized from its
//! raw event bytes - `<` and the tag as written and `>`, `/>` for a
//! self-closed tag, text, CDATA, comments and processing instructions as
//! they stand - which is byte for byte what Excel writes; only whitespace
//! inside an end tag is not kept. The XML declaration and the root's start
//! tag are kept verbatim, so every namespace and `mc:Ignorable` survives,
//! a Strict part's included.
//!
//! On write the model's elements and the carried ones are emitted in the
//! order `CT_Worksheet` lists them, a child the schema does not name
//! keeping its place after the known one before it. Each carried child is
//! classed by what a structural edit (inserting rows, removing a sheet)
//! owes it: [`Class::Free`] names no cell, [`Class::Shifted`] names cells
//! the edit rewrites, and [`Class::Blocking`] names cells no edit here can
//! carry through, so such an edit is refused naming it.

use std::fmt::Write as _;
use std::io::Write;
use std::sync::{Arc, LazyLock};

use quick_xml::events::{BytesStart, Event};

use super::package::NamespaceFamily;
use smol_str::SmolStr;

use crate::Result;

use super::cell::CellRange;
use super::layout::{Frozen, Layout};
use super::package::local_name;

/// The children of `CT_Worksheet`, in the order the schema lists them: a
/// carried child's slot is its index here.
pub(crate) const ORDER: [&str; 39] = [
    "sheetPr",
    "dimension",
    "sheetViews",
    "sheetFormatPr",
    "cols",
    "sheetData",
    "sheetCalcPr",
    "sheetProtection",
    "protectedRanges",
    "scenarios",
    "autoFilter",
    "sortState",
    "dataConsolidate",
    "customSheetViews",
    "mergeCells",
    "phoneticPr",
    "conditionalFormatting",
    "dataValidations",
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];

/// The `extLst` entries a structural edit rewrites: conditional formats,
/// data validations and sparklines of the 2010 schema.
pub(crate) const X14_NAMESPACE: &str =
    "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
pub(crate) const XM_NAMESPACE: &str = "http://schemas.microsoft.com/office/excel/2006/main";

/// The only extension families whose selected cell owners can move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShiftedExtension {
    ConditionalFormatting,
    DataValidation,
    Sparkline,
}

impl ShiftedExtension {
    pub(crate) fn from_uri(uri: &str) -> Option<Self> {
        match uri {
            "{78C0D931-6437-407d-A8EE-F0AAD7539E65}" => Some(Self::ConditionalFormatting),
            "{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}" => Some(Self::DataValidation),
            "{05C60535-1F16-4fd2-B633-F4F36F0B64E0}" => Some(Self::Sparkline),
            _ => None,
        }
    }

    pub(crate) const fn container(self) -> (&'static str, &'static str) {
        match self {
            Self::ConditionalFormatting => ("conditionalFormattings", "conditionalFormatting"),
            Self::DataValidation => ("dataValidations", "dataValidation"),
            Self::Sparkline => ("sparklineGroups", "sparklineGroup"),
        }
    }
}

/// What a structural edit owes a carried child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Names no cell: written as read whatever moves.
    Free,
    /// Names cells a structural edit rewrites where they moved.
    Shifted,
    /// Names cells no edit here can carry through: a structural edit is
    /// refused naming it.
    Blocking,
    /// `sheetViews`: carried, its first view's frozen pane written from
    /// the model.
    Modelled,
    /// Where one of the model's own elements stood - `dimension`, `cols`,
    /// `sheetData`, `mergeCells` - which is written from the model there.
    Regenerated,
}

impl Class {
    /// The class of the root's child named `name`.
    fn of(name: &[u8]) -> Self {
        match name {
            b"dimension" | b"cols" | b"sheetData" | b"mergeCells" => Self::Regenerated,
            b"sheetViews" => Self::Modelled,
            b"sheetPr" | b"sheetFormatPr" | b"sheetCalcPr" | b"sheetProtection"
            | b"printOptions" | b"pageMargins" | b"pageSetup" | b"headerFooter"
            | b"legacyDrawingHF" | b"drawingHF" | b"picture" | b"phoneticPr"
            | b"customProperties" | b"webPublishItems" => Self::Free,
            b"hyperlinks"
            | b"conditionalFormatting"
            | b"dataValidations"
            | b"autoFilter"
            | b"sortState"
            | b"protectedRanges"
            | b"ignoredErrors"
            | b"rowBreaks"
            | b"colBreaks"
            | b"cellWatches"
            | b"drawing"
            | b"legacyDrawing"
            | b"tableParts"
            | b"extLst" => Self::Shifted,
            _ => Self::Blocking,
        }
    }

    /// The class as a pin names it.
    #[cfg(feature = "internals")]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Shifted => "shifted",
            Self::Blocking => "blocking",
            Self::Modelled => "modelled",
            Self::Regenerated => "regenerated",
        }
    }
}

/// One child of the root carried outside the cells, or a run of text
/// between two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Carried {
    /// The local name; empty for a run of text.
    pub(crate) name: SmolStr,
    /// Its index in [`ORDER`], or the index of the known child before it.
    pub(crate) slot: u8,
    /// The child as written; empty where the model writes it.
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) class: Class,
}

/// One of the model's own elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Model {
    Dimension,
    SheetViews,
    Cols,
    SheetData,
    MergeCells,
}

impl Model {
    const ALL: [Self; 5] = [
        Self::Dimension,
        Self::SheetViews,
        Self::Cols,
        Self::SheetData,
        Self::MergeCells,
    ];

    /// The element's local name.
    const fn name(self) -> &'static str {
        match self {
            Self::Dimension => "dimension",
            Self::SheetViews => "sheetViews",
            Self::Cols => "cols",
            Self::SheetData => "sheetData",
            Self::MergeCells => "mergeCells",
        }
    }

    /// The element's index in [`ORDER`].
    fn slot(self) -> u8 {
        slot_of(self.name().as_bytes()).unwrap_or(0)
    }

    /// The model element a placeholder named `name` stands for.
    fn of(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|model| model.name() == name)
    }
}

/// The index of `name` in [`ORDER`].
fn slot_of(name: &[u8]) -> Option<u8> {
    ORDER
        .iter()
        .position(|known| known.as_bytes() == name)
        .and_then(|at| u8::try_from(at).ok())
}

/// What a worksheet part states around its cells, in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorksheetFrame {
    /// Everything before the root: the XML declaration and what follows it.
    pub(crate) declaration: Option<Arc<[u8]>>,
    /// The root's start tag, verbatim.
    pub(crate) root: Arc<[u8]>,
    /// The root's qualified name, which its end tag spells.
    pub(crate) root_name: SmolStr,
    pub(crate) items: Vec<Carried>,
    /// Anything after the root's end tag.
    pub(crate) epilogue: Option<Arc<[u8]>>,
    /// The frozen pane the carried `sheetViews` states, which it is written
    /// back verbatim for.
    pub(crate) pane: Option<Frozen>,
}

impl WorksheetFrame {
    /// Change one selected carried child, preserving the other children
    /// and taking schema position/classification from their existing owners.
    pub(crate) fn set_child(
        &mut self,
        name: &'static str,
        previous: Option<&[u8]>,
        bytes: Option<Arc<[u8]>>,
    ) {
        if let Some(index) = previous.and_then(|bytes| {
            self.items
                .iter()
                .position(|item| item.name == name && item.bytes.as_ref() == bytes)
        }) {
            match bytes {
                Some(bytes) => self.items[index].bytes = bytes,
                None => {
                    self.items.remove(index);
                }
            }
        } else if let Some(bytes) = bytes {
            let slot =
                slot_of(name.as_bytes()).expect("the carried child is in CT_Worksheet order");
            let index = self
                .items
                .iter()
                .position(|item| item.slot > slot)
                .unwrap_or(self.items.len());
            self.items.insert(
                index,
                Carried {
                    name: SmolStr::new_static(name),
                    slot,
                    bytes,
                    class: Class::of(name.as_bytes()),
                },
            );
        }
    }
    /// The canonical envelope for a sheet built in memory. Its root is
    /// shared so writing a fresh sheet needs no new envelope allocation.
    pub(crate) fn new(family: NamespaceFamily) -> Self {
        fn root(family: NamespaceFamily) -> Arc<[u8]> {
            format!(
                "<worksheet xmlns=\"{}\" xmlns:r=\"{}\">",
                family.namespace(),
                family.relationships_namespace()
            )
            .into_bytes()
            .into()
        }
        static TRANSITIONAL: LazyLock<Arc<[u8]>> =
            LazyLock::new(|| root(NamespaceFamily::Transitional));
        static STRICT: LazyLock<Arc<[u8]>> = LazyLock::new(|| root(NamespaceFamily::Strict));
        let root = match family {
            NamespaceFamily::Transitional => &*TRANSITIONAL,
            NamespaceFamily::Strict => &*STRICT,
        };
        Self {
            declaration: None,
            root: Arc::clone(root),
            root_name: SmolStr::new_static("worksheet"),
            items: Vec::new(),
            epilogue: None,
            pane: None,
        }
    }

    /// The retained envelope and children an inverse must budget for,
    /// even when their buffers are shared with the current worksheet.
    pub(crate) fn byte_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.declaration.as_ref().map_or(0, |bytes| bytes.len())
            + self.root.len()
            + self.root_name.len()
            + self.epilogue.as_ref().map_or(0, |bytes| bytes.len())
            + self
                .items
                .iter()
                .map(|item| std::mem::size_of::<Carried>() + item.name.len() + item.bytes.len())
                .sum::<usize>()
    }

    /// The prefix the root is written under, with its colon: `x:` for
    /// `<x:worksheet>`, empty for none. Every element the model writes
    /// takes it.
    pub(crate) fn prefix(&self) -> &str {
        self.root_name
            .rfind(':')
            .map_or("", |colon| &self.root_name[..=colon])
    }

    /// The carried children that name cells no structural edit here can
    /// carry through.
    #[cfg(feature = "internals")]
    pub(crate) fn blocking(&self) -> impl Iterator<Item = &str> + '_ {
        self.items
            .iter()
            .filter(|item| item.class == Class::Blocking)
            .map(|item| item.name.as_str())
    }

    /// Leave behind what names a relationship of the part the sheet was
    /// read from, because a sheet put in a new part holds none of them:
    /// the printer settings `pageSetup` names go and its paper, orientation
    /// and scale stay; a hyperlink to a URL goes and one to a place in the
    /// workbook stays, as does an extension naming no part; a child that is
    /// nothing without its relationship - a drawing, comments, a table, a
    /// control - goes whole.
    pub(crate) fn detach(&mut self) {
        let prefixes = self.relationship_prefixes();
        self.items.retain_mut(|item| {
            if item.name.is_empty() {
                return true;
            }
            let Some(elements) = elements(&item.bytes, prefixes.clone()) else {
                // Carried bytes are well-formed by construction; one that is
                // not cannot be shown to name no relationship.
                return false;
            };
            let Some(root) = elements.first() else {
                return true;
            };
            if !root.relationship_below {
                return true;
            }
            if root.relationship {
                // `CT_PageSetup`'s `r:id` alone is optional.
                if item.name != "pageSetup" {
                    return false;
                }
                let tag = root.tag_without(|key| names_relationship(key, &root.prefixes));
                item.bytes = splice(&item.bytes, [(root.start..root.tag_end, tag)]).into();
                return true;
            }
            // A list whose members each stand alone loses the members that
            // name one; any other child is nothing without what it names.
            if !matches!(item.name.as_str(), "hyperlinks" | "extLst") {
                return false;
            }
            let members = elements.iter().filter(|element| element.depth == 1);
            if members.clone().all(|member| member.relationship_below) {
                return false;
            }
            let gone = members
                .filter(|member| member.relationship_below)
                .map(|member| (member.start..member.end, Vec::new()));
            item.bytes = splice(&item.bytes, gone).into();
            true
        });
    }

    /// Give up every `dxfId` a carried child states - an index into the
    /// differential formats of the workbook the sheet was read from, which
    /// the workbook now holding it does not have: a conditional format
    /// keeps its rule and loses its format, as a cell keeps its value and
    /// takes the default style. A 2010 conditional format states its format
    /// inline and keeps it.
    pub(crate) fn forget_styles(&mut self) {
        for item in &mut self.items {
            if item.name.is_empty() || !contains(&item.bytes, b"dxfId") {
                continue;
            }
            let Some(elements) = elements(&item.bytes, Vec::new()) else {
                continue;
            };
            let edits: Vec<_> = elements
                .iter()
                .filter(|element| element.dxf)
                .map(|element| {
                    let tag = element.tag_without(|key| local_name(key) == b"dxfId");
                    (element.start..element.tag_end, tag)
                })
                .collect();
            item.bytes = splice(&item.bytes, edits).into();
        }
    }

    /// Every prefix the root binds to the relationships namespace.
    fn relationship_prefixes(&self) -> Vec<Vec<u8>> {
        // The root's start tag stands open; closed, it is an element.
        let mut root = self.root.to_vec();
        write!(root, "</{}>", self.root_name).ok();
        elements(&root, Vec::new())
            .and_then(|found| found.into_iter().next())
            .map(|root| root.prefixes)
            .unwrap_or_default()
    }

    /// Write the part: the declaration and the root as read, then the
    /// carried children and the model's elements in schema order, `model`
    /// writing each of the model's - handed the carried `sheetViews` for
    /// that one - and the root's end tag.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure or `model`'s.
    pub(crate) fn write<W: Write>(
        &self,
        writer: &mut W,
        mut model: impl FnMut(&mut W, Model, Option<&[u8]>) -> Result<()>,
    ) -> Result<()> {
        if let Some(declaration) = &self.declaration {
            writer.write_all(declaration)?;
        }
        writer.write_all(&self.root)?;
        let present: Vec<Model> = self
            .items
            .iter()
            .filter_map(|item| match item.class {
                Class::Regenerated | Class::Modelled => Model::of(&item.name),
                _ => None,
            })
            .collect();
        let mut written = [false; Model::ALL.len()];
        for item in &self.items {
            for (at, absent) in Model::ALL.into_iter().enumerate() {
                if !written[at] && !present.contains(&absent) && absent.slot() < item.slot {
                    written[at] = true;
                    model(writer, absent, None)?;
                }
            }
            match (item.class, Model::of(&item.name)) {
                (Class::Regenerated, Some(own)) => {
                    written[own as usize] = true;
                    model(writer, own, None)?;
                }
                (Class::Modelled, Some(own)) => {
                    written[own as usize] = true;
                    model(writer, own, Some(&item.bytes))?;
                }
                _ => writer.write_all(&item.bytes)?,
            }
        }
        for (at, remaining) in Model::ALL.into_iter().enumerate() {
            if !written[at] {
                model(writer, remaining, None)?;
            }
        }
        write!(writer, "</{}>", self.root_name)?;
        if let Some(epilogue) = &self.epilogue {
            writer.write_all(epilogue)?;
        }
        Ok(())
    }
}

/// Write an event as its raw bytes spell it.
pub(crate) fn write_raw(event: &Event<'_>, target: &mut Vec<u8>) {
    let (before, after): (&[u8], &[u8]) = match event {
        Event::Start(_) => (b"<", b">"),
        Event::Empty(_) => (b"<", b"/>"),
        Event::End(_) => (b"</", b">"),
        Event::Text(_) => (b"", b""),
        Event::CData(_) => (b"<![CDATA[", b"]]>"),
        Event::Comment(_) => (b"<!--", b"-->"),
        Event::Decl(_) | Event::PI(_) => (b"<?", b"?>"),
        Event::DocType(_) => (b"<!DOCTYPE ", b">"),
        Event::GeneralRef(_) => (b"&", b";"),
        Event::Eof => return,
    };
    target.extend_from_slice(before);
    target.extend_from_slice(event);
    target.extend_from_slice(after);
}

/// One element of a carried child: where its tags lie, and what its
/// attributes state.
struct Element {
    /// The offset of its `<`, past its start tag's `>`, and past its end.
    start: usize,
    tag_end: usize,
    end: usize,
    /// How many elements of the child enclose it.
    depth: usize,
    tag: BytesStart<'static>,
    empty: bool,
    /// The prefixes bound to the relationships namespace where it stands.
    prefixes: Vec<Vec<u8>>,
    /// Whether an attribute of its own is in the relationships namespace.
    relationship: bool,
    /// Whether it, or an element inside it, has one.
    relationship_below: bool,
    /// Whether it states a `dxfId`.
    dxf: bool,
}

impl Element {
    /// Its start tag without the attributes `drop` names, each other one
    /// as written.
    fn tag_without(&self, drop: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let mut tag = vec![b'<'];
        tag.extend_from_slice(self.tag.name().as_ref());
        for attribute in self.tag.attributes().with_checks(false).flatten() {
            if drop(attribute.key.as_ref()) {
                continue;
            }
            let quote = if attribute.value.contains(&b'"') {
                b'\''
            } else {
                b'"'
            };
            tag.push(b' ');
            tag.extend_from_slice(attribute.key.as_ref());
            tag.extend_from_slice(&[b'=', quote]);
            tag.extend_from_slice(&attribute.value);
            tag.push(quote);
        }
        tag.extend_from_slice(if self.empty { b"/>" } else { b">" });
        tag
    }
}

/// The elements of `bytes` in document order, `prefixes` bound to the
/// relationships namespace around them; `None` when the bytes are not
/// well-formed.
fn elements(bytes: &[u8], prefixes: Vec<Vec<u8>>) -> Option<Vec<Element>> {
    let mut reader = super::styles::reader(bytes);
    let mut buffer = Vec::new();
    let mut found: Vec<Element> = Vec::new();
    // The elements open, by their index in `found`.
    let mut open: Vec<usize> = Vec::new();
    loop {
        let start = usize::try_from(reader.buffer_position()).ok()?;
        let event = reader.read_event_into(&mut buffer).ok()?;
        let end = usize::try_from(reader.buffer_position()).ok()?;
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                let empty = matches!(event, Event::Empty(_));
                let mut bound = open
                    .last()
                    .map_or_else(|| prefixes.clone(), |at| found[*at].prefixes.clone());
                let attributes: Vec<_> = tag.attributes().with_checks(false).flatten().collect();
                for attribute in &attributes {
                    let key = attribute.key.as_ref();
                    if let Some(prefix) = key.strip_prefix(b"xmlns:") {
                        let value = attribute
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .ok()?;
                        bound.retain(|held| held.as_slice() != prefix);
                        if value == super::RELATIONSHIPS_NAMESPACE
                            || value == super::STRICT_RELATIONSHIPS_NAMESPACE
                        {
                            bound.push(prefix.to_vec());
                        }
                    }
                }
                let relationship = attributes
                    .iter()
                    .any(|attribute| names_relationship(attribute.key.as_ref(), &bound));
                let dxf = attributes
                    .iter()
                    .any(|attribute| local_name(attribute.key.as_ref()) == b"dxfId");
                if relationship {
                    for at in &open {
                        found[*at].relationship_below = true;
                    }
                }
                found.push(Element {
                    start,
                    tag_end: end,
                    end,
                    depth: open.len(),
                    tag: tag.to_owned(),
                    empty,
                    prefixes: bound,
                    relationship,
                    relationship_below: relationship,
                    dxf,
                });
                if !empty {
                    open.push(found.len() - 1);
                }
            }
            Event::End(_) => {
                let at = open.pop()?;
                found[at].end = end;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    open.is_empty().then_some(found)
}

/// Whether the attribute `key` is in the relationships namespace: its
/// prefix one of `prefixes`.
fn names_relationship(key: &[u8], prefixes: &[Vec<u8>]) -> bool {
    key.iter()
        .position(|byte| *byte == b':')
        .is_some_and(|colon| {
            let prefix = &key[..colon];
            prefix != b"xmlns" && prefixes.iter().any(|held| held.as_slice() == prefix)
        })
}

/// Whether `bytes` holds `needle`.
fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

/// `bytes` with each range of `edits` - in document order - replaced by its
/// bytes; a range inside one already replaced goes with it.
fn splice(
    bytes: &[u8],
    edits: impl IntoIterator<Item = (std::ops::Range<usize>, Vec<u8>)>,
) -> Vec<u8> {
    let mut spliced = Vec::with_capacity(bytes.len());
    let mut at = 0;
    for (range, replacement) in edits {
        if range.start < at {
            continue;
        }
        spliced.extend_from_slice(&bytes[at..range.start]);
        spliced.extend_from_slice(&replacement);
        at = range.end;
    }
    spliced.extend_from_slice(&bytes[at..]);
    spliced
}

/// A child of the root being captured.
struct Open {
    name: SmolStr,
    slot: u8,
    class: Class,
    bytes: Vec<u8>,
    /// Elements open inside it.
    depth: usize,
    /// Whether its bytes are the model's to write, and so not kept.
    discard: bool,
    /// Whether the child belongs to the worksheet's SpreadsheetML namespace.
    main: bool,
    /// Elements nested under a foreign element cannot state model layout.
    foreign_depth: usize,
}

/// What an event outside the cells was to the parser.
pub(crate) enum Capture {
    /// Captured.
    Continue,
    /// `<sheetData>` opened - or stood, self-closed - where the cells are.
    SheetData { empty: bool },
}

/// Which worksheet facts the existing event walker retains.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum FrameCapture {
    #[default]
    Full,
    TableParts,
    HeaderMerges(Option<CellRange>),
}

/// The frame and the layout gathered while a worksheet part is read.
#[derive(Default)]
pub(crate) struct FrameReader {
    capture: FrameCapture,
    declaration: Vec<u8>,
    root: Option<Vec<u8>>,
    root_name: SmolStr,
    items: Vec<Carried>,
    text: Vec<u8>,
    open: Option<Open>,
    /// The slot of the last known child of the root.
    last_slot: u8,
    ended: bool,
    epilogue: Vec<u8>,
    layout: Layout,
    /// How many `sheetView` elements have opened.
    views: usize,
    /// Whether the first view's pane was read.
    pane_read: bool,
}

impl FrameReader {
    /// Observe only the worksheet root and table membership fragments.
    pub(crate) fn table_parts() -> Self {
        Self {
            capture: FrameCapture::TableParts,
            ..Self::default()
        }
    }

    /// Keep only merged ranges intersecting a selected header rectangle.
    pub(crate) fn header_merges(window: Option<CellRange>) -> Self {
        Self {
            capture: FrameCapture::HeaderMerges(window),
            ..Self::default()
        }
    }

    /// The metadata-only pass does not construct a WorksheetFrame.
    pub(crate) fn into_header_merges(self) -> Vec<CellRange> {
        self.layout.merges
    }

    /// Set the selected header window after coordinate-only sheetData scan.
    pub(crate) fn set_header_window(&mut self, window: CellRange) {
        if let FrameCapture::HeaderMerges(held) = &mut self.capture {
            *held = Some(window);
        }
    }

    const fn keeps_merges(&self) -> bool {
        matches!(self.capture, FrameCapture::HeaderMerges(_))
    }

    /// Full model capture also retains row formats and rich inline strings.
    pub(crate) fn keeps_cells(&self) -> bool {
        self.capture == FrameCapture::Full
    }

    /// Whether the worksheet root and every observed child have closed.
    pub(crate) fn is_complete(&self) -> bool {
        self.root.is_some() && self.ended && self.open.is_none()
    }

    /// Fold one event read outside `<sheetData>`.
    ///
    /// # Errors
    ///
    /// Returns a refusal of a layout fact whose value the schema does not
    /// spell.
    pub(crate) fn event(&mut self, event: &Event<'_>, main: bool) -> Result<Capture> {
        if self.root.is_none() {
            match event {
                Event::Start(start) | Event::Empty(start) => {
                    let mut root = Vec::new();
                    write_raw(event, &mut root);
                    self.root_name = SmolStr::new(String::from_utf8_lossy(start.name().as_ref()));
                    self.ended = matches!(event, Event::Empty(_));
                    if self.ended {
                        // A root with no child: the start tag stands open
                        // for the model's elements.
                        root.truncate(root.len() - 2);
                        root.push(b'>');
                    }
                    self.root = Some(root);
                }
                _ if self.keeps_cells() => write_raw(event, &mut self.declaration),
                _ => {}
            }
            return Ok(Capture::Continue);
        }
        if self.ended {
            if self.keeps_cells() {
                write_raw(event, &mut self.epilogue);
            }
            return Ok(Capture::Continue);
        }
        if let Some(open) = self.open.as_mut() {
            if !open.discard {
                write_raw(event, &mut open.bytes);
            }
            match event {
                Event::Start(start) | Event::Empty(start) => {
                    let name = local_name(start.name().as_ref()).to_vec();
                    let depth = open.depth;
                    let parent = open.name.clone();
                    let semantic = open.main && open.foreign_depth == 0 && main;
                    if matches!(event, Event::Start(_)) {
                        open.depth += 1;
                        if !semantic {
                            open.foreign_depth += 1;
                        }
                    }
                    if (self.keeps_cells() || self.keeps_merges()) && semantic {
                        self.inside(&parent, depth, &name, start)?;
                    }
                }
                Event::End(_) => {
                    if open.foreign_depth > 0 {
                        open.foreign_depth -= 1;
                    }
                    open.depth -= 1;
                    if open.depth == 0 {
                        self.close();
                    }
                }
                _ => {}
            }
            return Ok(Capture::Continue);
        }
        match event {
            Event::Start(start) | Event::Empty(start) => {
                self.flush_text();
                let name = local_name(start.name().as_ref()).to_vec();
                let empty = matches!(event, Event::Empty(_));
                if main && let Some(slot) = slot_of(&name) {
                    self.last_slot = slot;
                }
                let class = if main {
                    Class::of(&name)
                } else {
                    Class::Blocking
                };
                let name_text = if main {
                    SmolStr::new(String::from_utf8_lossy(&name))
                } else {
                    SmolStr::new(String::from_utf8_lossy(start.name().as_ref()))
                };
                match name.as_slice() {
                    b"sheetData" if main => {
                        if self.keeps_cells() {
                            self.placeholder(name_text);
                        }
                        return Ok(Capture::SheetData { empty });
                    }
                    b"sheetFormatPr" if main && self.keeps_cells() => {
                        self.layout.read_defaults(start)?
                    }
                    _ => {}
                }
                let mut bytes = Vec::new();
                let discard = class == Class::Regenerated
                    || match self.capture {
                        FrameCapture::Full => false,
                        FrameCapture::TableParts => !main || name != b"tableParts",
                        FrameCapture::HeaderMerges(_) => true,
                    };
                if !discard {
                    write_raw(event, &mut bytes);
                }
                self.open = Some(Open {
                    name: name_text,
                    slot: self.last_slot,
                    class,
                    bytes,
                    depth: 1,
                    discard,
                    main,
                    foreign_depth: 0,
                });
                if empty {
                    self.close();
                }
            }
            Event::End(_) => {
                self.flush_text();
                self.ended = true;
            }
            Event::Eof => {}
            _ if self.keeps_cells() => write_raw(event, &mut self.text),
            _ => {}
        }
        Ok(Capture::Continue)
    }

    /// Read what the element `name`, at `depth` inside the root's child
    /// `parent`, states for the layout.
    fn inside(
        &mut self,
        parent: &str,
        depth: usize,
        name: &[u8],
        start: &BytesStart<'_>,
    ) -> Result<()> {
        if let FrameCapture::HeaderMerges(window) = self.capture {
            if parent == "mergeCells" && depth == 1 && name == b"mergeCell" {
                // Layout owns the one parser for mergeCell@ref. Malformed
                // refs outside the window still refuse the worksheet.
                self.layout.read_merge(start)?;
                if !window.is_some_and(|window| {
                    self.layout
                        .merges
                        .last()
                        .is_some_and(|merge| merge.intersects(window))
                }) {
                    self.layout.merges.pop();
                }
            }
            return Ok(());
        }
        match (parent, depth, name) {
            ("cols", 1, b"col") => self.layout.columns.read(start)?,
            ("mergeCells", 1, b"mergeCell") => self.layout.read_merge(start)?,
            ("sheetViews", 1, b"sheetView") => self.views += 1,
            ("sheetViews", 2, b"pane") if self.views == 1 && !self.pane_read => {
                self.pane_read = true;
                self.layout.pane = frozen(start);
            }
            ("extLst", 1, b"ext") => {
                let uri = super::package::exact_attribute(start, b"uri", 0)?;
                if !uri
                    .as_deref()
                    .is_some_and(|uri| ShiftedExtension::from_uri(uri).is_some())
                    && let Some(open) = self.open.as_mut()
                {
                    open.class = Class::Blocking;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// A placeholder where one of the model's elements stood.
    fn placeholder(&mut self, name: SmolStr) {
        self.items.push(Carried {
            name,
            slot: self.last_slot,
            bytes: Arc::from(Vec::new()),
            class: Class::Regenerated,
        });
    }

    /// Finish the child being captured.
    fn close(&mut self) {
        let Some(open) = self.open.take() else {
            return;
        };
        if !self.keeps_cells() && open.discard {
            return;
        }
        self.items.push(Carried {
            name: open.name,
            slot: open.slot,
            bytes: Arc::from(open.bytes),
            class: open.class,
        });
    }

    /// Finish the run of text between two children.
    fn flush_text(&mut self) {
        if self.text.is_empty() {
            return;
        }
        self.items.push(Carried {
            name: SmolStr::default(),
            slot: self.last_slot,
            bytes: Arc::from(std::mem::take(&mut self.text)),
            class: Class::Free,
        });
    }

    /// The frame and the layout read, `None` for a part with no root.
    pub(crate) fn finish(mut self) -> Option<(WorksheetFrame, Layout)> {
        self.flush_text();
        let root = self.root?;
        Some((
            WorksheetFrame {
                declaration: (!self.declaration.is_empty()).then(|| Arc::from(self.declaration)),
                root: Arc::from(root),
                root_name: self.root_name,
                items: self.items,
                epilogue: (!self.epilogue.is_empty()).then(|| Arc::from(self.epilogue)),
                pane: self.layout.pane,
            },
            self.layout,
        ))
    }
}

/// The frozen pane a `<pane>` states: its splits when its state is frozen,
/// `None` for a split pane that scrolls.
fn frozen(start: &BytesStart<'_>) -> Option<Frozen> {
    let mut state = None;
    let (mut rows, mut columns) = (0_u32, 0_u32);
    for held in start.attributes().with_checks(false).flatten() {
        let value = String::from_utf8_lossy(&held.value).into_owned();
        let split = || {
            value
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|split| split.is_finite() && *split >= 0.0)
                .map_or(0, |split| split as u32)
        };
        match local_name(held.key.as_ref()) {
            b"state" => state = Some(value.clone()),
            b"xSplit" => columns = split(),
            b"ySplit" => rows = split(),
            _ => {}
        }
    }
    matches!(state.as_deref(), Some("frozen" | "frozenSplit"))
        .then_some(Frozen { rows, columns })
        .filter(|frozen| frozen.rows > 0 || frozen.columns > 0)
}

/// The first view's pane and selection for `frozen`, `None` for none.
pub(crate) fn pane_elements(frozen: Option<Frozen>) -> String {
    let Some(frozen) = frozen else {
        return String::new();
    };
    let mut text = String::from("<pane");
    if frozen.columns > 0 {
        let _ = write!(text, " xSplit=\"{}\"", frozen.columns);
    }
    if frozen.rows > 0 {
        let _ = write!(text, " ySplit=\"{}\"", frozen.rows);
    }
    let pane = frozen.active_pane();
    let _ = write!(
        text,
        " topLeftCell=\"{}\" activePane=\"{pane}\" state=\"frozen\"/><selection pane=\"{pane}\"/>",
        frozen.top_left()
    );
    text
}

/// The `sheetViews` a sheet with no carried one states for `frozen`: one
/// view of the first workbook view, nothing when nothing is frozen.
pub(crate) fn sheet_views(frozen: Option<Frozen>, prefix: &str) -> String {
    if frozen.is_none() {
        return String::new();
    }
    let elements = pane_elements(frozen);
    let elements = if prefix.is_empty() {
        elements
    } else {
        elements
            .replace("<pane", &format!("<{prefix}pane"))
            .replace("<selection", &format!("<{prefix}selection"))
    };
    format!(
        "<{prefix}sheetViews><{prefix}sheetView workbookViewId=\"0\">{elements}</{prefix}sheetView></{prefix}sheetViews>"
    )
}

/// The carried `sheetViews` with its first view's pane and selection those
/// `frozen` states, every other attribute and child kept.
///
/// # Errors
///
/// Returns [`Error::Codec`](crate::Error::Codec) when the carried bytes are
/// not well-formed.
pub(crate) fn repaned(views: &[u8], frozen: Option<Frozen>) -> Result<Vec<u8>> {
    use std::cell::Cell;

    let seen = Cell::new(0_usize);
    let fragment = pane_elements(frozen);
    super::package::rewrite(
        views,
        &super::package::Rewrite {
            skip: &|start| {
                let qualified = start.name();
                let name = local_name(qualified.as_ref());
                if name == b"sheetView" {
                    seen.set(seen.get() + 1);
                }
                seen.get() == 1 && matches!(name, b"pane" | b"selection")
            },
            after_start: &|name| {
                (name == b"sheetView" && seen.get() == 1 && !fragment.is_empty())
                    .then(|| fragment.clone())
            },
            ..super::package::Rewrite::default()
        },
    )
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/excel/carried.rs` pins and a caller cannot reach:
    //! the children a parsed sheet carries, where each goes and what a
    //! structural edit owes it.

    use crate::excel::Sheet;

    /// Each carried child of the sheet's part - `(name, slot, class)`, a
    /// run of text named `#text` - in document order; empty for a sheet
    /// built in memory.
    #[must_use]
    pub fn items(sheet: &Sheet) -> Vec<(String, u8, &'static str)> {
        sheet.frame().map_or_else(Vec::new, |frame| {
            frame
                .items
                .iter()
                .map(|item| {
                    let name = if item.name.is_empty() {
                        "#text".to_owned()
                    } else {
                        item.name.to_string()
                    };
                    (name, item.slot, item.class.as_str())
                })
                .collect()
        })
    }

    /// Change one captured child through the existing frame setter.
    pub fn set_child(
        sheet: &mut Sheet,
        name: &'static str,
        previous: Option<&[u8]>,
        bytes: Option<&[u8]>,
    ) {
        let mut frame = sheet
            .frame()
            .cloned()
            .unwrap_or_else(|| super::WorksheetFrame::new(super::NamespaceFamily::Transitional));
        frame.set_child(name, previous, bytes.map(std::sync::Arc::from));
        sheet.set_frame(Some(Box::new(frame)));
    }

    /// The children a structural edit is refused over.
    #[must_use]
    pub fn blocking(sheet: &Sheet) -> Vec<String> {
        sheet.frame().map_or_else(Vec::new, |frame| {
            frame.blocking().map(str::to_owned).collect()
        })
    }
}
