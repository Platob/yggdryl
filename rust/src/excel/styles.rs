//! The styles part as a model: every cell format a workbook holds, what each
//! one resolves to, and the append-only writer that adds to it.
//!
//! A cell's `s` attribute indexes `cellXfs`, whose `xf` names a number
//! format, a font, a fill and a border by their indexes in the part's own
//! lists, beside its alignment and protection. [`StyleSheet`] reads those
//! lists and resolves each entry into a [`CellStyle`] once. It also derives
//! the one fact that changes what a number *is*: `45292` under `yyyy-mm-dd`
//! is a date, under `General` a number, which is [`NumberFormat`].
//!
//! What the model does not read - `cellStyleXfs`, `cellStyles`, `dxfs`,
//! `tableStyles`, `colors/mruColors`, `extLst` - is carried in the part's
//! bytes and never reordered, so every `xfId`, `dxfId` and table style that
//! names an entry of one stays valid. Writing is a splice of those bytes:
//! what a save added is appended after the lists' last entries and their
//! counts are patched, so every index a cell states before the save names
//! the same entry after it.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::io::BufRead;
use std::sync::{Arc, OnceLock};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use smol_str::{SmolStr, format_smolstr};

use crate::{DataTypeId, Error, Result, TimeUnit};

use super::format::{FormatCode, builtin_code, in_builtin_table, undeclared_code};
use super::package::{self, Insertion, attribute, codec_error, escape_attribute, local_name};
use super::style::{
    Alignment, Border, BorderStyle, CellStyle, Color, Edge, Fill, Font, FontScheme, Horizontal,
    PatternType, Protection, StyleId, Underline, Vertical, VerticalRun,
};
use super::theme::{Theme, indexed, tinted};

/// What a cell's number format says its number is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NumberFormat {
    /// `General`, a number format, text, or anything that is not a date: the
    /// number is a number.
    #[default]
    General,
    /// A date without a clock, `yyyy-mm-dd`: the serial is a day.
    Date,
    /// A clock without a date, `h:mm:ss`: the serial is a fraction of a day.
    Time,
    /// A date and a clock, `yyyy-mm-dd hh:mm:ss`.
    DateTime,
    /// A date and a clock to the millisecond, `yyyy-mm-dd hh:mm:ss.000`.
    DateTimeFraction,
    /// An elapsed time past a day, `[h]:mm:ss`: the serial is a duration.
    Duration,
}

impl NumberFormat {
    /// Every format, General first.
    pub const ALL: [Self; 6] = [
        Self::General,
        Self::Date,
        Self::Time,
        Self::DateTime,
        Self::DateTimeFraction,
        Self::Duration,
    ];

    /// The format's name: `general`, `date`, `time`, `datetime`,
    /// `datetime_fraction` or `duration`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTime => "datetime",
            Self::DateTimeFraction => "datetime_fraction",
            Self::Duration => "duration",
        }
    }

    /// Whether the format reads a number as a date, a time or a duration.
    #[must_use]
    pub const fn is_temporal(self) -> bool {
        !matches!(self, Self::General)
    }

    /// The format a temporal of the datatype `id` is written under as a
    /// serial - at `unit`, and `naive` or zoned - `None` for one no serial
    /// spells: a zoned datetime, an interval, anything not temporal. A date
    /// is a `Date`, a naive datetime a `DateTime` to the second and a
    /// `DateTimeFraction` below it, a time a `Time`, a duration a
    /// `Duration`. The one rule a cell's serial and a record column's style
    /// are both read by.
    pub(crate) const fn of_temporal(id: DataTypeId, unit: TimeUnit, naive: bool) -> Option<Self> {
        Some(match id {
            DataTypeId::Date32 | DataTypeId::Date64 => Self::Date,
            DataTypeId::DateTime64 if naive => {
                if matches!(unit, TimeUnit::Second) {
                    Self::DateTime
                } else {
                    Self::DateTimeFraction
                }
            }
            DataTypeId::Time32 | DataTypeId::Time64 => Self::Time,
            DataTypeId::Duration32 | DataTypeId::Duration64 => Self::Duration,
            _ => return None,
        })
    }

    /// The code this crate writes a cell of the format under when the cell
    /// has no style of the format already, `None` for General: the first
    /// three a `numFmt` the styles part declares, `h:mm:ss` and `[h]:mm:ss`
    /// the built-in formats 21 and 46.
    #[must_use]
    pub const fn code(self) -> Option<&'static str> {
        match self {
            Self::General => None,
            Self::Date => Some("yyyy-mm-dd"),
            Self::DateTime => Some("yyyy-mm-dd hh:mm:ss"),
            Self::DateTimeFraction => Some("yyyy-mm-dd hh:mm:ss.000"),
            Self::Time => Some("h:mm:ss"),
            Self::Duration => Some("[h]:mm:ss"),
        }
    }
}

impl std::str::FromStr for NumberFormat {
    type Err = crate::Error;

    /// Read a format by its name, as [`as_str`](Self::as_str) spells it.
    fn from_str(text: &str) -> crate::Result<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.as_str() == text.trim())
            .ok_or_else(|| crate::Error::Parse {
                target: "number format",
                position: 0,
                reason: smol_str::format_smolstr!(
                    "expected general, date, time, datetime, datetime_fraction or duration for a \
                     number format, got {text:?}"
                ),
            })
    }
}

impl std::fmt::Display for NumberFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The most cell formats Excel holds in one workbook.
pub const MAX_CELL_FORMATS: usize = 64_000;

/// One `cellXfs` entry as the part states it: the indexes it points at and
/// its own facts.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Xf {
    number_format: u32,
    font: u32,
    fill: u32,
    border: u32,
    parent: u32,
    alignment: Alignment,
    protection: Protection,
    quote_prefix: bool,
    /// `pivotButton`: the cell shows a pivot table's field button.
    pivot_button: bool,
    /// The `apply` flags the entry states, in [`APPLY`] order; `None` where
    /// it states none, which the writer derives from what the entry holds.
    apply: [Option<bool>; 6],
}

impl Xf {
    /// Whether a cell may be written under the entry for what it resolves
    /// to alone: it shows no pivot button and states no `apply` flag false,
    /// which would display a fact of its named style in place of its own.
    fn is_plain(&self) -> bool {
        !self.pivot_button && self.apply.iter().all(|flag| *flag != Some(false))
    }
}

/// The `apply` flags of a `cellXfs` entry, in the order it holds them.
const APPLY: [&str; 6] = [
    "applyNumberFormat",
    "applyFont",
    "applyFill",
    "applyBorder",
    "applyAlignment",
    "applyProtection",
];

/// How many entries of each list the part held as read; entries past them
/// are what the model appended since.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    number_formats: usize,
    fonts: usize,
    fills: usize,
    borders: usize,
    xfs: usize,
}

/// Where a table's lists stood, for [`StyleSheet::rollback`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Checkpoint {
    counts: Counts,
    appended: usize,
    highest_number_format: u32,
}

/// Which of the lists a writer appends to the part states at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Present {
    number_formats: bool,
    fonts: bool,
    fills: bool,
    borders: bool,
    xfs: bool,
}

