//! The one pass over a worksheet part: `<sheetData>` folded into rows of
//! cells, each cell's facts as the file states them.
//!
//! Every reader of a sheet - the random-access [`Sheet`](super::Sheet), the
//! record stream, the schema tally, the row count - is this parser with a
//! different consumer, so the grammar of a `<row>` and a `<c>` is read in one
//! place: the optional `r` on both (a running cursor supplies the row after
//! the previous one and the column after the previous cell), the `t`, the
//! `s`, the `cm`, `vm` and `ph`, the `<v>`, the `<is>` with its `t` and
//! `r/t` runs, the `<f>`. A row or cell whose `r` goes backwards is refused,
//! because Excel requires the order and repairs a file without it, and so is
//! an attribute the schema does not spell - an `s` past sixteen bits among
//! them - naming the cell.

use std::borrow::Cow;
use std::io::BufRead;
use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, NamespaceResolver, QName, ResolveResult};
use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use super::carried::{Capture, FrameReader, WorksheetFrame, write_raw};
use super::cell::{CellKind, CellRef, MAX_COLUMNS, MAX_ROWS};
use super::layout::{Layout, RowFormat};
use super::package::{NamespaceFamily, codec_error, local_name, reference_text, text_piece};
use super::style::StyleId;

/// The optional row/cell `r` cursor shared by worksheet reading and table
/// replacement. The caller adds its sheet and byte or cell location.
pub(crate) type CoordinateResult = std::result::Result<u32, (Option<CellRef>, SmolStr)>;

pub(crate) fn row_coordinate(next_row: u32, reference: Option<&str>) -> CoordinateResult {
    let row = match reference {
        Some(text) => text
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|row| (1..=MAX_ROWS).contains(row))
            .map(|row| row - 1)
            .ok_or_else(|| {
                (
                    None,
                    format_smolstr!("expected a row number from 1 to {MAX_ROWS}, got {text:?}"),
                )
            })?,
        None => next_row,
    };
    if row < next_row {
        return Err((
            None,
            format_smolstr!(
                "expected rows in ascending order, got row {} after row {}",
                row + 1,
                next_row
            ),
        ));
    }
    if row >= MAX_ROWS {
        return Err((
            None,
            format_smolstr!("expected at most {MAX_ROWS} rows, got row {}", row + 1),
        ));
    }
    Ok(row)
}

pub(crate) fn cell_coordinate(
    row: u32,
    next_column: u32,
    reference: Option<&str>,
) -> CoordinateResult {
    let column = match reference {
        Some(text) => {
            let reference: CellRef = text
                .trim()
                .parse()
                .map_err(|error: Error| (None, super::cell::wire_reason(&error)))?;
            if reference.row() != row {
                return Err((
                    Some(reference),
                    format_smolstr!("expected a cell of row {}, got {reference}", row + 1),
                ));
            }
            if reference.column() < next_column {
                return Err((
                    Some(reference),
                    format_smolstr!(
                        "expected cells in ascending column order, got {reference} after column {}",
                        CellRef::column_name(next_column - 1)
                    ),
                ));
            }
            reference.column()
        }
        None => next_column,
    };
    if column >= MAX_COLUMNS {
        return Err((
            None,
            format_smolstr!(
                "expected at most {MAX_COLUMNS} columns, got column {}",
                column + 1
            ),
        ));
    }
    Ok(column)
}

/// One `<c>` as the file states it, its text still the file's: a shared
/// string's index, a serial's digits, a boolean's `0` or `1`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawCell {
    pub(crate) column: u32,
    pub(crate) kind: CellKind,
    /// The `s` attribute, a `cellXfs` index; the default when absent.
    pub(crate) style: StyleId,
    /// The `cm` attribute: the cell metadata record a dynamic array's
    /// anchor names.
    pub(crate) cell_metadata: Option<u32>,
    /// The `vm` attribute: the value metadata record a rich value names.
    pub(crate) value_metadata: Option<u32>,
    /// The `ph` attribute: whether the cell shows its phonetic text.
    pub(crate) phonetic: bool,
    /// The `<v>` text, or the `<is>` text with its runs joined and its
    /// `_xHHHH_` escapes decoded; inline up to `SmolStr`'s capacity, which
    /// every serial and most text fits, so a cell costs no allocation.
    pub(crate) content: SmolStr,
    /// Whether the cell stated any content at all: `<c/>` states none.
    pub(crate) has_content: bool,
    /// The `<f>`, when the cell carries a formula.
    pub(crate) formula: Option<RawFormula>,
    /// The `<is>` of a rich inline string, raw: its runs, their fonts and
    /// its phonetic text, which the text alone does not say. Read only by a
    /// parse that keeps the part's frame.
    pub(crate) inline_runs: Option<Arc<[u8]>>,
}

impl RawCell {
    /// Borrow a cell's text, resolving a shared-string index once through
    /// the same located boundary for record reads and region occupancy.
    pub(crate) fn resolved_content<'a>(
        &'a self,
        strings: &'a super::shared_strings::SharedStrings,
        sheet: &str,
        reference: CellRef,
    ) -> Result<&'a str> {
        let refuse = |reason| Error::InvalidRecord {
            path: format_smolstr!("{sheet}!{reference}"),
            reason,
        };
        if self.kind == CellKind::SharedString && self.has_content {
            let index: usize = self.content.trim().parse().map_err(|_| {
                refuse(format_smolstr!(
                    "expected a shared string index, got {:?}",
                    self.content
                ))
            })?;
            return strings.get(index).map(|text| text.as_str()).ok_or_else(|| {
                refuse(format_smolstr!(
                    "expected a shared string index below {}, got {index}",
                    strings.len()
                ))
            });
        }
        Ok(self.content.as_str())
    }
}

/// One `<f>` as the file states it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawFormula {
    /// The text, entities resolved; empty for a shared formula's dependent.
    pub(crate) text: SmolStr,
    /// What the element states beside its text, the default for a plain
    /// `<f>`: held in the parser's own buffer, so a shared formula's
    /// dependent costs no allocation to read.
    pub(crate) attributes: FormulaAttributes,
}

/// What kind of formula an `<f>` states: its `t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FormulaKind {
    /// `normal`, or no `t`.
    #[default]
    Normal,
    /// `shared`: one formula of a group whose master states the text.
    Shared,
    /// `array`: a formula whose result fills `ref` - entered with
    /// Ctrl+Shift+Enter, or a dynamic array's anchor under a `cm`.
    Array,
    /// `dataTable`: a what-if table over `r1` and `r2`.
    DataTable,
}

/// The attributes of an `<f>` beside its text, as the file states them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FormulaAttributes {
    pub(crate) kind: FormulaKind,
    /// `ref`: the range an array, data table or shared group covers.
    pub(crate) reference: Option<SmolStr>,
    /// `si`: the shared group.
    pub(crate) shared_index: Option<u32>,
    /// `ca`: calculate the cell every time.
    pub(crate) always_calculate: bool,
    /// `aca`: calculate the array every time.
    pub(crate) array_always_calculate: bool,
    /// `dt2D`: a two-input data table.
    pub(crate) two_dimensional: bool,
    /// `dtr`: a one-input data table's input is a row.
    pub(crate) row_input: bool,
    /// `del1`, `del2`: the input cells were deleted.
    pub(crate) first_deleted: bool,
    pub(crate) second_deleted: bool,
    /// `r1`, `r2`: the input cells.
    pub(crate) first_input: Option<SmolStr>,
    pub(crate) second_input: Option<SmolStr>,
    /// `bx`: the formula assigns a value to a name.
    pub(crate) assigns: bool,
}

impl FormulaAttributes {
    /// Read the attributes of an `<f>`.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming the attribute whose value the schema does
    /// not spell.
    fn read(start: &BytesStart<'_>, position: usize) -> Result<Self> {
        let mut attributes = Self::default();
        for held in start.attributes().with_checks(false) {
            let held = held.map_err(|error| codec_error(position, error.to_string()))?;
            let value = held
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| codec_error(position, error.to_string()))?;
            let flag = || -> Result<bool> {
                match value.trim() {
                    "1" | "true" => Ok(true),
                    "0" | "false" => Ok(false),
                    other => Err(Error::Parse {
                        target: "formula attribute",
                        position: 0,
                        reason: format_smolstr!(
                            "expected 1, true, 0 or false in a formula's `{}`, got {other:?}",
                            String::from_utf8_lossy(local_name(held.key.as_ref()))
                        ),
                    }),
                }
            };
            match held.key.as_ref() {
                b"t" => {
                    attributes.kind = match value.trim() {
                        "normal" => FormulaKind::Normal,
                        "shared" => FormulaKind::Shared,
                        "array" => FormulaKind::Array,
                        "dataTable" => FormulaKind::DataTable,
                        other => {
                            return Err(Error::Parse {
                                target: "formula type",
                                position: 0,
                                reason: format_smolstr!(
                                    "expected normal, shared, array or dataTable for a formula's `t`, got {other:?}"
                                ),
                            });
                        }
                    };
                }
                b"ref" => attributes.reference = Some(SmolStr::new(value.trim())),
                b"si" => {
                    attributes.shared_index =
                        Some(value.trim().parse().map_err(|_| Error::Parse {
                            target: "formula attribute",
                            position: 0,
                            reason: format_smolstr!(
                                "expected a shared formula index in a formula's `si`, got {value:?}"
                            ),
                        })?);
                }
                b"ca" => attributes.always_calculate = flag()?,
                b"aca" => attributes.array_always_calculate = flag()?,
                b"dt2D" => attributes.two_dimensional = flag()?,
                b"dtr" => attributes.row_input = flag()?,
                b"del1" => attributes.first_deleted = flag()?,
                b"del2" => attributes.second_deleted = flag()?,
                b"r1" => attributes.first_input = Some(SmolStr::new(value.trim())),
                b"r2" => attributes.second_input = Some(SmolStr::new(value.trim())),
                b"bx" => attributes.assigns = flag()?,
                _ => {}
            }
        }
        Ok(attributes)
    }

    /// Whether the attributes say anything a formula written as a plain
    /// `<f>` must keep: an array or data table, or a calculation flag.
    pub(crate) fn is_kept(&self) -> bool {
        matches!(self.kind, FormulaKind::Array | FormulaKind::DataTable)
            || self.always_calculate
            || self.array_always_calculate
            || self.assigns
    }

    /// The attributes a formula written on its own keeps: a shared
    /// formula's group left out, since each cell is written plain.
    pub(crate) fn kept(&self) -> Option<Box<Self>> {
        if !self.is_kept() {
            return None;
        }
        let mut kept = self.clone();
        if kept.kind == FormulaKind::Shared {
            kept.kind = FormulaKind::Normal;
            kept.reference = None;
            kept.shared_index = None;
        }
        Some(Box::new(kept))
    }
}