/// The styles part of a workbook as a model: every `cellXfs` entry resolved
/// into the [`CellStyle`] a cell naming it is displayed with.
///
/// Held state: the part's bytes as read, which a save splices its additions
/// into, and one resolved style per entry - the bound is the part's own
/// size and its entries, at most [`MAX_CELL_FORMATS`]. A workbook with no
/// styles part holds the smallest one Excel opens without repair: one font,
/// the two reserved fills, one border and the `Normal` style's entry.
///
/// An index past the end of its list - a cell's `s` past `cellXfs`, an
/// entry's `fontId` past `fonts` - names nothing and displays as the
/// default. The model never tracks who states one: a save that appends may
/// give that index to what it appends, so a sheet copied as stored whose
/// cell states it names the appended entry from then on; a sheet written
/// from its cells has already read it as the default.
///
/// ```
/// use yggdryl::excel::{StyleId, Workbook};
///
/// let workbook = Workbook::new();
/// let styles = workbook.style_sheet()?;
/// assert_eq!(styles.len(), 1);
/// let default = styles.style(StyleId::DEFAULT).expect("the default style");
/// assert_eq!(default.number_format, "General");
/// assert_eq!(default.font.name, "Calibri");
/// assert_eq!(default.font.size, 11.0);
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone)]
pub struct StyleSheet {
    bytes: Arc<[u8]>,
    /// The declared codes, by id; appended ones listed in `appended`.
    codes: BTreeMap<u32, SmolStr>,
    appended: Vec<u32>,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    xfs: Vec<Xf>,
    /// The normalized XF prefix read when this workbook opened. Only the
    /// plain entries appended after it may move across in-flight saves.
    original_xfs: usize,
    /// One resolved style per entry of `xfs`.
    styles: Vec<CellStyle>,
    /// Each entry's number format, read once per code.
    formats: Vec<FormatCode>,
    /// What each entry of `xfs` says its number is.
    kinds: Vec<NumberFormat>,
    /// `colors/indexedColors`, when the part replaces the default palette.
    palette: Option<Arc<[u32]>>,
    /// The highest `numFmtId` the part states anywhere - declared, or named
    /// by a cell format, a named style's format or a differential format -
    /// which a new code's id is taken past.
    highest_number_format: u32,
    read: Counts,
    /// How many entries each list held once read: past `read` by the
    /// entries index 0 names that the part leaves out.
    baseline: Counts,
    present: Present,
}

/// The distinct mutable style meanings retained by one inverse. Original
/// XFs keep their IDs and opaque XML; retaining their resolved display
/// style would lose facts that only that XML holds.
#[derive(Clone, Debug, Default)]
pub(crate) struct StyleBindings {
    entries: Vec<(StyleId, CellStyle)>,
}

impl StyleBindings {
    /// Capture from one unchanged table, once per referenced mutable ID.
    /// Invalid IDs retain the existing out-of-range/default semantics.
    pub(crate) fn capture(&mut self, table: &StyleSheet, id: StyleId) {
        if usize::from(id.as_u16()) < table.original_xfs {
            return;
        }
        let Err(at) = self.entries.binary_search_by_key(&id, |(id, _)| *id) else {
            return;
        };
        if let Some(style) = table.style(id) {
            self.entries.insert(at, (id, style.clone()));
        }
    }

    /// The captured meanings, in ID order, with no descriptor copied.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (StyleId, &CellStyle)> {
        self.entries.iter().map(|(id, style)| (*id, style))
    }

    /// Retained capacity and payload, counting shared strings and gradient
    /// bytes conservatively so the journal cannot omit their storage.
    pub(crate) fn byte_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.entries.capacity() * std::mem::size_of::<(StyleId, CellStyle)>()
            + self
                .entries
                .iter()
                .map(|(_, style)| {
                    style.number_format.len()
                        + style.font.name.len()
                        + match &style.fill {
                            Fill::Gradient(bytes) => bytes.len(),
                            _ => 0,
                        }
                })
                .sum::<usize>()
    }
}

impl std::fmt::Debug for StyleSheet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StyleSheet")
            .field("bytes", &self.bytes.len())
            .field("number_formats", &self.codes.len())
            .field("fonts", &self.fonts.len())
            .field("fills", &self.fills.len())
            .field("borders", &self.borders.len())
            .field("cell_formats", &self.xfs.len())
            .finish_non_exhaustive()
    }
}

impl StyleSheet {
    /// The default style lists in the namespace family of a workbook that
    /// did not supply a styles part. Loaded parts keep their authored bytes.
    pub(crate) fn new(family: super::package::NamespaceFamily) -> Self {
        const OPEN: &[u8] =
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"";
        const END: &[u8] = b"\">";
        let namespace = family.namespace().as_bytes();
        let mut bytes =
            Vec::with_capacity(OPEN.len() + namespace.len() + END.len() + EMPTY_STYLE_LISTS.len());
        bytes.extend_from_slice(OPEN);
        bytes.extend_from_slice(namespace);
        bytes.extend_from_slice(END);
        bytes.extend_from_slice(EMPTY_STYLE_LISTS.as_bytes());
        Self::from_xml(bytes).expect("the crate's own empty styles part is well-formed")
    }
}

impl Default for StyleSheet {
    /// The styles part a workbook without one is written with.
    fn default() -> Self {
        Self::new(super::package::NamespaceFamily::Transitional)
    }
}

/// The styles part a package without one is written with: Excel's default
/// font, the two fills every part reserves, an empty border and the
/// `Normal` style.
const EMPTY_STYLE_LISTS: &str = "\
<fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
<fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>\
<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
<cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs>\
<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
</styleSheet>";

/// The top-level lists of a styles part, in the schema's order.
const STYLESHEET_ORDER: [&[u8]; 11] = [
    b"numFmts",
    b"fonts",
    b"fills",
    b"borders",
    b"cellStyleXfs",
    b"cellXfs",
    b"cellStyles",
    b"dxfs",
    b"tableStyles",
    b"colors",
    b"extLst",
];

/// Which top-level list the reader is inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Other,
    NumberFormats,
    Fonts,
    Fills,
    Borders,
    CellFormats,
    Colors,
}

/// The piece of a fill being read.
#[derive(Clone, Copy, Debug, Default)]
struct PatternRead {
    pattern: PatternType,
    foreground: Option<Color>,
    background: Option<Color>,
}