/// One `<row>`: its zero-based index, its cells in column order, and what
/// it states beyond them when the parse keeps the frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RawRow {
    pub(crate) index: u32,
    pub(crate) cells: Vec<RawCell>,
    pub(crate) format: Option<RowFormat>,
}

/// Where the parser stands inside the part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// Outside `<sheetData>`.
    Outside,
    /// Between rows.
    Rows,
    /// Inside a `<row>`, between cells.
    Row,
    /// Inside a `<c>`, between its children.
    Cell,
    /// Inside `<v>`.
    Value,
    /// Inside `<f>`.
    Formula,
    /// Inside `<is>`, between its runs.
    Inline,
    /// Inside a `<t>` of `<is>` whose text is the cell's.
    InlineText,
}

/// What one event did to the cursor.
enum Step {
    /// Nothing a consumer sees.
    Continue,
    /// A row closed.
    Row(RawRow),
    /// The part ended.
    End,
}

/// The rows of one worksheet part, one at a time.
///
/// Rows are yielded in document order, which the parser holds to be
/// ascending; a row the part declares with no cell is yielded with none.
pub(crate) struct SheetRows<R: BufRead> {
    reader: Reader<R>,
    namespaces: NamespaceResolver,
    family: Option<NamespaceFamily>,
    /// Foreign elements inside sheetData are skipped by streaming reads.
    foreign_depth: usize,
    buffer: Vec<u8>,
    /// The sheet's name, which every refusal names as Excel does: `Trades!B3`.
    sheet: SmolStr,
    place: Place,
    /// Elements deep inside a subtree nothing is read from, such as `rPh`.
    skipping: usize,
    /// The row after the last one yielded, which a `<row>` with no `r` takes.
    next_row: u32,
    /// The cursor's row, once a `<row>` has opened.
    row: u32,
    /// The column after the last cell, which a `<c>` with no `r` takes.
    next_column: u32,
    cells: Vec<RawCell>,
    current: RawCell,
    /// The character data of the element the cursor is in, gathered across
    /// its text, CDATA and reference events; one buffer for the whole part.
    content: String,
    /// Whether the part ended.
    done: bool,
    /// Whether `<sheetData>` was seen, so a part without one is refused.
    saw_data: bool,
    /// Table metadata observes the existing frame without constructing rows.
    table_parts_only: bool,
    /// Default Rows selection observes coordinates, never RawCells.
    geometry_levels: Option<u32>,
    coordinates_only: bool,
    observed_dimension: Option<super::cell::CellRange>,
    /// What the part states outside the cells, gathered when the parse
    /// keeps the frame; `None` on the record path, which reads cells alone.
    frame: Option<Box<FrameReader>>,
    /// The raw `<is>` of the cell being read, when the frame is kept, in a
    /// buffer every inline string reuses - only a rich one is copied out -
    /// and whether it holds runs, `None` while no `<is>` is being read.
    inline: Vec<u8>,
    inline_rich: Option<bool>,
    /// The format of the row being read.
    row_format: Option<RowFormat>,
    /// The extra attributes of the last row that stated some, and the buffer
    /// they are gathered in: rows share one text.
    row_extra: String,
    last_row_extra: Option<Arc<str>>,
}

impl<R: BufRead> SheetRows<R> {
    /// Parse the worksheet part `source` of the sheet `sheet`, which the
    /// refusals name.
    pub(crate) fn new(source: R, sheet: impl Into<SmolStr>) -> Self {
        Self {
            reader: super::styles::reader(source),
            namespaces: NamespaceResolver::default(),
            family: None,
            foreign_depth: 0,
            buffer: Vec::with_capacity(256),
            sheet: sheet.into(),
            place: Place::Outside,
            skipping: 0,
            next_row: 0,
            row: 0,
            next_column: 0,
            cells: Vec::new(),
            current: RawCell::default(),
            content: String::new(),
            done: false,
            saw_data: false,
            table_parts_only: false,
            geometry_levels: None,
            coordinates_only: false,
            observed_dimension: None,
            frame: None,
            inline: Vec::new(),
            inline_rich: None,
            row_format: None,
            row_extra: String::new(),
            last_row_extra: None,
        }
    }