impl StyleSheet {
    /// Read `styles.xml`.
    ///
    /// A `numFmtId` the part declares no code for is a built-in: its en-US
    /// code, or General for one no table lists, as Excel displays it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`](crate::Error::Codec) when the bytes are not
    /// the part, or an attribute is not what the schema spells.
    pub(crate) fn from_xml(bytes: Vec<u8>) -> Result<Self> {
        let bytes: Arc<[u8]> = Arc::from(bytes);
        let mut sheet = Self {
            bytes: Arc::clone(&bytes),
            codes: BTreeMap::new(),
            appended: Vec::new(),
            fonts: Vec::new(),
            fills: Vec::new(),
            borders: Vec::new(),
            xfs: Vec::new(),
            original_xfs: 0,
            styles: Vec::new(),
            formats: Vec::new(),
            kinds: Vec::new(),
            palette: None,
            highest_number_format: 0,
            read: Counts::default(),
            baseline: Counts::default(),
            present: Present::default(),
        };
        let mut reader = Reader::from_reader(&bytes[..]);
        reader.config_mut().trim_text(false);
        let mut buffer = Vec::new();
        let mut depth = 0_usize;
        let mut section = Section::Other;
        let mut in_palette = false;
        let mut palette: Vec<u32> = Vec::new();
        let mut font: Option<Font> = None;
        let mut pattern: Option<PatternRead> = None;
        let mut fill: Option<Fill> = None;
        let mut gradient: Option<usize> = None;
        let mut border: Option<Border> = None;
        let mut edge: Option<(u8, Edge)> = None;
        let mut xf: Option<Xf> = None;
        loop {
            let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| codec_error(position, error.to_string()))?;
            let (start, empty) = match &event {
                Event::Start(start) => (Some(start), false),
                Event::Empty(start) => (Some(start), true),
                _ => (None, false),
            };
            if let Some(start) = start {
                let qualified = start.name();
                let name = local_name(qualified.as_ref());
                if matches!(name, b"numFmt" | b"xf") {
                    if let Some(id) = attribute(start, b"numFmtId", position)?
                        .and_then(|id| id.trim().parse::<u32>().ok())
                    {
                        sheet.highest_number_format = sheet.highest_number_format.max(id);
                    }
                }
                match depth {
                    1 => {
                        section = match name {
                            b"numFmts" => {
                                sheet.present.number_formats = true;
                                Section::NumberFormats
                            }
                            b"fonts" => {
                                sheet.present.fonts = true;
                                Section::Fonts
                            }
                            b"fills" => {
                                sheet.present.fills = true;
                                Section::Fills
                            }
                            b"borders" => {
                                sheet.present.borders = true;
                                Section::Borders
                            }
                            b"cellXfs" => {
                                sheet.present.xfs = true;
                                Section::CellFormats
                            }
                            b"colors" => Section::Colors,
                            _ => Section::Other,
                        };
                    }
                    2 => match (section, name) {
                        (Section::NumberFormats, b"numFmt") => {
                            let id = attribute(start, b"numFmtId", position)?
                                .and_then(|id| id.trim().parse::<u32>().ok());
                            let code = attribute(start, b"formatCode", position)?;
                            if let (Some(id), Some(code)) = (id, code) {
                                sheet.codes.insert(id, SmolStr::new(code));
                            }
                        }
                        (Section::Fonts, b"font") => font = Some(blank_font()),
                        (Section::Fills, b"fill") => fill = Some(Fill::None),
                        (Section::Borders, b"border") => {
                            border = Some(Border {
                                diagonal_up: flag(start, b"diagonalUp", false, position)?,
                                diagonal_down: flag(start, b"diagonalDown", false, position)?,
                                outline: flag(start, b"outline", true, position)?,
                                ..Border::default()
                            });
                        }
                        (Section::CellFormats, b"xf") => xf = Some(read_xf(start, position)?),
                        (Section::Colors, b"indexedColors") => in_palette = !empty,
                        _ => {}
                    },
                    3 => match section {
                        Section::Fonts => {
                            if let Some(font) = font.as_mut() {
                                read_font_child(font, name, start, position)?;
                            }
                        }
                        Section::Fills => match name {
                            b"patternFill" => {
                                let pattern_type = match attribute(start, b"patternType", position)?
                                {
                                    Some(value) => {
                                        schema(PatternType::from_attribute(&value), position)?
                                    }
                                    None => PatternType::None,
                                };
                                pattern = Some(PatternRead {
                                    pattern: pattern_type,
                                    ..PatternRead::default()
                                });
                            }
                            b"gradientFill" => gradient = Some(position),
                            _ => {}
                        },
                        Section::Borders => {
                            if let Some(side) = edge_side(name) {
                                let style = match attribute(start, b"style", position)? {
                                    Some(value) => {
                                        schema(BorderStyle::from_attribute(&value), position)?
                                    }
                                    None => BorderStyle::None,
                                };
                                edge = Some((side, Edge { style, color: None }));
                            }
                        }
                        Section::CellFormats => {
                            if let Some(xf) = xf.as_mut() {
                                match name {
                                    b"alignment" => xf.alignment = read_alignment(start, position)?,
                                    b"protection" => {
                                        xf.protection = Protection {
                                            locked: flag(start, b"locked", true, position)?,
                                            hidden: flag(start, b"hidden", false, position)?,
                                        };
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Section::Colors if in_palette && name == b"rgbColor" => {
                            if let Some(value) = attribute(start, b"rgb", position)? {
                                palette.push(argb(&value, position)?);
                            }
                        }
                        _ => {}
                    },
                    4 => match (section, name) {
                        (Section::Fills, b"fgColor") => {
                            if let Some(pattern) = pattern.as_mut() {
                                pattern.foreground = read_color(start, position)?;
                            }
                        }
                        (Section::Fills, b"bgColor") => {
                            if let Some(pattern) = pattern.as_mut() {
                                pattern.background = read_color(start, position)?;
                            }
                        }
                        (Section::Borders, b"color") => {
                            if let Some((_, edge)) = edge.as_mut() {
                                edge.color = read_color(start, position)?;
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
                if !empty {
                    depth += 1;
                    buffer.clear();
                    continue;
                }
            }
            // An element closes: its end tag, or an empty element read above.
            let closed = match &event {
                Event::End(end) => {
                    depth = depth.saturating_sub(1);
                    Some(local_name(end.name().as_ref()).to_vec())
                }
                Event::Empty(start) => Some(local_name(start.name().as_ref()).to_vec()),
                Event::Eof => break,
                _ => None,
            };
            if let Some(name) = closed {
                let end = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
                match (depth, section, name.as_slice()) {
                    (1, _, _) => {
                        section = Section::Other;
                        in_palette = false;
                    }
                    (2, Section::Fonts, b"font") => {
                        if let Some(font) = font.take() {
                            sheet.fonts.push(font);
                        }
                    }
                    (2, Section::Fills, b"fill") => {
                        if let Some(fill) = fill.take() {
                            sheet.fills.push(fill);
                        }
                    }
                    (2, Section::Borders, b"border") => {
                        if let Some(border) = border.take() {
                            sheet.borders.push(border);
                        }
                    }
                    (2, Section::CellFormats, b"xf") => {
                        if let Some(xf) = xf.take() {
                            sheet.xfs.push(xf);
                        }
                    }
                    (2, Section::Colors, b"indexedColors") => in_palette = false,
                    (3, Section::Fills, b"patternFill") => {
                        if let (Some(read), Some(fill)) = (pattern.take(), fill.as_mut()) {
                            *fill = if read.pattern == PatternType::None
                                && read.foreground.is_none()
                                && read.background.is_none()
                            {
                                Fill::None
                            } else {
                                Fill::Pattern {
                                    pattern: read.pattern,
                                    foreground: read.foreground,
                                    background: read.background,
                                }
                            };
                        }
                    }
                    (3, Section::Fills, b"gradientFill") => {
                        if let (Some(start), Some(fill)) = (gradient.take(), fill.as_mut()) {
                            let raw = bytes.get(start..end).unwrap_or_default();
                            *fill = Fill::Gradient(Arc::from(raw));
                        }
                    }
                    (3, Section::Borders, _) => {
                        if let (Some((side, read)), Some(border)) = (edge.take(), border.as_mut()) {
                            *edge_of(border, side) = read;
                        }
                    }
                    _ => {}
                }
            }
            buffer.clear();
        }
        if !palette.is_empty() {
            sheet.palette = Some(Arc::from(palette));
        }
        sheet.read = Counts {
            number_formats: sheet.codes.len(),
            fonts: sheet.fonts.len(),
            fills: sheet.fills.len(),
            borders: sheet.borders.len(),
            xfs: sheet.xfs.len(),
        };
        // A list the part leaves empty still has the entry index 0 names -
        // the cell format a cell stating no `s` displays with, and the font,
        // fill and border it points at, with the `gray125` fill Excel
        // reserves after the first - each held past what was read, so a save
        // that appends writes them first and nothing it adds takes the index
        // a stated one already resolves through.
        if sheet.fonts.is_empty() {
            sheet.fonts.push(Font::default());
        }
        if sheet.fills.is_empty() {
            sheet.fills.push(Fill::None);
            sheet.fills.push(Fill::Pattern {
                pattern: PatternType::Gray125,
                foreground: None,
                background: None,
            });
        }
        if sheet.borders.is_empty() {
            sheet.borders.push(Border::default());
        }
        if sheet.xfs.is_empty() {
            sheet.xfs.push(Xf::default());
        }
        // The implicit entry is original too; an absent cellXfs list must
        // never allow a save to relocate the meaning of the default ID.
        sheet.original_xfs = sheet.xfs.len();
        sheet.styles = sheet.xfs.iter().map(|xf| sheet.resolved(xf)).collect();
        // One code read per id, however many entries name it.
        let mut read: BTreeMap<u32, FormatCode> = BTreeMap::new();
        sheet.formats = sheet
            .xfs
            .iter()
            .map(|xf| {
                read.entry(xf.number_format)
                    .or_insert_with(|| FormatCode::from_file(&sheet.code_of(xf.number_format)))
                    .clone()
            })
            .collect();
        sheet.kinds = sheet.formats.iter().map(FormatCode::kind).collect();
        sheet.baseline = sheet.counts();
        Ok(sheet)
    }

    /// How many cell formats the part holds: the indexes a cell's `s` may
    /// name, `0` to one less than this. A part listing none holds the
    /// default one a cell stating no `s` displays with.
    #[must_use]
    pub fn len(&self) -> usize {
        self.xfs.len()
    }

    /// Whether the part holds no cell format at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.xfs.is_empty()
    }

    /// The style a cell stating `id` is displayed with, `None` for an index
    /// past the part's cell formats.
    #[must_use]
    pub fn style(&self, id: StyleId) -> Option<&CellStyle> {
        self.styles.get(usize::from(id.as_u16()))
    }

    /// The number format a cell of style `id` displays with, read once when
    /// the styles were: `None` for an index past the part's cell formats.
    #[must_use]
    pub fn format(&self, id: StyleId) -> Option<&FormatCode> {
        self.formats.get(usize::from(id.as_u16()))
    }

    /// The palette an indexed colour names, when the part replaces the
    /// default one (`colors/indexedColors`): ARGB values by index.
    #[must_use]
    pub fn indexed_colors(&self) -> Option<&[u32]> {
        self.palette.as_deref()
    }

    /// The RGB value `color` displays as, `0xRRGGBB`: an `rgb` colour's
    /// own, a `theme` colour's from `theme`, an `indexed` one's from the
    /// part's palette, else the default one - each with its tint applied as
    /// ECMA-376 §18.8.19 states - and `None` for `auto` and for the two
    /// system colours past the palette (64 and 65), which are the reader's
    /// own foreground and background.
    ///
    /// ```
    /// use yggdryl::excel::{Color, Theme, Workbook};
    ///
    /// let workbook = Workbook::new();
    /// let styles = workbook.style_sheet()?;
    /// let theme = Theme::default();
    /// assert_eq!(styles.resolve(&Color::Rgb(0xFF_1F_49_7D), &theme), Some(0x1F_497D));
    /// // Excel rounds Text 1, lighter by half, to RGB 128 in each channel.
    /// let gray = Color::Theme { index: 1, tint: 0.499_984_740_745_262 };
    /// assert_eq!(styles.resolve(&gray, &theme), Some(0x80_8080));
    /// assert_eq!(styles.resolve(&Color::Indexed { index: 10, tint: 0.0 }, &theme), Some(0xFF_0000));
    /// assert_eq!(styles.resolve(&Color::Auto, &theme), None);
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    #[must_use]
    pub fn resolve(&self, color: &Color, theme: &Theme) -> Option<u32> {
        match *color {
            Color::Rgb(argb) => Some(argb & 0x00FF_FFFF),
            Color::Theme { index, tint } => theme.color(index).map(|rgb| tinted(rgb, tint)),
            Color::Indexed { index, tint } => {
                indexed(self.palette.as_deref(), usize::from(index)).map(|rgb| tinted(rgb, tint))
            }
            Color::Auto => None,
        }
    }

    /// What a number in a cell of style `id` is; General for an index past
    /// the part's cell formats.
    pub(crate) fn number_format(&self, id: StyleId) -> NumberFormat {
        self.kinds
            .get(usize::from(id.as_u16()))
            .copied()
            .unwrap_or_default()
    }

    /// Whether a cell of style `id` holding a value of `format` shows with
    /// another number format than `id`'s: a temporal the style does not
    /// read as - a date built in memory, whose style is the default.
    pub(crate) fn overrides(&self, id: StyleId, format: NumberFormat) -> bool {
        format.is_temporal() && self.number_format(id) != format
    }

    /// The number format a cell shows: its typed temporal override, or the
    /// authored style. A General numeric cache under a date style therefore
    /// still has a date reading when a new formula result arrives.
    pub(crate) fn shown_number_format(&self, id: StyleId, format: NumberFormat) -> NumberFormat {
        if self.overrides(id, format) {
            format
        } else {
            self.number_format(id)
        }
    }

    /// The style a cell of style `id` holding a value of `format` shows and
    /// is saved with: `id`'s, its number format the temporal's own code
    /// ([`NumberFormat::code`]) where `id`'s does not read as `format`
    /// ([`Splice::written`]).
    pub(crate) fn shown_style(&self, id: StyleId, format: NumberFormat) -> Cow<'_, CellStyle> {
        let style = self
            .style(id)
            .map_or_else(|| Cow::Owned(CellStyle::default()), Cow::Borrowed);
        match format.code().filter(|_| self.overrides(id, format)) {
            Some(code) => Cow::Owned(CellStyle {
                number_format: SmolStr::new_static(code),
                ..style.into_owned()
            }),
            None => style,
        }
    }

    /// The number format a cell of style `id` holding a value of `format`
    /// shows with, as [`Self::shown_style`] states it: `None` for an index
    /// past the part's cell formats.
    pub(crate) fn shown_format(&self, id: StyleId, format: NumberFormat) -> Option<&FormatCode> {
        if self.overrides(id, format) {
            temporal_format(format)
        } else {
            self.format(id)
        }
    }

    /// Every entry a save may have placed at another ID. Written tables
    /// retain this provenance bound; a save's former length is not a bound
    /// on the shared prefix when two snapshots overlap.
    pub(crate) fn bindings(&self) -> impl Iterator<Item = (StyleId, &CellStyle)> {
        self.styles
            .iter()
            .enumerate()
            .skip(self.original_xfs)
            .filter_map(|(at, style)| Some((StyleId::new(u16::try_from(at).ok()?), style)))
    }

    /// Resolve captured plain styles against this table before publishing
    /// any mutation. Equal IDs stay unchanged; existing equivalent entries
    /// are reused; only an absent style clones and grows the table.
    ///
    /// # Errors
    ///
    /// Returns the table's capacity or number-format refusal, leaving this
    /// table untouched even if an earlier binding appended successfully.
    pub(crate) fn rebind<'a>(
        self: &Arc<Self>,
        bindings: impl IntoIterator<Item = (StyleId, &'a CellStyle)>,
    ) -> Result<(Arc<Self>, HashMap<StyleId, StyleId>)> {
        let mut table = Arc::clone(self);
        let mut moved = HashMap::new();
        for (from, style) in bindings {
            if table.style(from) == Some(style) {
                continue;
            }
            let to = match table.find(style) {
                Some(to) => to,
                None => {
                    table.check_append()?;
                    Arc::make_mut(&mut table).intern(style)?
                }
            };
            if to != from {
                moved.insert(from, to);
            }
        }
        Ok((table, moved))
    }

    /// The first entry displaying exactly as `style` that a cell may be
    /// written under for that alone ([`Xf::is_plain`]), when there is one.
    pub(crate) fn find(&self, style: &CellStyle) -> Option<StyleId> {
        self.styles
            .iter()
            .zip(&self.xfs)
            .position(|(held, xf)| held == style && xf.is_plain())
            .and_then(|at| u16::try_from(at).ok())
            .map(StyleId::new)
    }

    /// Refuse an additional distinct cell format before a caller copies
    /// a shared table. The owned interner uses the same bound.
    pub(crate) fn check_append(&self) -> Result<()> {
        if self.xfs.len() >= MAX_CELL_FORMATS {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.styles"),
                reason: format_smolstr!(
                    "expected at most {MAX_CELL_FORMATS} cell formats in a workbook, which is as \
                     many as Excel opens, got one more"
                ),
            });
        }
        Ok(())
    }

    /// The entry displaying as `style`: one already there, else one
    /// appended with whatever number format, font, fill and border it needs
    /// that the part does not already hold.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the part already holds
    /// [`MAX_CELL_FORMATS`] entries, which is as many as Excel opens.
    pub(crate) fn intern(&mut self, style: &CellStyle) -> Result<StyleId> {
        if let Some(id) = self.find(style) {
            return Ok(id);
        }
        self.check_append()?;
        // The first two fills are Excel's reserved `none` and `gray125`,
        // whatever the part holds there: a fill a part lists alone comes
        // after both, never as the second.
        if !self.fills.contains(&style.fill) {
            for reserved in [
                Fill::None,
                Fill::Pattern {
                    pattern: PatternType::Gray125,
                    foreground: None,
                    background: None,
                },
            ]
            .into_iter()
            .skip(self.fills.len())
            {
                self.fills.push(reserved);
            }
        }
        let xf = Xf {
            number_format: self.number_format_id(&style.number_format)?,
            font: interned(&mut self.fonts, &style.font),
            fill: interned(&mut self.fills, &style.fill),
            border: interned(&mut self.borders, &style.border),
            parent: style.parent,
            alignment: style.alignment,
            protection: style.protection,
            quote_prefix: style.quote_prefix,
            pivot_button: false,
            apply: [None; 6],
        };
        let id = StyleId::new(u16::try_from(self.xfs.len()).unwrap_or(u16::MAX));
        let format = self
            .xfs
            .iter()
            .position(|held| held.number_format == xf.number_format)
            .map_or_else(
                || FormatCode::from_file(&self.code_of(xf.number_format)),
                |at| self.formats[at].clone(),
            );
        self.styles.push(self.resolved(&xf));
        self.kinds.push(format.kind());
        self.formats.push(format);
        self.xfs.push(xf);
        Ok(id)
    }

    /// How many entries each list holds now.
    fn counts(&self) -> Counts {
        Counts {
            number_formats: self.codes.len(),
            fonts: self.fonts.len(),
            fills: self.fills.len(),
            borders: self.borders.len(),
            xfs: self.xfs.len(),
        }
    }

    /// Whether the table holds anything the part does not: an entry
    /// appended, or one index 0 names that the part leaves out.
    pub(crate) fn is_grown(&self) -> bool {
        self.read != self.counts()
    }

    /// Whether a change appended anything since the part was read, which a
    /// save writes whether or not a sheet asks for it.
    pub(crate) fn is_appended(&self) -> bool {
        self.baseline != self.counts()
    }

    /// Where every list stands: what [`Self::rollback`] returns them to.
    pub(crate) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            counts: self.counts(),
            appended: self.appended.len(),
            highest_number_format: self.highest_number_format,
        }
    }

    /// Drop everything appended since `checkpoint`, which a change that
    /// failed part way appended.
    pub(crate) fn rollback(&mut self, checkpoint: &Checkpoint) {
        for id in self.appended.drain(checkpoint.appended..) {
            self.codes.remove(&id);
        }
        self.highest_number_format = checkpoint.highest_number_format;
        self.fonts.truncate(checkpoint.counts.fonts);
        self.fills.truncate(checkpoint.counts.fills);
        self.borders.truncate(checkpoint.counts.borders);
        self.xfs.truncate(checkpoint.counts.xfs);
        self.styles.truncate(checkpoint.counts.xfs);
        self.formats.truncate(checkpoint.counts.xfs);
        self.kinds.truncate(checkpoint.counts.xfs);
    }

    /// The table as the part `bytes` holds it: what [`Self::to_part`]
    /// wrote from it, taken as read, so the next save appends only what is
    /// appended from now on.
    pub(crate) fn written(mut self, bytes: Vec<u8>) -> Self {
        self.present.number_formats |= !self.appended.is_empty();
        self.present.fonts |= self.fonts.len() > self.read.fonts;
        self.present.fills |= self.fills.len() > self.read.fills;
        self.present.borders |= self.borders.len() > self.read.borders;
        self.present.xfs |= self.xfs.len() > self.read.xfs;
        self.bytes = Arc::from(bytes);
        self.appended.clear();
        self.read = self.counts();
        self.baseline = self.read;
        // This records serialization, not a new workbook origin. Keep
        // original_xfs so later snapshots reconcile every appended entry.
        self
    }

    /// The id `code` is written under: a built-in the part does not declare
    /// otherwise, else a code the part declares, else a new one past every
    /// id the part states anywhere - an id a cell format names without a
    /// declaration included, so a cell showing General keeps showing it -
    /// and past 163, the last one Excel reserves.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the part already names the
    /// highest id there is, which leaves no id past it.
    fn number_format_id(&mut self, code: &str) -> Result<u32> {
        if let Some(id) = (0..=49).find(|id| {
            in_builtin_table(*id)
                && !self.codes.contains_key(id)
                && builtin_code(*id).is_some_and(|builtin| {
                    builtin == code || (*id == 0 && builtin.eq_ignore_ascii_case(code))
                })
        }) {
            return Ok(id);
        }
        if let Some((id, _)) = self.codes.iter().find(|(_, held)| held.as_str() == code) {
            return Ok(*id);
        }
        let id = self
            .highest_number_format
            .max(163)
            .checked_add(1)
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.styles"),
                reason: format_smolstr!(
                    "expected a number format id past every one the styles name for the code \
                     {code:?}, got none left past {}",
                    u32::MAX
                ),
            })?;
        self.highest_number_format = id;
        self.codes.insert(id, SmolStr::new(code));
        self.appended.push(id);
        Ok(id)
    }

    /// The code the id `id` displays with in this part.
    fn code_of(&self, id: u32) -> SmolStr {
        self.codes
            .get(&id)
            .cloned()
            .unwrap_or_else(|| SmolStr::new_static(undeclared_code(id)))
    }

    /// The style an entry resolves to.
    fn resolved(&self, xf: &Xf) -> CellStyle {
        CellStyle {
            number_format: self.code_of(xf.number_format),
            font: self
                .fonts
                .get(xf.font as usize)
                .cloned()
                .unwrap_or_default(),
            fill: self
                .fills
                .get(xf.fill as usize)
                .cloned()
                .unwrap_or_default(),
            border: self
                .borders
                .get(xf.border as usize)
                .copied()
                .unwrap_or_default(),
            alignment: xf.alignment,
            protection: xf.protection,
            quote_prefix: xf.quote_prefix,
            parent: xf.parent,
        }
    }

    /// The part as it is written: the bytes read, with everything appended
    /// since spliced in after each list's last entry and the counts of the
    /// lists grown stated again; the bytes as read when nothing was.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`](crate::Error::Codec) when the bytes read are
    /// not well-formed XML.
    pub(crate) fn to_part(&self) -> Result<Vec<u8>> {
        if !self.is_grown() {
            return Ok(self.bytes.to_vec());
        }
        let mut lists: Vec<(&'static [u8], usize, String)> = Vec::new();
        if !self.appended.is_empty() {
            let mut fragment = String::new();
            for id in &self.appended {
                if let Some(code) = self.codes.get(id) {
                    fragment.push_str(&format!(
                        "<numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>",
                        escape_attribute(code)
                    ));
                }
            }
            lists.push((b"numFmts", self.codes.len(), fragment));
        }
        if self.fonts.len() > self.read.fonts {
            let mut fragment = String::new();
            for font in &self.fonts[self.read.fonts..] {
                write_font(&mut fragment, font);
            }
            lists.push((b"fonts", self.fonts.len(), fragment));
        }
        if self.fills.len() > self.read.fills {
            let mut fragment = String::new();
            for fill in &self.fills[self.read.fills..] {
                write_fill(&mut fragment, fill);
            }
            lists.push((b"fills", self.fills.len(), fragment));
        }
        if self.borders.len() > self.read.borders {
            let mut fragment = String::new();
            for border in &self.borders[self.read.borders..] {
                write_border(&mut fragment, border);
            }
            lists.push((b"borders", self.borders.len(), fragment));
        }
        if self.xfs.len() > self.read.xfs {
            let mut fragment = String::new();
            for xf in &self.xfs[self.read.xfs..] {
                write_xf(&mut fragment, xf);
            }
            lists.push((b"cellXfs", self.xfs.len(), fragment));
        }
        let present = |name: &[u8]| match name {
            b"numFmts" => self.present.number_formats,
            b"fonts" => self.present.fonts,
            b"fills" => self.present.fills,
            b"borders" => self.present.borders,
            _ => self.present.xfs,
        };
        // A list the part does not state goes in whole at its place in the
        // schema, before the first list that follows it and is there.
        let insert: Vec<Insertion<'_>> = lists
            .iter()
            .filter(|(name, _, _)| !present(name))
            .map(|(name, count, fragment)| {
                let at = STYLESHEET_ORDER
                    .iter()
                    .position(|held| held == name)
                    .map_or(STYLESHEET_ORDER.len(), |at| at + 1);
                Insertion {
                    fragment: format!(
                        "<{list} count=\"{count}\">{fragment}</{list}>",
                        list = String::from_utf8_lossy(name)
                    ),
                    parent: b"styleSheet",
                    before_first_of: &STYLESHEET_ORDER[at..],
                }
            })
            .collect();
        let grown = |name: &[u8]| {
            lists
                .iter()
                .find(|(list, _, _)| *list == name && present(list))
        };
        package::rewrite(
            &self.bytes,
            &package::Rewrite {
                attributes: &|name| {
                    grown(name).map_or_else(Vec::new, |(_, count, _)| {
                        vec![("count", Some(count.to_string()))]
                    })
                },
                before_end: &|name| grown(name).map(|(_, _, fragment)| fragment.clone()),
                insert: &insert,
                ..package::Rewrite::default()
            },
        )
    }
}