    /// Parse the worksheet part `source` of the sheet `sheet`, keeping what
    /// it states outside the cells - the frame and the layout
    /// [`Self::into_frame`] answers once the rows are read - and each row's
    /// format and each rich inline string's runs.
    pub(crate) fn capturing(source: R, sheet: impl Into<SmolStr>) -> Self {
        let mut rows = Self::new(source, sheet);
        rows.frame = Some(Box::default());
        rows
    }

    /// Observe table membership without constructing rows or cells.
    pub(crate) fn observing_tables(source: R, sheet: impl Into<SmolStr>) -> Self {
        let mut rows = Self::new(source, sheet);
        rows.frame = Some(Box::new(FrameReader::table_parts()));
        rows.table_parts_only = true;
        rows
    }

    /// Observe the existing worksheet event stream with row/cell construction
    /// skipped; retain only merge refs intersecting the header rectangle.
    pub(crate) fn observing_header_merges(
        source: R,
        sheet: impl Into<SmolStr>,
        window: super::cell::CellRange,
    ) -> Self {
        let mut rows = Self::new(source, sheet);
        rows.frame = Some(Box::new(FrameReader::header_merges(Some(window))));
        rows.table_parts_only = true;
        rows
    }

    /// Observe the actual dimension without RawRow/RawCell construction.
    /// The frame filters trailing merges once sheetData establishes an anchor.
    pub(crate) fn observing_header_geometry(
        source: R,
        sheet: impl Into<SmolStr>,
        levels: u32,
    ) -> Self {
        let mut rows = Self::new(source, sheet);
        rows.frame = Some(Box::new(FrameReader::header_merges(None)));
        rows.geometry_levels = Some(levels);
        rows.coordinates_only = true;
        rows
    }

    /// Infer from one ordinary row pass while capturing trailing merges.
    pub(crate) fn capturing_header_geometry(
        source: R,
        sheet: impl Into<SmolStr>,
        levels: u32,
    ) -> Self {
        let mut rows = Self::new(source, sheet);
        rows.frame = Some(Box::new(FrameReader::header_merges(None)));
        rows.geometry_levels = Some(levels);
        rows
    }

    pub(crate) fn into_header_geometry(
        self,
    ) -> Result<(Option<super::cell::CellRange>, Vec<super::cell::CellRange>)> {
        let dimension = self.observed_dimension;
        self.into_header_merges().map(|merges| (dimension, merges))
    }

    /// Tally body rows and capture bounded merges in the same inference pass.
    /// The final layout is available only after the iterator reaches EOF,
    /// because mergeCells follows sheetData in the worksheet grammar.
    pub(crate) fn capturing_header_merges(
        source: R,
        sheet: impl Into<SmolStr>,
        window: super::cell::CellRange,
    ) -> Self {
        let mut rows = Self::new(source, sheet);
        rows.frame = Some(Box::new(FrameReader::header_merges(Some(window))));
        rows
    }

    pub(crate) fn into_header_merges(self) -> Result<Vec<super::cell::CellRange>> {
        if !self.done || !self.frame.as_ref().is_some_and(|frame| frame.is_complete()) {
            return Err(Error::InvalidRecord {
                path: self.sheet,
                reason: SmolStr::new_static(
                    "expected a complete worksheet before reading its merged header spans",
                ),
            });
        }
        Ok(self
            .frame
            .map_or_else(Vec::new, |frame| frame.into_header_merges()))
    }

    /// The frame and the layout a capturing parse read, once every row has
    /// been read; `None` for a parse that keeps no frame.
    pub(crate) fn into_frame(self) -> Option<(WorksheetFrame, Layout)> {
        self.frame.and_then(|frame| frame.finish())
    }

    /// The byte position the reader stands at.
    fn byte_position(&self) -> usize {
        usize::try_from(self.reader.buffer_position()).unwrap_or(usize::MAX)
    }

    /// Refuse the part at the cursor, naming it and the cell.
    fn refuse(&self, reason: SmolStr) -> Error {
        self.refuse_at(
            CellRef::new(self.row, self.next_column.saturating_sub(1)),
            reason,
        )
    }

    /// Refuse the part at the cursor, naming the cell `reference`.
    fn refuse_at(&self, reference: CellRef, reason: SmolStr) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("{}!{reference}", self.sheet),
            reason: format_smolstr!("at byte {}: {reason}", self.byte_position()),
        }
    }

    /// Refuse what the part states outside the cells, naming the sheet and
    /// the element.
    fn refuse_outside(&self, error: Error, position: usize) -> Error {
        match error {
            Error::InvalidRecord { path, reason } => Error::InvalidRecord {
                path: format_smolstr!("{}!{}", self.sheet, path.trim_start_matches("$.")),
                reason: format_smolstr!("at byte {position}: {reason}"),
            },
            other => other,
        }
    }

    /// Hand back the cells of a row once read, so the next row fills the
    /// same buffer rather than allocating one of its own.
    pub(crate) fn recycle(&mut self, mut cells: Vec<RawCell>) {
        if cells.capacity() > self.cells.capacity() {
            cells.clear();
            self.cells = cells;
        }
    }

    /// Resolve the element's main/Strict SpreadsheetML scope once. The
    /// common cell path scans attributes but normalizes only declarations;
    /// NamespaceResolver owns scopes just as the package editor does.
    fn element_namespace(&mut self, event: &Event<'_>, position: usize) -> Result<bool> {
        let (Event::Start(start) | Event::Empty(start)) = event else {
            return Ok(true);
        };
        let level = self.namespaces.level().checked_add(1).ok_or_else(|| {
            codec_error(
                position,
                "expected XML nesting within the namespace resolver's limit",
            )
        })?;
        self.namespaces.set_level(level);
        let mut declarations = 0;
        for held in start.attributes().with_checks(false) {
            let held = held.map_err(|error| codec_error(position, error.to_string()))?;
            let Some(prefix) = QName(held.key.as_ref()).as_namespace_binding() else {
                continue;
            };
            declarations += 1;
            if declarations > self.namespaces.max_declarations_per_element() {
                return Err(codec_error(
                    position,
                    "too many namespace declarations on one element",
                ));
            }
            let value = held
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| codec_error(position, error.to_string()))?;
            self.namespaces
                .add(prefix, Namespace(value.as_bytes()))
                .map_err(|error| codec_error(position, error.to_string()))?;
        }
        let family = match self.namespaces.resolve_element(start.name()).0 {
            ResolveResult::Bound(uri) => std::str::from_utf8(uri.as_ref())
                .ok()
                .and_then(NamespaceFamily::from_namespace),
            ResolveResult::Unbound => None,
            ResolveResult::Unknown(prefix) => {
                return Err(Error::InvalidRecord {
                    path: self.sheet.clone(),
                    reason: format_smolstr!(
                        "expected a bound XML namespace prefix, got {:?} at byte {position}",
                        String::from_utf8_lossy(&prefix)
                    ),
                });
            }
        };
        if level == 1 {
            if self.family.is_some() {
                return Err(Error::InvalidRecord {
                    path: self.sheet.clone(),
                    reason: format_smolstr!(
                        "expected one worksheet root, got a second element at byte {position}"
                    ),
                });
            }
            if family.is_none() || local_name(start.name().as_ref()) != b"worksheet" {
                return Err(Error::InvalidRecord {
                    path: self.sheet.clone(),
                    reason: format_smolstr!(
                        "expected a SpreadsheetML worksheet root, got {:?} in {:?} at byte {position}",
                        String::from_utf8_lossy(start.name().as_ref()),
                        self.namespaces.resolve_element(start.name()).0,
                    ),
                });
            }
            self.family = family;
        }
        Ok(family.is_some() && family == self.family)
    }

    /// Read the next row, `None` at the end of the part.
    fn read_row(&mut self) -> Result<Option<RawRow>> {
        if self.done {
            return Ok(None);
        }
        loop {
            // The event borrows the buffer it was read into, so the buffer
            // leaves `self` for the duration of one event and comes back.
            let mut buffer = std::mem::take(&mut self.buffer);
            buffer.clear();
            let position = self.byte_position();
            let step = match self.reader.read_event_into(&mut buffer) {
                Ok(event) => {
                    let main = self.element_namespace(&event, position)?;
                    let step = self.step(&event, position, main);
                    if matches!(event, Event::End(_) | Event::Empty(_)) {
                        self.namespaces.pop();
                    }
                    step
                }
                Err(error) => Err(codec_error(position, error.to_string())),
            };
            self.buffer = buffer;
            match step? {
                Step::Continue => {}
                Step::Row(row) => return Ok(Some(row)),
                Step::End => {
                    self.done = true;
                    if !self.saw_data {
                        return Err(Error::InvalidRecord {
                            path: self.sheet.clone(),
                            reason: SmolStr::new_static(
                                "expected a worksheet part holding sheetData, got a part without one",
                            ),
                        });
                    }
                    return Ok(None);
                }
            }
        }
    }

    /// Fold one event into the cursor's state.
    fn step(&mut self, event: &Event<'_>, position: usize, main: bool) -> Result<Step> {
        if self.place == Place::Outside && !matches!(event, Event::Eof) {
            if let Some(frame) = self.frame.as_mut() {
                return match frame.event(event, main) {
                    Ok(Capture::Continue) => Ok(Step::Continue),
                    Ok(Capture::SheetData { empty }) => {
                        self.saw_data = true;
                        if !empty {
                            self.place = Place::Rows;
                            if self.table_parts_only {
                                // The same event loop resumes the frame reader
                                // after sheetData, where tableParts is stated.
                                self.skipping = 1;
                            }
                        }
                        Ok(Step::Continue)
                    }
                    Err(error) => Err(self.refuse_outside(error, position)),
                };
            }
        }
        if self.foreign_depth > 0 {
            match event {
                Event::Start(_) => self.foreign_depth += 1,
                Event::End(_) => self.foreign_depth -= 1,
                Event::Eof => {
                    return Err(self.refuse(SmolStr::new_static(
                        "expected the foreign worksheet element to close before EOF",
                    )));
                }
                _ => {}
            }
            return Ok(Step::Continue);
        }
        if !main && matches!(event, Event::Start(_) | Event::Empty(_)) {
            // A model rewrite owns the whole sheetData subtree. It cannot
            // silently erase unknown elements the streaming read can skip.
            if self.frame.as_deref().is_some_and(FrameReader::keeps_cells) {
                let tag = match event {
                    Event::Start(tag) | Event::Empty(tag) => tag,
                    _ => unreachable!(),
                };
                return Err(Error::InvalidRecord {
                    path: self.sheet.clone(),
                    reason: format_smolstr!(
                        "expected SpreadsheetML in sheetData, got {:?} in {:?} at byte {position}",
                        String::from_utf8_lossy(tag.name().as_ref()),
                        self.namespaces.resolve_element(tag.name()).0,
                    ),
                });
            }
            if matches!(event, Event::Start(_)) {
                self.foreign_depth = 1;
            }
            return Ok(Step::Continue);
        }
        if let Some(rich) = self.inline_rich.as_mut() {
            let closes = matches!(event, Event::End(end)
                if self.skipping == 0
                    && self.place == Place::Inline
                    && local_name(end.name().as_ref()) == b"is");
            if !closes {
                if let Event::Start(start) | Event::Empty(start) = event {
                    *rich |= matches!(
                        local_name(start.name().as_ref()),
                        b"r" | b"rPh" | b"phoneticPr"
                    );
                }
                write_raw(event, &mut self.inline);
            }
        }
        match event {
            Event::Start(start) => {
                if self.skipping > 0 {
                    self.skipping += 1;
                    return Ok(Step::Continue);
                }
                let qualified = start.name();
                let name = local_name(qualified.as_ref());
                Ok(self
                    .open(start, name, position, false)?
                    .map_or(Step::Continue, Step::Row))
            }
            Event::Empty(start) => {
                if self.skipping > 0 {
                    return Ok(Step::Continue);
                }
                let qualified = start.name();
                let name = local_name(qualified.as_ref());
                Ok(self
                    .open(start, name, position, true)?
                    .map_or(Step::Continue, Step::Row))
            }
            Event::End(end) => {
                if self.skipping > 0 {
                    self.skipping -= 1;
                    if self.table_parts_only && self.skipping == 0 && self.place == Place::Rows {
                        // The outer skip began at sheetData; its closing tag
                        // gives the existing FrameReader the following parts.
                        self.place = Place::Outside;
                    }
                    return Ok(Step::Continue);
                }
                let qualified = end.name();
                Ok(self
                    .close(local_name(qualified.as_ref()))
                    .map_or(Step::Continue, Step::Row))
            }
            Event::Text(held) => {
                if self.reads_text() {
                    let piece = text_piece(held.xml10_content(), position)?;
                    self.content.push_str(&piece);
                }
                Ok(Step::Continue)
            }
            Event::CData(held) => {
                if self.reads_text() {
                    let piece = text_piece(held.xml10_content(), position)?;
                    self.content.push_str(&piece);
                }
                Ok(Step::Continue)
            }
            Event::GeneralRef(_) => {
                if self.reads_text() {
                    let piece = reference_text(event, position)?;
                    self.content.push_str(&piece);
                }
                Ok(Step::Continue)
            }
            Event::Eof => {
                if self.namespaces.level() != 0
                    || self.skipping != 0
                    || self.place != Place::Outside
                    || self
                        .frame
                        .as_ref()
                        .is_some_and(|frame| !frame.is_complete())
                {
                    return Err(self.refuse(SmolStr::new_static(
                        "expected a complete worksheet, got EOF before its closing tag",
                    )));
                }
                Ok(Step::End)
            }
            _ => Ok(Step::Continue),
        }
    }

    /// Whether character data at the cursor is a cell's content.
    const fn reads_text(&self) -> bool {
        self.skipping == 0
            && matches!(
                self.place,
                Place::Value | Place::Formula | Place::InlineText
            )
    }

    /// Open an element; a self-closed `<row/>` or `<c/>` is opened and closed.
    fn open(
        &mut self,
        start: &quick_xml::events::BytesStart<'_>,
        name: &[u8],
        position: usize,
        empty: bool,
    ) -> Result<Option<RawRow>> {
        match (self.place, name) {
            (Place::Outside, b"sheetData") if self.namespaces.level() == 2 => {
                self.saw_data = true;
                if empty {
                    return Ok(None);
                }
                self.place = Place::Rows;
            }
            (Place::Rows, b"row") => {
                let reference = start
                    .try_get_attribute(b"r")
                    .map_err(|error| codec_error(position, error.to_string()))?
                    .map(|held| {
                        held.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|error| codec_error(position, error.to_string()))
                    })
                    .transpose()?;
                let row = row_coordinate(self.next_row, reference.as_deref())
                    .map_err(|(_, reason)| self.refuse(reason))?;
                self.row = row;
                // Rows header geometry anchors at its first physical cell,
                // then includes every later explicit row, even a null tail.
                if self.geometry_levels.is_some() {
                    if let Some(held) = self.observed_dimension {
                        self.observed_dimension = Some(super::cell::CellRange::new(
                            held.start(),
                            CellRef::new(row.max(held.end().row()), held.end().column()),
                        ));
                    }
                }
                self.next_row = row + 1;
                self.next_column = 0;
                self.cells.clear();
                self.row_format = None;
                if self.frame.as_ref().is_some_and(|frame| frame.keeps_cells()) {
                    let format =
                        RowFormat::read(start, &mut self.row_extra, &mut self.last_row_extra)
                            .map_err(|error| {
                                self.refuse_at(
                                    CellRef::new(row, 0),
                                    super::cell::wire_reason(&error),
                                )
                            })?;
                    self.row_format = (!format.is_empty()).then_some(format);
                }
                if empty {
                    return Ok((!self.coordinates_only).then_some(RawRow {
                        index: row,
                        cells: Vec::new(),
                        format: self.row_format.take(),
                    }));
                }
                self.place = Place::Row;
            }
            (Place::Row, b"c") => {
                let stated = CellAttributes::read(start, position)?;
                let column =
                    cell_coordinate(self.row, self.next_column, stated.reference.as_deref())
                        .map_err(|(at, reason)| match at {
                            Some(at) => self.refuse_at(at, reason),
                            None => self.refuse(reason),
                        })?;
                // The cursor is on this cell from here on, so a refusal of
                // its attributes names it.
                self.next_column = column + 1;
                if self.geometry_levels.is_some() {
                    let at = CellRef::new(self.row, column);
                    self.observed_dimension = Some(match self.observed_dimension {
                        None => super::cell::CellRange::new(at, at),
                        Some(held) => super::cell::CellRange::new(
                            CellRef::new(
                                held.start().row().min(at.row()),
                                held.start().column().min(at.column()),
                            ),
                            CellRef::new(
                                held.end().row().max(at.row()),
                                held.end().column().max(at.column()),
                            ),
                        ),
                    });
                    if self.coordinates_only {
                        if !empty {
                            self.skipping = 1;
                        }
                        return Ok(None);
                    }
                }
                let refused = |error: Error| self.refuse(super::cell::wire_reason(&error));
                let kind = match stated.kind {
                    Some(text) => CellKind::from_attribute(text.trim()).map_err(refused)?,
                    None => CellKind::Number,
                };
                let style = match stated.style {
                    Some(text) => StyleId::from_attribute(&text).map_err(refused)?,
                    None => StyleId::DEFAULT,
                };
                let cell_metadata = stated
                    .cell_metadata
                    .map(|text| metadata_index(&text, "cm"))
                    .transpose()
                    .map_err(refused)?;
                let value_metadata = stated
                    .value_metadata
                    .map(|text| metadata_index(&text, "vm"))
                    .transpose()
                    .map_err(refused)?;
                let phonetic = match stated.phonetic.as_deref().map(str::trim) {
                    None | Some("0" | "false") => false,
                    Some("1" | "true") => true,
                    Some(other) => {
                        return Err(self.refuse(format_smolstr!(
                            "expected 1, true, 0 or false in a cell's `ph`, got {other:?}"
                        )));
                    }
                };
                self.current = RawCell {
                    column,
                    kind,
                    style,
                    cell_metadata,
                    value_metadata,
                    phonetic,
                    content: SmolStr::default(),
                    has_content: false,
                    formula: None,
                    inline_runs: None,
                };
                self.content.clear();
                if empty {
                    self.cells.push(std::mem::take(&mut self.current));
                    return Ok(None);
                }
                self.place = Place::Cell;
            }
            (Place::Cell, b"v") => {
                self.current.has_content = true;
                if !empty {
                    self.place = Place::Value;
                }
            }
            (Place::Cell, b"f") => {
                let attributes = FormulaAttributes::read(start, position)
                    .map_err(|error| self.refuse(super::cell::wire_reason(&error)))?;
                self.current.formula = Some(RawFormula {
                    text: SmolStr::default(),
                    attributes,
                });
                if !empty {
                    self.place = Place::Formula;
                }
            }
            (Place::Cell, b"is") => {
                self.current.has_content = true;
                if !empty {
                    self.place = Place::Inline;
                    if self.frame.as_ref().is_some_and(|frame| frame.keeps_cells()) {
                        self.inline.clear();
                        self.inline_rich = Some(false);
                    }
                }
            }
            (Place::Inline, b"t") => {
                if !empty {
                    self.place = Place::InlineText;
                }
            }
            (Place::Inline, b"rPh" | b"phoneticPr") => {
                if !empty {
                    self.skipping = 1;
                }
            }
            // A rich run holds its `t`, read at the `Inline` place.
            (Place::Inline, b"r") => {}
            // `extLst`, `rPr` and anything else stated inside the data is
            // not a fact this reader keeps; a self-closed one has no subtree.
            (Place::Cell | Place::Row | Place::Rows | Place::Inline, _) if !empty => {
                self.skipping = 1;
            }
            _ => {}
        }
        Ok(None)
    }

    /// Close an element, yielding the row a `</row>` completes.
    fn close(&mut self, name: &[u8]) -> Option<RawRow> {
        match (self.place, name) {
            (Place::Value, b"v") => self.place = Place::Cell,
            (Place::Formula, b"f") => {
                if let Some(formula) = &mut self.current.formula {
                    formula.text = SmolStr::new(&self.content);
                }
                self.content.clear();
                self.place = Place::Cell;
            }
            (Place::InlineText, b"t") => self.place = Place::Inline,
            (Place::Inline, b"is") => {
                if self.inline_rich.take() == Some(true) {
                    self.current.inline_runs = Some(Arc::from(self.inline.as_slice()));
                }
                self.place = Place::Cell;
            }
            (Place::Cell, b"c") => {
                let mut cell = std::mem::take(&mut self.current);
                // Text written into the part itself - inline, or a formula's
                // cached string - carries the `_xHHHH_` escapes; a shared
                // string's content is its index.
                cell.content =
                    if matches!(cell.kind, CellKind::InlineString | CellKind::FormulaString) {
                        SmolStr::new(super::shared_strings::decode(&self.content))
                    } else {
                        SmolStr::new(&self.content)
                    };
                self.content.clear();
                self.cells.push(cell);
                self.place = Place::Row;
            }
            (Place::Row, b"row") => {
                self.place = Place::Rows;
                if self.coordinates_only {
                    return None;
                }
                return Some(RawRow {
                    index: self.row,
                    cells: std::mem::take(&mut self.cells),
                    format: self.row_format.take(),
                });
            }
            (Place::Rows, b"sheetData") => {
                self.place = Place::Outside;
                if let (Some(levels), Some(dimension), Some(frame)) = (
                    self.geometry_levels,
                    self.observed_dimension,
                    self.frame.as_mut(),
                ) {
                    let last = dimension
                        .start()
                        .row()
                        .saturating_add(levels.saturating_sub(1))
                        .min(MAX_ROWS - 1);
                    frame.set_header_window(super::cell::CellRange::new(
                        dimension.start(),
                        CellRef::new(last, dimension.end().column()),
                    ));
                }
            }
            _ => {}
        }
        None
    }
}