/// The index of `value` in `list`, appending it where the list lacks it.
fn interned<T: Clone + PartialEq>(list: &mut Vec<T>, value: &T) -> u32 {
    let at = list
        .iter()
        .position(|held| held == value)
        .unwrap_or_else(|| {
            list.push(value.clone());
            list.len() - 1
        });
    u32::try_from(at).unwrap_or(u32::MAX)
}

/// A font as a `<font>` stating nothing reads: no name, eleven points.
fn blank_font() -> Font {
    Font {
        name: SmolStr::default(),
        ..Font::default()
    }
}

/// Read one child of `<font>` into `font`.
fn read_font_child(
    font: &mut Font,
    name: &[u8],
    start: &BytesStart<'_>,
    position: usize,
) -> Result<()> {
    let value = || attribute(start, b"val", position);
    match name {
        b"b" => font.bold = flag(start, b"val", true, position)?,
        b"i" => font.italic = flag(start, b"val", true, position)?,
        b"strike" => font.strike = flag(start, b"val", true, position)?,
        b"condense" => font.condense = flag(start, b"val", true, position)?,
        b"extend" => font.extend = flag(start, b"val", true, position)?,
        b"outline" => font.outline = flag(start, b"val", true, position)?,
        b"shadow" => font.shadow = flag(start, b"val", true, position)?,
        b"u" => {
            font.underline = match value()? {
                Some(value) => schema(Underline::from_attribute(&value), position)?,
                None => Underline::Single,
            };
        }
        b"vertAlign" => {
            if let Some(value) = value()? {
                font.vertical = schema(VerticalRun::from_attribute(&value), position)?;
            }
        }
        b"sz" => {
            if let Some(value) = value()? {
                font.size = number(&value, "a font size", position)?;
            }
        }
        b"color" => font.color = read_color(start, position)?,
        b"name" | b"rFont" => {
            if let Some(value) = value()? {
                font.name = SmolStr::new(value);
            }
        }
        b"family" => {
            if let Some(value) = value()? {
                font.family = Some(number(&value, "a font family", position)?);
            }
        }
        b"charset" => {
            if let Some(value) = value()? {
                font.charset = Some(number(&value, "a font charset", position)?);
            }
        }
        b"scheme" => {
            if let Some(value) = value()? {
                font.scheme = Some(schema(FontScheme::from_attribute(&value), position)?);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Read a `cellXfs` entry's attributes.
fn read_xf(start: &BytesStart<'_>, position: usize) -> Result<Xf> {
    // An id that is absent or not a number names the first entry, as it
    // always has here.
    let index = |name: &[u8]| -> Result<u32> {
        Ok(attribute(start, name, position)?
            .and_then(|value| value.trim().parse::<u32>().ok())
            .unwrap_or(0))
    };
    Ok(Xf {
        number_format: index(b"numFmtId")?,
        font: index(b"fontId")?,
        fill: index(b"fillId")?,
        border: index(b"borderId")?,
        parent: index(b"xfId")?,
        alignment: Alignment::default(),
        protection: Protection::default(),
        quote_prefix: flag(start, b"quotePrefix", false, position)?,
        pivot_button: flag(start, b"pivotButton", false, position)?,
        apply: [
            stated_flag(start, b"applyNumberFormat", position)?,
            stated_flag(start, b"applyFont", position)?,
            stated_flag(start, b"applyFill", position)?,
            stated_flag(start, b"applyBorder", position)?,
            stated_flag(start, b"applyAlignment", position)?,
            stated_flag(start, b"applyProtection", position)?,
        ],
    })
}

/// Read an `<alignment>`.
fn read_alignment(start: &BytesStart<'_>, position: usize) -> Result<Alignment> {
    let mut alignment = Alignment::default();
    if let Some(value) = attribute(start, b"horizontal", position)? {
        alignment.horizontal = schema(Horizontal::from_attribute(&value), position)?;
    }
    if let Some(value) = attribute(start, b"vertical", position)? {
        alignment.vertical = schema(Vertical::from_attribute(&value), position)?;
    }
    if let Some(value) = attribute(start, b"textRotation", position)? {
        alignment.rotation = number(&value, "a text rotation", position)?;
    }
    if let Some(value) = attribute(start, b"indent", position)? {
        alignment.indent = number(&value, "an indent", position)?;
    }
    if let Some(value) = attribute(start, b"relativeIndent", position)? {
        alignment.relative_indent = number(&value, "a relative indent", position)?;
    }
    if let Some(value) = attribute(start, b"readingOrder", position)? {
        alignment.reading_order = number(&value, "a reading order", position)?;
    }
    alignment.wrap = flag(start, b"wrapText", false, position)?;
    alignment.justify_last_line = flag(start, b"justifyLastLine", false, position)?;
    alignment.shrink_to_fit = flag(start, b"shrinkToFit", false, position)?;
    Ok(alignment)
}

/// Read a colour element's attributes: `None` when it states no colour.
fn read_color(start: &BytesStart<'_>, position: usize) -> Result<Option<Color>> {
    let tint = match attribute(start, b"tint", position)? {
        Some(value) => number(&value, "a tint", position)?,
        None => 0.0,
    };
    if let Some(value) = attribute(start, b"rgb", position)? {
        return Ok(Some(Color::Rgb(argb(&value, position)?)));
    }
    if let Some(value) = attribute(start, b"theme", position)? {
        return Ok(Some(Color::Theme {
            index: number(&value, "a theme colour", position)?,
            tint,
        }));
    }
    if let Some(value) = attribute(start, b"indexed", position)? {
        return Ok(Some(Color::Indexed {
            index: number(&value, "an indexed colour", position)?,
            tint,
        }));
    }
    if flag(start, b"auto", false, position)? {
        return Ok(Some(Color::Auto));
    }
    Ok(None)
}

/// An `ST_UnsignedIntHex` colour: eight hex digits of ARGB, or six of RGB
/// taken as opaque.
fn argb(value: &str, position: usize) -> Result<u32> {
    let digits = value.trim();
    let parsed = u32::from_str_radix(digits, 16).ok().filter(|_| {
        matches!(digits.len(), 6 | 8) && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    match parsed {
        Some(color) if digits.len() == 6 => Ok(0xFF00_0000 | color),
        Some(color) => Ok(color),
        None => Err(codec_error(
            position,
            format_smolstr!("expected eight hex digits of ARGB for a colour, got {value:?}"),
        )),
    }
}

/// An `xsd:boolean` attribute, `default` when absent.
fn flag(start: &BytesStart<'_>, name: &[u8], default: bool, position: usize) -> Result<bool> {
    Ok(stated_flag(start, name, position)?.unwrap_or(default))
}

/// An `xsd:boolean` attribute, `None` when absent.
fn stated_flag(start: &BytesStart<'_>, name: &[u8], position: usize) -> Result<Option<bool>> {
    match attribute(start, name, position)? {
        None => Ok(None),
        Some(value) => match value.trim() {
            "1" | "true" => Ok(Some(true)),
            "0" | "false" => Ok(Some(false)),
            other => Err(codec_error(
                position,
                format_smolstr!(
                    "expected 1, true, 0 or false for `{}`, got {other:?}",
                    String::from_utf8_lossy(name)
                ),
            )),
        },
    }
}

/// A number attribute of the type the field holds.
fn number<T: std::str::FromStr>(value: &str, what: &str, position: usize) -> Result<T> {
    value.trim().parse::<T>().map_err(|_| {
        codec_error(
            position,
            format_smolstr!("expected a number for {what}, got {value:?}"),
        )
    })
}

/// A schema enumeration's refusal, placed at the element.
fn schema<T>(read: Result<T>, position: usize) -> Result<T> {
    read.map_err(|error| match error {
        Error::Parse { reason, .. } => codec_error(position, reason),
        other => other,
    })
}

/// Which edge of a border an element names, as the edge's slot.
fn edge_side(name: &[u8]) -> Option<u8> {
    Some(match name {
        b"left" | b"start" => 0,
        b"right" | b"end" => 1,
        b"top" => 2,
        b"bottom" => 3,
        b"diagonal" => 4,
        b"vertical" => 5,
        b"horizontal" => 6,
        _ => return None,
    })
}

/// The edge of `border` at slot `side`.
fn edge_of(border: &mut Border, side: u8) -> &mut Edge {
    match side {
        0 => &mut border.left,
        1 => &mut border.right,
        2 => &mut border.top,
        3 => &mut border.bottom,
        4 => &mut border.diagonal,
        5 => &mut border.vertical,
        _ => &mut border.horizontal,
    }
}

/// Write a colour element named `name`.
fn write_color(out: &mut String, name: &str, color: &Color) {
    let tint = |out: &mut String, tint: f64| {
        if tint != 0.0 {
            out.push_str(&format!(" tint=\"{tint}\""));
        }
    };
    out.push('<');
    out.push_str(name);
    match color {
        Color::Rgb(value) => out.push_str(&format!(" rgb=\"{value:08X}\"")),
        Color::Theme { index, tint: shade } => {
            out.push_str(&format!(" theme=\"{index}\""));
            tint(out, *shade);
        }
        Color::Indexed { index, tint: shade } => {
            out.push_str(&format!(" indexed=\"{index}\""));
            tint(out, *shade);
        }
        Color::Auto => out.push_str(" auto=\"1\""),
    }
    out.push_str("/>");
}

/// Write one `<font>` in the order Excel writes its children.
fn write_font(out: &mut String, font: &Font) {
    out.push_str("<font>");
    for (on, name) in [
        (font.bold, "b"),
        (font.italic, "i"),
        (font.strike, "strike"),
        (font.condense, "condense"),
        (font.extend, "extend"),
        (font.outline, "outline"),
        (font.shadow, "shadow"),
    ] {
        if on {
            out.push_str(&format!("<{name}/>"));
        }
    }
    match font.underline {
        Underline::None => {}
        Underline::Single => out.push_str("<u/>"),
        other => out.push_str(&format!("<u val=\"{other}\"/>")),
    }
    if font.vertical != VerticalRun::Baseline {
        out.push_str(&format!("<vertAlign val=\"{}\"/>", font.vertical));
    }
    out.push_str(&format!("<sz val=\"{}\"/>", font.size));
    if let Some(color) = &font.color {
        write_color(out, "color", color);
    }
    if !font.name.is_empty() {
        out.push_str(&format!("<name val=\"{}\"/>", escape_attribute(&font.name)));
    }
    if let Some(family) = font.family {
        out.push_str(&format!("<family val=\"{family}\"/>"));
    }
    if let Some(charset) = font.charset {
        out.push_str(&format!("<charset val=\"{charset}\"/>"));
    }
    if let Some(scheme) = font.scheme {
        out.push_str(&format!("<scheme val=\"{scheme}\"/>"));
    }
    out.push_str("</font>");
}

/// Write one `<fill>`.
fn write_fill(out: &mut String, fill: &Fill) {
    out.push_str("<fill>");
    match fill {
        Fill::None => out.push_str("<patternFill patternType=\"none\"/>"),
        Fill::Pattern {
            pattern,
            foreground,
            background,
        } => {
            out.push_str(&format!("<patternFill patternType=\"{pattern}\""));
            if foreground.is_none() && background.is_none() {
                out.push_str("/>");
            } else {
                out.push('>');
                if let Some(color) = foreground {
                    write_color(out, "fgColor", color);
                }
                if let Some(color) = background {
                    write_color(out, "bgColor", color);
                }
                out.push_str("</patternFill>");
            }
        }
        Fill::Gradient(raw) => out.push_str(&String::from_utf8_lossy(raw)),
    }
    out.push_str("</fill>");
}

/// Write one `<border>`: the four sides and the diagonal always, the inner
/// edges only where they draw.
fn write_border(out: &mut String, border: &Border) {
    out.push_str("<border");
    if border.diagonal_up {
        out.push_str(" diagonalUp=\"1\"");
    }
    if border.diagonal_down {
        out.push_str(" diagonalDown=\"1\"");
    }
    if !border.outline {
        out.push_str(" outline=\"0\"");
    }
    out.push('>');
    let edges = [
        ("left", &border.left, true),
        ("right", &border.right, true),
        ("top", &border.top, true),
        ("bottom", &border.bottom, true),
        ("diagonal", &border.diagonal, true),
        ("vertical", &border.vertical, false),
        ("horizontal", &border.horizontal, false),
    ];
    for (name, edge, always) in edges {
        if !always && *edge == Edge::default() {
            continue;
        }
        out.push('<');
        out.push_str(name);
        if edge.style != BorderStyle::None {
            out.push_str(&format!(" style=\"{}\"", edge.style));
        }
        match &edge.color {
            Some(color) => {
                out.push('>');
                write_color(out, "color", color);
                out.push_str(&format!("</{name}>"));
            }
            None => out.push_str("/>"),
        }
    }
    out.push_str("</border>");
}

/// Write one `cellXfs` entry, stating the `apply` flag of every part it
/// takes from somewhere other than the defaults.
fn write_xf(out: &mut String, xf: &Xf) {
    out.push_str(&format!(
        "<xf numFmtId=\"{}\" fontId=\"{}\" fillId=\"{}\" borderId=\"{}\" xfId=\"{}\"",
        xf.number_format, xf.font, xf.fill, xf.border, xf.parent
    ));
    if xf.quote_prefix {
        out.push_str(" quotePrefix=\"1\"");
    }
    if xf.pivot_button {
        out.push_str(" pivotButton=\"1\"");
    }
    let alignment = xf.alignment != Alignment::default();
    let protection = xf.protection != Protection::default();
    let derived = [
        xf.number_format != 0,
        xf.font != 0,
        xf.fill != 0,
        xf.border != 0,
        alignment,
        protection,
    ];
    for ((name, stated), derived) in APPLY.iter().zip(xf.apply).zip(derived) {
        match stated {
            Some(applies) => out.push_str(&format!(" {name}=\"{}\"", u8::from(applies))),
            None if derived => out.push_str(&format!(" {name}=\"1\"")),
            None => {}
        }
    }
    if !alignment && !protection {
        out.push_str("/>");
        return;
    }
    out.push('>');
    if alignment {
        let held = &xf.alignment;
        out.push_str("<alignment");
        if held.horizontal != Horizontal::General {
            out.push_str(&format!(" horizontal=\"{}\"", held.horizontal));
        }
        if held.vertical != Vertical::Bottom {
            out.push_str(&format!(" vertical=\"{}\"", held.vertical));
        }
        if held.rotation != 0 {
            out.push_str(&format!(" textRotation=\"{}\"", held.rotation));
        }
        if held.wrap {
            out.push_str(" wrapText=\"1\"");
        }
        if held.indent != 0 {
            out.push_str(&format!(" indent=\"{}\"", held.indent));
        }
        if held.relative_indent != 0 {
            out.push_str(&format!(" relativeIndent=\"{}\"", held.relative_indent));
        }
        if held.justify_last_line {
            out.push_str(" justifyLastLine=\"1\"");
        }
        if held.shrink_to_fit {
            out.push_str(" shrinkToFit=\"1\"");
        }
        if held.reading_order != 0 {
            out.push_str(&format!(" readingOrder=\"{}\"", held.reading_order));
        }
        out.push_str("/>");
    }
    if protection {
        out.push_str("<protection");
        if !xf.protection.locked {
            out.push_str(" locked=\"0\"");
        }
        if xf.protection.hidden {
            out.push_str(" hidden=\"1\"");
        }
        out.push_str("/>");
    }
    out.push_str("</xf>");
}

/// The styles a save writes cells of the crate's temporal formats under,
/// one per format, interned into the workbook's styles as a cell first
/// needs one: `yyyy-mm-dd`, `yyyy-mm-dd hh:mm:ss`,
/// `yyyy-mm-dd hh:mm:ss.000`, `h:mm:ss` and `[h]:mm:ss` - each Excel's
/// default style with that number format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TemporalStyles([Option<StyleId>; 5]);

impl TemporalStyles {
    /// The slot of a temporal format.
    const fn slot(format: NumberFormat) -> Option<usize> {
        Some(match format {
            NumberFormat::General => return None,
            NumberFormat::Date => 0,
            NumberFormat::DateTime => 1,
            NumberFormat::DateTimeFraction => 2,
            NumberFormat::Time => 3,
            NumberFormat::Duration => 4,
        })
    }

    /// The style interned for `format`, `None` while none is.
    pub(crate) fn get(&self, format: NumberFormat) -> Option<StyleId> {
        Self::slot(format).and_then(|slot| self.0[slot])
    }
}

/// The code a temporal `format` is shown and written under where a cell's
/// style does not read as it ([`NumberFormat::code`]), read once; `None`
/// for General.
fn temporal_format(format: NumberFormat) -> Option<&'static FormatCode> {
    static CODES: OnceLock<[FormatCode; 5]> = OnceLock::new();
    let codes = CODES.get_or_init(|| {
        [
            NumberFormat::Date,
            NumberFormat::DateTime,
            NumberFormat::DateTimeFraction,
            NumberFormat::Time,
            NumberFormat::Duration,
        ]
        .map(|format| FormatCode::from_file(format.code().unwrap_or_default()))
    });
    TemporalStyles::slot(format).map(|slot| &codes[slot])
}

/// The styles one save writes under: the workbook's table as it holds it,
/// the copy the save appends to once a cell needs a style the table lacks,
/// and the temporal styles found or interned so far.
///
/// The table a workbook holds is never changed by a save: a save that
/// appends writes the copy, and the workbook reads the part it wrote once
/// it adopts the package, at the same indexes, because the table only
/// grows.
pub(crate) struct Splice<'a> {
    held: &'a StyleSheet,
    grown: Option<StyleSheet>,
    temporal: TemporalStyles,
    /// The style each other style a temporal cell holds is written under,
    /// by that style and the cell's format.
    restated: Vec<((StyleId, NumberFormat), StyleId)>,
}

impl<'a> Splice<'a> {
    /// A save over the table `held`.
    pub(crate) const fn new(held: &'a StyleSheet) -> Self {
        Self {
            held,
            grown: None,
            temporal: TemporalStyles([None; 5]),
            restated: Vec::new(),
        }
    }

    /// The table as the workbook holds it, which every index a cell states
    /// before the save names an entry of.
    pub(crate) const fn held(&self) -> &StyleSheet {
        self.held
    }

    /// The style a cell of `format` is written under when its own does not
    /// read as that format: the default style with the format's code, found
    /// in the table or appended to it.
    ///
    /// # Errors
    ///
    /// Returns the table's refusal when it is full, or a refusal for
    /// General, which no temporal style is.
    pub(crate) fn temporal(&mut self, format: NumberFormat) -> Result<StyleId> {
        if let Some(id) = self.temporal.get(format) {
            return Ok(id);
        }
        let (Some(slot), Some(code)) = (TemporalStyles::slot(format), format.code()) else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.styles"),
                reason: format_smolstr!("expected a temporal format, got {format}"),
            });
        };
        let current = self.grown.as_ref().unwrap_or(self.held);
        let style = CellStyle {
            number_format: SmolStr::new_static(code),
            ..current.style(StyleId::DEFAULT).cloned().unwrap_or_default()
        };
        let id = match current.find(&style) {
            Some(id) => id,
            None => self
                .grown
                .get_or_insert_with(|| self.held.clone())
                .intern(&style)?,
        };
        self.temporal.0[slot] = Some(id);
        Ok(id)
    }

    /// The style a cell of style `own` holding a value of `format` is
    /// written under: `own`, or where it does not read as `format`, `own`'s
    /// with the format's code ([`StyleSheet::shown_style`]) - the default
    /// style's being [`Self::temporal`] - found in the table or appended to
    /// it, so a bold date stays bold.
    ///
    /// # Errors
    ///
    /// Returns the table's refusal when it is full.
    pub(crate) fn written(&mut self, own: StyleId, format: NumberFormat) -> Result<StyleId> {
        if !self.held.overrides(own, format) {
            return Ok(own);
        }
        if own == StyleId::DEFAULT {
            return self.temporal(format);
        }
        if let Some((_, id)) = self.restated.iter().find(|(key, _)| *key == (own, format)) {
            return Ok(*id);
        }
        let style = self.held.shown_style(own, format).into_owned();
        let current = self.grown.as_ref().unwrap_or(self.held);
        let id = match current.find(&style) {
            Some(id) => id,
            None => self
                .grown
                .get_or_insert_with(|| self.held.clone())
                .intern(&style)?,
        };
        self.restated.push(((own, format), id));
        Ok(id)
    }

    /// The temporal styles interned so far.
    pub(crate) const fn temporal_styles(&self) -> &TemporalStyles {
        &self.temporal
    }

    /// The table the save appended to, `None` when it appended nothing.
    pub(crate) fn into_grown(self) -> Option<StyleSheet> {
        self.grown.filter(StyleSheet::is_grown)
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/excel/styles.rs` pins and a caller cannot reach: the
    //! model read from a part's bytes, a style interned into it, and the part
    //! a save writes from it.

    use std::collections::HashMap;
    use std::sync::Arc;

    use super::{CellStyle, StyleBindings, StyleId, StyleSheet};
    use crate::Result;

    /// The immutable prefix, including an implicit default entry.
    pub fn original_xfs(sheet: &StyleSheet) -> usize {
        sheet.original_xfs
    }

    /// Mark a written table without starting a new workbook origin.
    pub fn written(sheet: StyleSheet) -> Result<StyleSheet> {
        let bytes = sheet.to_part()?;
        Ok(sheet.written(bytes))
    }

    /// Captured descriptor count and retained bytes for these references.
    pub fn binding_info(sheet: &StyleSheet, ids: &[StyleId]) -> (usize, usize) {
        let mut bindings = StyleBindings::default();
        for id in ids {
            bindings.capture(sheet, *id);
        }
        (bindings.entries.len(), bindings.byte_size())
    }

    /// Rebind an inverse's distinct references using the production owner.
    pub fn rebind(
        source: &StyleSheet,
        target: &Arc<StyleSheet>,
        ids: &[StyleId],
    ) -> Result<(Arc<StyleSheet>, HashMap<StyleId, StyleId>)> {
        let mut bindings = StyleBindings::default();
        for id in ids {
            bindings.capture(source, *id);
        }
        target.rebind(bindings.iter())
    }

    /// Reconcile the complete appended suffix as a save adoption does.
    pub fn rebind_all(
        source: &StyleSheet,
        target: &Arc<StyleSheet>,
    ) -> Result<(Arc<StyleSheet>, HashMap<StyleId, StyleId>)> {
        target.rebind(source.bindings())
    }

    /// Read a styles part.
    pub fn from_xml(bytes: Vec<u8>) -> Result<StyleSheet> {
        StyleSheet::from_xml(bytes)
    }

    /// The entry displaying as `style`, appended where the part lacks it.
    pub fn intern(sheet: &mut StyleSheet, style: &CellStyle) -> Result<StyleId> {
        sheet.intern(style)
    }

    /// The part a save writes.
    pub fn to_part(sheet: &StyleSheet) -> Result<Vec<u8>> {
        sheet.to_part()
    }
}

/// The reader every part of the package is parsed with: whitespace kept,
/// because a string cell's spaces are its own.
pub(super) fn reader<R: BufRead>(source: R) -> Reader<R> {
    let mut reader = Reader::from_reader(source);
    reader.config_mut().trim_text(false);
    reader
}