/// The attributes of one `<c>` this reader keeps, read in one pass over
/// the element, each borrowed from the event's buffer unless an entity had
/// to be resolved.
#[derive(Default)]
struct CellAttributes<'a> {
    reference: Option<Cow<'a, str>>,
    kind: Option<Cow<'a, str>>,
    style: Option<Cow<'a, str>>,
    cell_metadata: Option<Cow<'a, str>>,
    value_metadata: Option<Cow<'a, str>>,
    phonetic: Option<Cow<'a, str>>,
}

impl<'a> CellAttributes<'a> {
    /// Read the attributes of `start`; the first of a name is the one kept.
    fn read(start: &'a BytesStart<'_>, position: usize) -> Result<Self> {
        let mut stated = Self::default();
        for held in start.attributes().with_checks(false) {
            let held = held.map_err(|error| codec_error(position, error.to_string()))?;
            let slot = match held.key.as_ref() {
                b"r" => &mut stated.reference,
                b"t" => &mut stated.kind,
                b"s" => &mut stated.style,
                b"cm" => &mut stated.cell_metadata,
                b"vm" => &mut stated.value_metadata,
                b"ph" => &mut stated.phonetic,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(
                    held.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|error| codec_error(position, error.to_string()))?,
                );
            }
        }
        Ok(stated)
    }
}

/// The metadata record index a cell's `cm` or `vm` attribute states.
fn metadata_index(text: &str, name: &str) -> Result<u32> {
    text.trim().parse().map_err(|_| Error::Parse {
        target: "cell metadata",
        position: 0,
        reason: format_smolstr!("expected a metadata index in a cell's `{name}`, got {text:?}"),
    })
}

impl<R: BufRead> Iterator for SheetRows<R> {
    type Item = Result<RawRow>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.read_row() {
            Ok(row) => row.map(Ok),
            Err(error) => {
                self.done = true;
                Some(Err(error))
            }
        }
    }
}
