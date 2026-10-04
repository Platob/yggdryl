//! The streamed worksheet writer: a worksheet part rendered row by row off
//! the record columns of a stream, as the archive reads it.
//!
//! What Excel owns is spelled here and nowhere else: a boolean as `t="b"`, a
//! date, time, naive datetime or duration as its serial under the matching
//! style, a float that is not a number as the `#NUM!` error, and text as an
//! inline string (`t="inlineStr"`), so a stream of a million distinct strings
//! costs one batch and no table. Every other leaf is spelled as the XML codec
//! spells it and written as text: a decimal's digits, a code, an identifier,
//! a zoned datetime in ISO 8601 with its offset, bytes in base64. A nested
//! value is its JSON text, because a cell is flat.
//!
//! The writer is a [`Read`]: the archive pulls it as it deflates the member,
//! so the part is never held whole. It writes no `dimension`, which cannot be
//! known before the last row, and `sheetData` even when empty.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use smol_str::format_smolstr;

use crate::{DataType, Error, Field, Result, Scalar, Serie, SerieReader};

use super::cell::{CellRef, DateSystem, MAX_COLUMNS, MAX_ROWS, cell_text};
use super::package::NamespaceFamily;
use super::records::RowsWriteLayout;
use super::style::StyleId;
use super::styles::{NumberFormat, TemporalStyles};
use super::workbook::Workbook;

/// How one column's cells are written, decided once per stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// `t="b"`.
    Boolean,
    /// A serial under the format's style, or text where the value is zoned.
    Temporal,
    /// Digits, or `#NUM!` for NaN and infinity.
    Float,
    /// Text read off the column's own storage, no value built.
    Text,
    /// Any other leaf, spelled as the XML codec spells it.
    Leaf,
    /// A struct, a sequence or a map, as JSON text.
    Nested,
}

impl Kind {
    fn of(dtype: &DataType) -> Self {
        if matches!(dtype, DataType::Boolean) {
            return Self::Boolean;
        }
        if matches!(
            dtype,
            DataType::Float16 | DataType::Float32 | DataType::Float64
        ) {
            return Self::Float;
        }
        if dtype
            .id()
            .temporal_kind()
            .is_some_and(|kind| !matches!(kind, crate::TemporalKind::Interval))
        {
            return Self::Temporal;
        }
        if dtype.string_parameters().is_some() {
            return Self::Text;
        }
        if dtype.is_nested() {
            return Self::Nested;
        }
        Self::Leaf
    }
}

/// The number format a column of `dtype` writes its cells under, `None`
/// for a column whose cells are no serial: the rule
/// [`DateSystem::serial_of`] reads each value by, asked of the column.
fn temporal_format(dtype: &DataType) -> Option<NumberFormat> {
    let (unit, naive) = match dtype {
        DataType::DateTime64 { unit, timezone } => (*unit, timezone.is_naive()),
        _ => (crate::TimeUnit::Second, true),
    };
    NumberFormat::of_temporal(dtype.id(), unit, naive)
}

/// The temporal formats the physical columns write their cells under,
/// each once, in column order: what the styles beside the part must hold
/// before a row is rendered.
pub(crate) fn temporal_formats<'a>(
    fields: impl IntoIterator<Item = &'a Field>,
) -> Vec<NumberFormat> {
    let mut formats = Vec::new();
    for format in fields
        .into_iter()
        .filter_map(|field| temporal_format(field.dtype()))
    {
        if !formats.contains(&format) {
            formats.push(format);
        }
    }
    formats
}

/// Whether a text column's storage bytes are its UTF-8: true for the UTF-8
/// and US-ASCII leaves, false for windows-1252, which rides binary storage.
fn utf8_stored(child: &Serie) -> bool {
    child
        .field()
        .and_then(|field| field.dtype().string_parameters())
        .is_some_and(|parameters| {
            matches!(
                parameters.charset(),
                crate::Charset::Utf8 | crate::Charset::Ascii
            )
        })
}

/// The worksheet part of a record stream, rendered as it is read.
pub(super) struct SheetXml {
    batches: Option<SerieReader>,
    root: Field,
    kinds: Vec<Kind>,
    system: DateSystem,
    /// Resolved at workbook intake, copied once before the stream starts.
    family: NamespaceFamily,
    /// The styles the workbook this sheet joins holds for the temporal
    /// formats its columns write.
    temporal: TemporalStyles,
    /// The top-left cell the rows start at.
    anchor: CellRef,
    /// Whether the anchor's row names the columns.
    /// Shared header geometry; Rows owns its physical Field paths.
    header: WriteHeader,
    header_label: usize,
    merge_index: usize,
    /// The zero-based row the next record is written at.
    row: u32,
    /// The sheet's name, for the refusal a row past the grid gets.
    sheet: smol_str::SmolStr,
    pending: VecDeque<u8>,
    scratch: Vec<u8>,
    reference: String,
    stage: Stage,
    /// Where a failure lands, shared with the caller: the archive reading
    /// this stream sees an `io::Error`, the caller the typed refusal.
    failure: Arc<Mutex<Option<Error>>>,
}

/// The one writer's header mode; Rows owns a compiled physical layout.
pub(super) enum WriteHeader {
    None,
    Source,
    Rows(RowsWriteLayout),
}

impl WriteHeader {
    fn layout(&self) -> Option<&RowsWriteLayout> {
        match self {
            Self::Rows(layout) => Some(layout),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Prologue,
    Headers,
    Rows,
    Epilogue,
    Merges,
    MergeEnd,
    Done,
}

impl SheetXml {
    /// The part for `batches`, whose records are `root`'s, starting at
    /// `anchor` in sheet `sheet`, each temporal cell under the style
    /// `temporal` holds for its format. The workbook supplies both its date
    /// system and namespace family without reading another part.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the root holds more columns than
    /// fit from the anchor, before a byte is rendered.
    pub(super) fn new(
        batches: SerieReader,
        root: Field,
        sheet: impl Into<smol_str::SmolStr>,
        workbook: &Workbook,
        temporal: &TemporalStyles,
        anchor: CellRef,
        header: WriteHeader,
    ) -> Result<Self> {
        anchor.require_in_grid()?;
        let columns = header
            .layout()
            .map_or(root.fields().len(), |layout| layout.leaves().len());
        if u64::from(anchor.column()) + columns as u64 > u64::from(MAX_COLUMNS) {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected at most {} columns from {anchor}, got {columns}",
                    MAX_COLUMNS - anchor.column()
                ),
            });
        }
        let kinds = match header.layout() {
            Some(layout) => layout
                .leaves()
                .iter()
                .map(|leaf| Kind::of(leaf.field.dtype()))
                .collect(),
            None => root
                .fields()
                .iter()
                .map(|field| Kind::of(field.dtype()))
                .collect(),
        };
        Ok(Self {
            batches: Some(batches),
            root,
            kinds,
            system: workbook.date_system(),
            family: workbook.namespace_family(),
            temporal: *temporal,
            anchor,
            header,
            header_label: 0,
            merge_index: 0,
            row: anchor.row(),
            sheet: sheet.into(),
            pending: VecDeque::new(),
            scratch: Vec::with_capacity(crate::DEFAULT_STREAM_BATCH_SIZE),
            reference: String::with_capacity(12),
            stage: Stage::Prologue,
            failure: Arc::new(Mutex::new(None)),
        })
    }

    /// The slot a failure lands in, to read once the archive has drained the
    /// stream.
    pub(crate) fn failure(&self) -> Arc<Mutex<Option<Error>>> {
        Arc::clone(&self.failure)
    }

    /// Render the next piece into the scratch buffer.
    fn render(&mut self) -> Result<bool> {
        self.scratch.clear();
        match self.stage {
            Stage::Prologue => {
                write!(
                    self.scratch,
                    "<worksheet xmlns=\"{}\" xmlns:r=\"{}\"><sheetData>",
                    self.family.namespace(),
                    self.family.relationships_namespace()
                )?;
                self.stage = if !matches!(&self.header, WriteHeader::None) {
                    Stage::Headers
                } else {
                    Stage::Rows
                };
                Ok(true)
            }
            Stage::Headers => {
                self.open_row()?;
                if let WriteHeader::Rows(layout) = &self.header {
                    let labels = layout.labels();
                    while self.header_label < labels.len()
                        && labels[self.header_label].0.row() == self.row
                    {
                        let (at, label) = &labels[self.header_label];
                        self.reference.clear();
                        at.write_a1(&mut self.reference);
                        write!(
                            self.scratch,
                            "<c r=\"{}\" t=\"inlineStr\"><is>",
                            self.reference
                        )?;
                        super::shared_strings::write_text_element(&mut self.scratch, label)?;
                        write!(self.scratch, "</is></c>")?;
                        self.header_label += 1;
                    }
                } else {
                    for (offset, field) in self.root.fields().iter().enumerate() {
                        self.reference.clear();
                        CellRef::new(self.row, self.anchor.column() + offset as u32)
                            .write_a1(&mut self.reference);
                        write!(
                            self.scratch,
                            "<c r=\"{}\" t=\"inlineStr\"><is>",
                            self.reference
                        )?;
                        super::shared_strings::write_text_element(&mut self.scratch, field.name())?;
                        write!(self.scratch, "</is></c>")?;
                    }
                }
                write!(self.scratch, "</row>")?;
                self.row += 1;
                if let Some(layout) = self.header.layout() {
                    if let Some((at, _)) = layout.labels().get(self.header_label) {
                        self.row = at.row();
                    } else {
                        // Covered header coordinates belong to the merge span,
                        // not to record occupancy. Body rows remain explicit.
                        self.row = layout.first_body_row();
                        self.stage = Stage::Rows;
                    }
                } else {
                    self.stage = Stage::Rows;
                }
                Ok(true)
            }
            Stage::Rows => {
                let remaining = (MAX_ROWS - self.row) as usize;
                let sheet = self.sheet.as_str();
                let column = self.anchor.column();
                let next = self.batches.as_mut().and_then(|reader| {
                    reader.next_with_preflight(|rows| {
                        if rows > remaining {
                            return Err(Error::InvalidRecord {
                                path: format_smolstr!(
                                    "{sheet}!{}",
                                    CellRef::new(MAX_ROWS, column)
                                ),
                                reason: format_smolstr!(
                                    "expected at most {MAX_ROWS} rows in a worksheet, got a row {} past them",
                                    MAX_ROWS + 1
                                ),
                            }
                            .into());
                        }
                        Ok(())
                    })
                });
                let Some(batch) = next else {
                    self.batches = None;
                    self.stage = Stage::Epilogue;
                    return Ok(true);
                };
                let batch = batch?;
                self.render_batch(&batch)?;
                Ok(true)
            }
            Stage::Epilogue => {
                write!(self.scratch, "</sheetData>")?;
                if let Some(layout) = self
                    .header
                    .layout()
                    .filter(|layout| !layout.merges().is_empty())
                {
                    write!(
                        self.scratch,
                        "<mergeCells count=\"{}\">",
                        layout.merges().len()
                    )?;
                    self.stage = Stage::Merges;
                } else {
                    write!(self.scratch, "</worksheet>")?;
                    self.stage = Stage::Done;
                }
                Ok(true)
            }
            Stage::Merges => {
                let layout = self.header.layout().expect("Rows merge stage");
                let span = layout.merges()[self.merge_index];
                write!(self.scratch, "<mergeCell ref=\"{span}\"/>")?;
                self.merge_index += 1;
                if self.merge_index == layout.merges().len() {
                    self.stage = Stage::MergeEnd;
                }
                Ok(true)
            }
            Stage::MergeEnd => {
                write!(self.scratch, "</mergeCells></worksheet>")?;
                self.stage = Stage::Done;
                Ok(true)
            }
            Stage::Done => Ok(false),
        }
    }

    fn open_row(&mut self) -> Result<()> {
        if self.row >= MAX_ROWS {
            return Err(Error::InvalidRecord {
                path: format_smolstr!(
                    "{}!{}",
                    self.sheet,
                    CellRef::new(self.row, self.anchor.column())
                ),
                reason: format_smolstr!(
                    "expected at most {MAX_ROWS} rows in a worksheet, got a row {} past them",
                    self.row + 1
                ),
            });
        }
        write!(self.scratch, "<row r=\"{}\">", self.row + 1)?;
        Ok(())
    }

    /// Render every row of one record column.
    fn render_batch(&mut self, batch: &Serie) -> Result<()> {
        let Some(record) = batch.as_struct() else {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected a record column for the rows of a sheet, got {}",
                    batch.field().map_or("a run", Field::name)
                ),
            });
        };
        let children = record.children();
        if let Some(layout) = self.header.layout() {
            let mut bound = layout.bind_columns(batch, children)?;
            for index in 0..batch.len() {
                bound.check_row(layout, index, self.row + index as u32, &self.sheet)?;
            }
            self.render_columns(batch.len(), bound.leaves().iter().copied())
        } else {
            self.render_columns(batch.len(), children.iter())
        }
    }

    /// One row loop for flat and nested leaves; no per-row field traversal.
    fn render_columns<'a, I>(&mut self, count: usize, columns: I) -> Result<()>
    where
        I: Clone + ExactSizeIterator<Item = &'a Serie>,
    {
        if columns.len() != self.kinds.len() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected {} columns for the sheet, got {}",
                    self.kinds.len(),
                    columns.len()
                ),
            });
        }
        for index in 0..count {
            self.open_row()?;
            for (offset, child) in columns.clone().enumerate() {
                let kind = self.kinds[offset];
                let column = self.anchor.column() + offset as u32;
                self.reference.clear();
                CellRef::new(self.row, column).write_a1(&mut self.reference);
                let reference = std::mem::take(&mut self.reference);
                let written = CellWriter {
                    system: self.system,
                    temporal: &self.temporal,
                    namespace: None,
                }
                .write(&mut self.scratch, child, index, kind, &reference, None);
                self.reference = reference;
                written.map_err(|error| Error::InvalidRecord {
                    path: format_smolstr!("{}!{}", self.sheet, CellRef::new(self.row, column)),
                    reason: super::cell::wire_reason(&error),
                })?;
            }
            write!(self.scratch, "</row>")?;
            self.row += 1;
        }
        Ok(())
    }
}

/// The single cell encoder used by a whole worksheet and a selected table
/// body. The caller supplies style and namespace decisions made by its owner.
fn write_cell_start(
    scratch: &mut Vec<u8>,
    reference: &str,
    style: Option<StyleId>,
    namespace: Option<NamespaceFamily>,
    kind: Option<&str>,
) -> Result<()> {
    write!(scratch, "<c r=\"{reference}\"")?;
    if let Some(style) = style {
        write!(scratch, " s=\"{}\"", style.as_u16())?;
    }
    if let Some(namespace) = namespace {
        write!(scratch, " xmlns=\"{}\"", namespace.namespace())?;
    }
    if let Some(kind) = kind {
        write!(scratch, " t=\"{kind}\"")?;
    }
    Ok(())
}

/// Encoding context shared by the full-sheet and selected-table writers.
struct CellWriter<'a> {
    system: DateSystem,
    temporal: &'a TemporalStyles,
    namespace: Option<NamespaceFamily>,
}

impl CellWriter<'_> {
    /// Write one non-null cell. A null produces no bytes so callers can choose
    /// whether to omit it (whole sheet) or retain an empty addressed cell (table).
    fn write(
        &self,
        scratch: &mut Vec<u8>,
        child: &Serie,
        index: usize,
        kind: Kind,
        reference: &str,
        style: Option<StyleId>,
    ) -> Result<()> {
        let Self {
            system,
            temporal,
            namespace,
        } = *self;
        if child.is_null(index)? {
            return Ok(());
        }
        match kind {
            Kind::Text if child.is_string_storage() && utf8_stored(child) => {
                let Some(bytes) = child.value_bytes(index) else {
                    return Ok(());
                };
                let text = std::str::from_utf8(bytes).map_err(|error| Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$"),
                    reason: format_smolstr!("the column holds bytes that are not UTF-8: {error}"),
                })?;
                write_text_cell(
                    scratch,
                    reference,
                    text.trim_end_matches('\0'),
                    style,
                    namespace,
                )
            }
            Kind::Boolean => {
                let value = child.scalar(index)?;
                write_cell_start(scratch, reference, style, namespace, Some("b"))?;
                write!(
                    scratch,
                    "><v>{}</v></c>",
                    u8::from(value.as_bool().unwrap_or(false))
                )?;
                Ok(())
            }
            Kind::Float => {
                let value = child.scalar(index)?;
                let number = value.as_f64().unwrap_or(f64::NAN);
                if number.is_finite() {
                    write_cell_start(scratch, reference, style, namespace, None)?;
                    if number.fract() == 0.0 && number.abs() < 1e15 {
                        write!(scratch, "><v>{}</v></c>", number as i64)?;
                    } else {
                        let mut buffer = ryu::Buffer::new();
                        write!(scratch, "><v>{}</v></c>", buffer.format(number))?;
                    }
                } else {
                    write_cell_start(scratch, reference, style, namespace, Some("e"))?;
                    write!(scratch, "><v>#NUM!</v></c>")?;
                }
                Ok(())
            }
            Kind::Temporal => {
                let value = child.scalar(index)?;
                match system.serial_of(&value)? {
                    Some((serial, format)) => {
                        let temporal_style = temporal.get(format).ok_or_else(|| Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$"),
                        reason: format_smolstr!(
                            "expected a style for the {format} format among the ones interned for the columns, got none"),
                    })?;
                        let chosen = style.or_else(|| {
                            (temporal_style != StyleId::DEFAULT).then_some(temporal_style)
                        });
                        write_cell_start(scratch, reference, chosen, namespace, None)?;
                        write!(scratch, "><v>{}</v></c>", super::cell::serial_text(serial))?;
                        Ok(())
                    }
                    None => {
                        write_text_cell(scratch, reference, &cell_text(&value), style, namespace)
                    }
                }
            }
            Kind::Nested => {
                let value = child.scalar(index)?;
                write_text_cell(scratch, reference, &value.into_json()?, style, namespace)
            }
            Kind::Text | Kind::Leaf => {
                let value = child.scalar(index)?;
                match &value {
                    Scalar::Null => Ok(()),
                    crate::string_scalars!(text) => {
                        write_text_cell(scratch, reference, text.as_str(), style, namespace)
                    }
                    Scalar::Int8(_)
                    | Scalar::Int16(_)
                    | Scalar::Int32(_)
                    | Scalar::Int64(_)
                    | Scalar::UInt8(_)
                    | Scalar::UInt16(_)
                    | Scalar::UInt32(_)
                    | Scalar::UInt64(_)
                    | Scalar::Int128(_)
                    | Scalar::UInt128(_)
                    | Scalar::Decimal32(_)
                    | Scalar::Decimal64(_)
                    | Scalar::Decimal128(_)
                    | Scalar::Decimal256(_)
                    | Scalar::Decimal(_)
                    | Scalar::BigDecimal(_) => {
                        write_cell_start(scratch, reference, style, namespace, None)?;
                        write!(scratch, "><v>")?;
                        crate::xml::write_leaf_text(scratch, &value, "cell")?;
                        write!(scratch, "</v></c>")?;
                        Ok(())
                    }
                    _ => write_text_cell(scratch, reference, &cell_text(&value), style, namespace),
                }
            }
        }
    }
}

fn write_text_cell(
    scratch: &mut Vec<u8>,
    reference: &str,
    text: &str,
    style: Option<StyleId>,
    namespace: Option<NamespaceFamily>,
) -> Result<()> {
    let length = text.chars().count();
    if length > super::cell::MAX_CELL_TEXT {
        return Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: format_smolstr!(
                "expected at most {} characters in a cell, got {length}",
                super::cell::MAX_CELL_TEXT
            ),
        });
    }
    write_cell_start(scratch, reference, style, namespace, Some("inlineStr"))?;
    write!(scratch, "><is>")?;
    super::shared_strings::write_text_element(scratch, text)?;
    write!(scratch, "</is></c>")?;
    Ok(())
}

impl Read for SheetXml {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        while self.pending.is_empty() {
            if self.stage == Stage::Done {
                return Ok(0);
            }
            match self.render() {
                Ok(true) => self.pending.extend(self.scratch.drain(..)),
                Ok(false) => return Ok(0),
                Err(error) => {
                    let message = error.to_string();
                    self.stage = Stage::Done;
                    if let Ok(mut slot) = self.failure.lock() {
                        *slot = Some(error);
                    }
                    return Err(std::io::Error::other(message));
                }
            }
        }
        let read = self.pending.len().min(target.len());
        for (slot, byte) in target.iter_mut().zip(self.pending.drain(..read)) {
            *slot = byte;
        }
        Ok(read)
    }
}

/// Overlay one named table's existing body cells without rewriting any other
/// worksheet item. Input batches land one at a time. The plan retains changed
/// cell bytes until read, and borrows unchanged ranges from the source Arc;
/// it does not materialize a second complete worksheet or build Scalar rows.
pub(super) struct TableWritten {
    pub(super) xml: super::package::DocumentReader,
    /// Exclusive row bound of the body actually consumed from the stream.
    pub(super) after: u32,
}

struct TotalsCells {
    cells: Vec<Option<Vec<u8>>>,
    unresolved: Vec<bool>,
    row_style: Option<StyleId>,
    row_scope: bool,
}

/// One bounded read of the old totals row through the worksheet package
/// editor. All other rows take its borrowed-tag raw-skip path.
struct TotalsCapture<'a> {
    sheet: &'a str,
    table: &'a super::table::Table,
    old_row: u32,
    next_row: u32,
    current_row: Option<u32>,
    next_column: u32,
    pending: Option<(usize, CellRef)>,
    cells: TotalsCells,
    family: Option<NamespaceFamily>,
    main: Vec<bool>,
    saw_root: bool,
    saw_row: bool,
}

impl<'a> TotalsCapture<'a> {
    fn new(sheet: &'a str, table: &'a super::table::Table) -> Self {
        let width = table.range.column_size() as usize;
        Self {
            sheet,
            table,
            old_row: table.range.end().row(),
            next_row: 0,
            current_row: None,
            next_column: 0,
            pending: None,
            cells: TotalsCells {
                cells: vec![None; width],
                unresolved: vec![false; width],
                row_style: None,
                row_scope: false,
            },
            family: None,
            main: Vec::new(),
            saw_root: false,
            saw_row: false,
        }
    }

    fn refuse(&self, at: CellRef, reason: impl Into<smol_str::SmolStr>) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("{}!{at}", self.sheet),
            reason: reason.into(),
        }
    }

    fn finish(self) -> Result<TotalsCells> {
        if !self.saw_root {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.table[{}]", self.table.name),
                reason: smol_str::SmolStr::new_static("expected a SpreadsheetML worksheet root"),
            });
        }
        Ok(self.cells)
    }
}

impl super::package::Edits for TotalsCapture<'_> {
    fn keep_subtree(
        &mut self,
        path: &[smol_str::SmolStr],
        attributes: &[(smol_str::SmolStr, String)],
        namespace: &quick_xml::name::ResolveResult<'_>,
    ) -> Result<bool> {
        if path.len() != 3
            || path[0] != "worksheet"
            || path[1] != "sheetData"
            || path[2] != "row"
            || self.main.last().copied() != Some(true)
        {
            return Ok(false);
        }
        if TableBody::family_of(namespace) != self.family {
            return Ok(true);
        }
        let mut references = attributes.iter().filter(|(key, _)| key == "r");
        let reference = references.next().map(|(_, value)| value.as_str());
        if references.next().is_some() {
            return Err(self.refuse(
                CellRef::new(self.old_row, self.table.range.start().column()),
                "expected one row r attribute",
            ));
        }
        let row =
            super::parser::row_coordinate(self.next_row, reference).map_err(|(at, reason)| {
                self.refuse(
                    at.unwrap_or(CellRef::new(
                        self.old_row,
                        self.table.range.start().column(),
                    )),
                    reason,
                )
            })?;
        if row == self.old_row {
            return Ok(false);
        }
        self.next_row = row + 1;
        Ok(true)
    }

    fn keep_raw_subtree(
        &mut self,
        path: &[smol_str::SmolStr],
        tag: &quick_xml::events::BytesStart<'_>,
        namespace: &quick_xml::name::ResolveResult<'_>,
        _: usize,
    ) -> Result<Option<Vec<u8>>> {
        let family = TableBody::family_of(namespace);
        if family != self.family || self.main.last().copied() != Some(true) {
            return Ok(None);
        }
        if path.len() == 2
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && super::package::local_name(tag.name().as_ref()) == b"row"
        {
            let mut row = None;
            for attribute in tag.attributes().with_checks(false) {
                let attribute = attribute.map_err(|error| {
                    self.refuse(
                        CellRef::new(self.old_row, self.table.range.start().column()),
                        format_smolstr!("expected valid totals row attribute: {error}"),
                    )
                })?;
                if attribute.key.as_namespace_binding().is_some() {
                    return Ok(None);
                }
                if attribute.key.as_ref() == b"r" {
                    if row.is_some() {
                        return Ok(None);
                    }
                    let value = attribute
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|error| {
                            self.refuse(
                                CellRef::new(self.old_row, self.table.range.start().column()),
                                format_smolstr!("expected valid totals row r: {error}"),
                            )
                        })?;
                    row = Some(
                        super::parser::row_coordinate(self.next_row, Some(value.as_ref()))
                            .map_err(|(at, reason)| {
                                self.refuse(
                                    at.unwrap_or(CellRef::new(
                                        self.old_row,
                                        self.table.range.start().column(),
                                    )),
                                    reason,
                                )
                            })?,
                    );
                }
            }
            let row = row
                .map(Ok)
                .unwrap_or_else(|| super::parser::row_coordinate(self.next_row, None))
                .map_err(|(at, reason)| {
                    self.refuse(
                        at.unwrap_or(CellRef::new(
                            self.old_row,
                            self.table.range.start().column(),
                        )),
                        reason,
                    )
                })?;
            if row != self.old_row {
                self.next_row = row + 1;
                return Ok(Some(Vec::new()));
            }
            return Ok(None);
        }
        if path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && self.current_row == Some(self.old_row)
            && super::package::local_name(tag.name().as_ref()) == b"c"
        {
            let mut reference = None;
            let mut declared = false;
            for attribute in tag.attributes().with_checks(false) {
                let attribute = attribute.map_err(|error| {
                    self.refuse(
                        CellRef::new(self.old_row, self.next_column.min(MAX_COLUMNS - 1)),
                        format_smolstr!("expected valid totals cell attribute: {error}"),
                    )
                })?;
                if attribute.key.as_namespace_binding().is_some() {
                    declared = true;
                    continue;
                }
                if attribute.key.as_ref() == b"r" {
                    let value = attribute
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|error| {
                            self.refuse(
                                CellRef::new(self.old_row, self.next_column.min(MAX_COLUMNS - 1)),
                                format_smolstr!("expected valid totals cell r: {error}"),
                            )
                        })?;
                    reference = Some(value);
                }
            }
            let column = super::parser::cell_coordinate(
                self.old_row,
                self.next_column,
                reference.as_deref(),
            )
            .map_err(|(at, reason)| {
                self.refuse(
                    at.unwrap_or(CellRef::new(
                        self.old_row,
                        self.next_column.min(MAX_COLUMNS - 1),
                    )),
                    reason,
                )
            })?;
            self.next_column = column + 1;
            let at = CellRef::new(self.old_row, column);
            if declared && self.table.range.contains_column(column) {
                self.cells.unresolved[(column - self.table.range.start().column()) as usize] = true;
            }
            if declared {
                self.pending = None;
                return Ok(None);
            }
            if !self.table.range.contains_column(column) {
                self.pending = None;
                return Ok(Some(Vec::new()));
            }
            for attribute in tag.attributes().with_checks(false) {
                let attribute = attribute.map_err(|error| {
                    self.refuse(
                        at,
                        format_smolstr!("expected valid totals cell attribute: {error}"),
                    )
                })?;
                if attribute.key.as_ref() == b"s" {
                    let value = attribute
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|error| {
                            self.refuse(
                                at,
                                format_smolstr!("expected valid totals cell s: {error}"),
                            )
                        })?;
                    StyleId::from_attribute(&value)
                        .map_err(|error| self.refuse(at, super::cell::wire_reason(&error)))?;
                }
            }
            self.pending = Some(((column - self.table.range.start().column()) as usize, at));
            return Ok(Some(Vec::new()));
        }
        Ok(None)
    }

    fn raw_subtree(
        &mut self,
        path: &[smol_str::SmolStr],
        _: &quick_xml::events::BytesStart<'_>,
        raw: &[u8],
    ) -> Result<()> {
        if path.len() == 3
            && path[2] == "row"
            && let Some((offset, at)) = self.pending.take()
        {
            if self.cells.cells[offset].is_some() {
                return Err(self.refuse(at, "expected one totals cell at this coordinate"));
            }
            self.cells.cells[offset] = Some(raw.to_vec());
        }
        Ok(())
    }

    fn start(
        &mut self,
        path: &[smol_str::SmolStr],
        attributes: &[(smol_str::SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<super::package::Tag> {
        let family = TableBody::family_of(&namespace);
        if path.len() == 1 {
            if path[0] != "worksheet" || family.is_none() || self.saw_root {
                return Err(self.refuse(
                    CellRef::new(self.old_row, self.table.range.start().column()),
                    "expected SpreadsheetML worksheet root",
                ));
            }
            self.family = family;
            self.saw_root = true;
            self.main.push(true);
            return Ok(super::package::Tag::Keep);
        }
        let main = self.main.last().copied().unwrap_or(false) && family == self.family;
        self.main.push(main);
        if main
            && path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
        {
            let reference = TableBody::attribute(attributes, "r");
            let row = super::parser::row_coordinate(self.next_row, reference).map_err(
                |(at, reason)| {
                    self.refuse(
                        at.unwrap_or(CellRef::new(
                            self.old_row,
                            self.table.range.start().column(),
                        )),
                        reason,
                    )
                },
            )?;
            self.next_row = row + 1;
            if row == self.old_row {
                if self.saw_row {
                    return Err(self.refuse(
                        CellRef::new(row, self.table.range.start().column()),
                        "expected one totals row",
                    ));
                }
                self.saw_row = true;
                self.current_row = Some(row);
                self.next_column = 0;
                self.cells.row_scope = attributes.iter().any(|(key, _)| {
                    key == "xml:space" || key == "xmlns" || key.starts_with("xmlns:")
                });
                let mut format = super::layout::RowFormat::default();
                for (key, value) in attributes {
                    format
                        .read_style_attribute(key.as_bytes(), value)
                        .map_err(|error| {
                            self.refuse(
                                CellRef::new(row, self.table.range.start().column()),
                                super::cell::wire_reason(&error),
                            )
                        })?;
                }
                self.cells.row_style = format.applied_style();
            }
        }
        Ok(super::package::Tag::Keep)
    }

    fn end(
        &mut self,
        path: &[smol_str::SmolStr],
        _: usize,
        _: usize,
        _: &[(smol_str::SmolStr, String)],
    ) -> super::package::Tag {
        self.main.pop();
        if path.len() == 3 && path[2] == "row" {
            self.current_row = None
        }
        super::package::Tag::Keep
    }
}

struct TotalsCellInspect<'a> {
    at: CellRef,
    sheet: &'a str,
    has_formula: bool,
    family: NamespaceFamily,
    main: Vec<bool>,
    closed_main: bool,
}

struct RetagTotalsCell {
    reference: String,
    seen: bool,
    clear_cached: bool,
    family: NamespaceFamily,
}

fn total_cell_main(
    namespace: &quick_xml::name::ResolveResult<'_>,
    family: NamespaceFamily,
) -> bool {
    match namespace {
        quick_xml::name::ResolveResult::Unbound => true,
        quick_xml::name::ResolveResult::Bound(uri) => uri.as_ref() == family.namespace().as_bytes(),
        _ => false,
    }
}

impl super::package::Edits for RetagTotalsCell {
    fn start(
        &mut self,
        path: &[smol_str::SmolStr],
        attributes: &[(smol_str::SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<super::package::Tag> {
        if path.len() == 1 && path[0] == "c" {
            self.seen = true;
            return Ok(super::package::Tag::Set(vec![(
                smol_str::SmolStr::new_static("r"),
                Some(self.reference.clone()),
            )]));
        }
        if self.clear_cached
            && path.len() == 2
            && path[0] == "c"
            && path[1] == "v"
            && total_cell_main(&namespace, self.family)
            && !attributes
                .iter()
                .any(|(key, value)| key == "xmlns" && value.is_empty())
        {
            return Ok(super::package::Tag::Drop);
        }
        Ok(super::package::Tag::Keep)
    }
}

impl super::package::Edits for TotalsCellInspect<'_> {
    fn start(
        &mut self,
        path: &[smol_str::SmolStr],
        attributes: &[(smol_str::SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<super::package::Tag> {
        if path.len() > 2 && path[0] == "c" && path[1] == "f" && self.main.last() == Some(&true) {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{}!{}", self.sheet, self.at),
                reason: smol_str::SmolStr::new_static(
                    "expected a text-only totals formula during movement",
                ),
            });
        }
        let main = self.main.last().copied().unwrap_or(true)
            && total_cell_main(&namespace, self.family)
            && !attributes
                .iter()
                .any(|(key, value)| key == "xmlns" && value.is_empty());
        self.main.push(main);
        if main && path.len() == 2 && path[0] == "c" && path[1] == "f" {
            self.has_formula = true;
            if attributes
                .iter()
                .any(|(key, value)| key == "si" || key == "ref" || key == "t" && value != "normal")
            {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}!{}", self.sheet, self.at),
                    reason: smol_str::SmolStr::new_static("expected a standalone totals formula"),
                });
            }
        }
        Ok(super::package::Tag::Keep)
    }

    fn text(&mut self, path: &[smol_str::SmolStr], value: &str) -> Result<Option<String>> {
        if self.closed_main && path.len() == 2 && path[0] == "c" && path[1] == "f" {
            let formula = super::formula::Formula::from_file(value, self.at);
            if formula
                .shape()
                .tokens
                .iter()
                .any(|token| matches!(token, super::formula::shape::Token::Reference(_)))
            {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}!{}", self.sheet, self.at),
                    reason: smol_str::SmolStr::new_static(
                        "expected a totals formula without A1 references during resize",
                    ),
                });
            }
        }
        Ok(None)
    }

    fn end(
        &mut self,
        _: &[smol_str::SmolStr],
        _: usize,
        _: usize,
        _: &[(smol_str::SmolStr, String)],
    ) -> super::package::Tag {
        self.closed_main = self.main.pop().unwrap_or(false);
        super::package::Tag::Keep
    }

    fn replacement(
        &mut self,
        _: &[smol_str::SmolStr],
        _: &[(smol_str::SmolStr, String)],
        qualified: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        if qualified.contains(&b':') {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{}!{}", self.sheet, self.at),
                reason: smol_str::SmolStr::new_static(
                    "expected a totals cell without inherited prefixed markup",
                ),
            });
        }
        Ok(None)
    }
}

pub(super) fn overwrite_table_body(
    source: std::sync::Arc<[u8]>,
    rows: SerieReader,
    table: &super::table::Table,
    columns: Vec<usize>,
    sheet: smol_str::SmolStr,
    system: DateSystem,
    splice: &mut super::styles::Splice<'_>,
) -> Result<TableWritten> {
    use super::package::{edit_document, edit_reader};
    let root = rows.field();
    let first = table
        .range
        .start()
        .row()
        .checked_add(table.header_rows)
        .ok_or_else(|| Error::InvalidRecord {
            path: table.name.clone(),
            reason: smol_str::SmolStr::new_static("expected table header within the grid"),
        })?;
    let after = table
        .range
        .end()
        .row()
        .checked_add(1)
        .and_then(|end| end.checked_sub(table.totals_rows))
        .filter(|end| *end >= first)
        .ok_or_else(|| Error::InvalidRecord {
            path: table.name.clone(),
            reason: smol_str::SmolStr::new_static("expected table totals within the grid"),
        })?;
    let kinds = columns
        .iter()
        .map(|&input| Kind::of(root.fields()[input].dtype()))
        .collect();
    let formats = columns
        .iter()
        .map(|&input| temporal_format(root.fields()[input].dtype()))
        .collect();
    let width = table.range.column_size() as usize;
    let totals = if table.totals_rows == 1 {
        let mut capture = TotalsCapture::new(&sheet, table);
        edit_document(&source, &mut capture)?;
        Some(capture.finish()?)
    } else if table.totals_rows == 0 {
        None
    } else {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("$.table[{}]", table.name),
            reason: smol_str::SmolStr::new_static(
                "expected at most one totals row for a body resize",
            ),
        });
    };
    let mut edit = TableBody {
        sheet,
        table: table.name.clone(),
        range: table.range,
        first,
        after,
        rows,
        batch: None,
        index: 0,
        written: 0,
        totals,
        totals_written: false,
        totals_target: false,
        old_totals: (table.totals_rows == 1).then_some(after),
        ended: false,
        clearing: false,
        columns,
        kinds,
        formats,
        seen: vec![false; width],
        default_columns: Vec::new(),
        next_style_column: 0,
        row_style: None,
        next_column: 0,
        next_source_row: 0,
        source_row: None,
        next_source_column: 0,
        source_cell: None,
        current_row: None,
        current_cell: None,
        extension_cell: false,
        main: Vec::new(),
        closed_main: false,
        family: None,
        saw_root: false,
        saw_data: false,
        system,
        splice,
        reference: String::with_capacity(12),
    };
    let edited = edit_reader(source, &mut edit)?;
    let after = edit.finish()?;
    Ok(TableWritten { xml: edited, after })
}

struct TableBody<'a, 'b> {
    sheet: smol_str::SmolStr,
    table: smol_str::SmolStr,
    range: super::cell::CellRange,
    first: u32,
    after: u32,
    rows: SerieReader,
    batch: Option<Serie>,
    index: usize,
    written: u32,
    totals: Option<TotalsCells>,
    totals_written: bool,
    totals_target: bool,
    old_totals: Option<u32>,
    ended: bool,
    /// A former body row beyond the stream's new end has its selected cells
    /// removed after their subtree has passed the ordinary refusal checks.
    clearing: bool,
    /// Physical table column -> input child, compiled once from Header.
    columns: Vec<usize>,
    kinds: Vec<Kind>,
    formats: Vec<Option<NumberFormat>>,
    seen: Vec<bool>,
    /// Styles for only the selected width, allocated only if a col states one.
    default_columns: Vec<Option<StyleId>>,
    next_style_column: u32,
    row_style: Option<StyleId>,
    next_column: usize,
    /// The physical cursors include rows and cells outside the table; OOXML
    /// permits omitted `r` attributes and assigns the next coordinate.
    next_source_row: u32,
    source_row: Option<u32>,
    next_source_column: u32,
    source_cell: Option<CellRef>,
    current_row: Option<u32>,
    current_cell: Option<(usize, CellRef, Option<StyleId>)>,
    extension_cell: bool,
    main: Vec<bool>,
    closed_main: bool,
    family: Option<NamespaceFamily>,
    saw_root: bool,
    saw_data: bool,
    system: DateSystem,
    splice: &'a mut super::styles::Splice<'b>,
    reference: String,
}

impl TableBody<'_, '_> {
    fn refusal(&self, at: CellRef, reason: impl Into<smol_str::SmolStr>) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("{}!{at}", self.sheet),
            reason: reason.into(),
        }
    }

    fn table_refusal(&self, reason: impl Into<smol_str::SmolStr>) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("$.table[{}]", self.table),
            reason: reason.into(),
        }
    }

    fn family_of(namespace: &quick_xml::name::ResolveResult<'_>) -> Option<NamespaceFamily> {
        match namespace {
            quick_xml::name::ResolveResult::Bound(uri) => std::str::from_utf8(uri.as_ref())
                .ok()
                .and_then(NamespaceFamily::from_namespace),
            _ => None,
        }
    }

    fn attribute<'a>(attributes: &'a [(smol_str::SmolStr, String)], name: &str) -> Option<&'a str> {
        attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn row_number(&self, attributes: &[(smol_str::SmolStr, String)]) -> Result<u32> {
        let at = CellRef::new(
            self.next_source_row.min(MAX_ROWS - 1),
            self.range.start().column(),
        );
        let mut references = attributes.iter().filter(|(key, _)| key == "r");
        let first = references.next().map(|(_, value)| value.as_str());
        if references.next().is_some() {
            return Err(self.refusal(at, "expected one row r attribute, got duplicates"));
        }
        super::parser::row_coordinate(self.next_source_row, first)
            .map_err(|(located, reason)| self.refusal(located.unwrap_or(at), reason))
    }

    fn cell_number(&self, attributes: &[(smol_str::SmolStr, String)]) -> Result<CellRef> {
        let row = self.source_row.expect("open worksheet row");
        let at = CellRef::new(row, self.next_source_column.min(MAX_COLUMNS - 1));
        let mut references = attributes.iter().filter(|(key, _)| key == "r");
        let first = references.next().map(|(_, value)| value.as_str());
        if references.next().is_some() {
            return Err(self.refusal(at, "expected one cell r attribute, got duplicates"));
        }
        let column = super::parser::cell_coordinate(row, self.next_source_column, first)
            .map_err(|(located, reason)| self.refusal(located.unwrap_or(at), reason))?;
        Ok(CellRef::new(row, column))
    }

    fn next_row(&mut self) -> Result<bool> {
        if self.ended {
            return Ok(false);
        }
        while self
            .batch
            .as_ref()
            .is_none_or(|batch| self.index >= batch.len())
        {
            self.batch = None;
            self.index = 0;
            let remaining =
                (MAX_ROWS - self.first - self.written - u32::from(self.totals.is_some())) as usize;
            let table = self.table.clone();
            let next = self.rows.next_with_preflight(|count| {
                if count > remaining {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("$.table[{table}]"),
                        reason: format_smolstr!(
                            "expected at most {remaining} body rows before the grid ends, got a batch of {count}"
                        ),
                    }.into());
                }
                Ok(())
            });
            match next {
                Some(Ok(batch)) if !batch.is_empty() => self.batch = Some(batch),
                Some(Ok(_)) => continue,
                Some(Err(error)) => return Err(error.into()),
                None => {
                    self.ended = true;
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn cell(
        &mut self,
        offset: usize,
        at: CellRef,
        own: Option<StyleId>,
        needs_namespace: bool,
    ) -> Result<Vec<u8>> {
        let style = match self.formats[offset] {
            Some(format) => Some(
                self.splice
                    .written(own.unwrap_or_default(), format)
                    .map_err(|error| self.refusal(at, super::cell::wire_reason(&error)))?,
            ),
            None => own,
        };
        let batch = self.batch.as_ref().expect("selected table row landed");
        let children = batch.as_struct().expect("record field resolved").children();
        let child = &children[self.columns[offset]];
        self.reference.clear();
        at.write_a1(&mut self.reference);
        let mut cell = Vec::new();
        let namespace = needs_namespace.then_some(self.family.expect("worksheet root"));
        CellWriter {
            system: self.system,
            temporal: self.splice.temporal_styles(),
            namespace,
        }
        .write(
            &mut cell,
            child,
            self.index,
            self.kinds[offset],
            &self.reference,
            style,
        )
        .map_err(|error| self.refusal(at, super::cell::wire_reason(&error)))?;
        if cell.is_empty() {
            write_cell_start(&mut cell, &self.reference, style, namespace, None)?;
            cell.extend_from_slice(b"/>");
        }
        Ok(cell)
    }

    fn moved_total(
        &self,
        offset: usize,
        at: CellRef,
        destination_style: Option<StyleId>,
    ) -> Result<Vec<u8>> {
        let totals = self.totals.as_ref().expect("one totals row");
        if totals.row_scope || totals.unresolved[offset] {
            return Err(self.refusal(at,
                "expected a totals cell without row-scoped or unresolved namespace markup during movement"));
        }
        let default_style = self.default_columns.get(offset).copied().flatten();
        // Native Excel keeps XF0 for an allocated cell without s; only
        // a physically absent cell inherits row/column defaults.
        if totals.cells[offset].is_none()
            && totals.row_style.or(default_style).unwrap_or_default()
                != destination_style.unwrap_or_default()
        {
            return Err(self.refusal(
                at,
                "expected the totals cell's inherited style to be unchanged at its new row",
            ));
        }
        let old = CellRef::new(
            self.old_totals.expect("totals row"),
            self.range.start().column() + offset as u32,
        );
        let Some(raw) = totals.cells[offset].as_ref() else {
            return Err(self.refusal(at, format_smolstr!(
                "expected a physical totals cell at {old} when resizing table {}, got none; table-column label/formula synthesis is not modeled",
                self.table)));
        };
        let mut inspect = TotalsCellInspect {
            at: old,
            sheet: &self.sheet,
            has_formula: false,
            family: self.family.expect("worksheet root"),
            main: Vec::new(),
            closed_main: false,
        };
        super::package::edit_document(raw, &mut inspect)?;
        let mut reference = String::with_capacity(12);
        at.write_a1(&mut reference);
        let mut retag = RetagTotalsCell {
            reference,
            seen: false,
            clear_cached: inspect.has_formula,
            family: self.family.expect("worksheet root"),
        };
        let edited = super::package::edit_document(raw, &mut retag)?;
        if !retag.seen {
            return Err(self.refusal(at, "expected one captured totals cell"));
        }
        Ok(edited.unwrap_or_else(|| raw.clone()))
    }

    fn missing_cells(&mut self, through: usize) -> Result<Vec<u8>> {
        let row = self.current_row.expect("selected row");
        let mut xml = Vec::new();
        while self.next_column < through {
            let offset = self.next_column;
            let at = CellRef::new(row, self.range.start().column() + offset as u32);
            let inherited = self
                .row_style
                .or_else(|| self.default_columns.get(offset).copied().flatten());
            if self.totals_target {
                xml.extend(self.moved_total(offset, at, inherited)?);
            } else {
                xml.extend(self.cell(offset, at, inherited, true)?);
            }
            self.seen[offset] = true;
            self.next_column += 1;
        }
        Ok(xml)
    }

    fn missing_row(&mut self, row: u32) -> Result<Vec<u8>> {
        debug_assert!(
            self.batch
                .as_ref()
                .is_some_and(|batch| self.index < batch.len())
        );
        self.current_row = Some(row);
        self.row_style = None;
        self.next_column = 0;
        self.seen.fill(false);
        let mut xml = Vec::new();
        write!(
            &mut xml,
            "<row r=\"{}\" xmlns=\"{}\">",
            row + 1,
            self.family.expect("worksheet root").namespace()
        )?;
        let width = self.columns.len();
        xml.extend(self.missing_cells(width)?);
        xml.extend_from_slice(b"</row>");
        self.current_row = None;
        self.written += 1;
        self.index += 1;
        Ok(xml)
    }

    fn missing_rows_before(&mut self, row: u32) -> Result<Vec<u8>> {
        let mut xml = Vec::new();
        while self.first + self.written < row && self.next_row()? {
            let next = self.first + self.written;
            xml.extend(self.missing_row(next)?);
        }
        if self.totals.is_some()
            && !self.totals_written
            && self.ended
            && self.first + self.written < row
        {
            let target = self.first + self.written;
            let mut total_row = Vec::new();
            write!(
                &mut total_row,
                "<row r=\"{}\" xmlns=\"{}\">",
                target + 1,
                self.family.expect("worksheet root").namespace()
            )?;
            for offset in 0..self.columns.len() {
                let at = CellRef::new(target, self.range.start().column() + offset as u32);
                let default_style = self.default_columns.get(offset).copied().flatten();
                total_row.extend(self.moved_total(offset, at, default_style)?);
            }
            total_row.extend_from_slice(b"</row>");
            xml.extend(total_row);
            self.totals_written = true;
        }
        Ok(xml)
    }

    fn finish(&mut self) -> Result<u32> {
        if !self.saw_root || !self.saw_data {
            return Err(self.table_refusal("expected a worksheet sheetData for the selected table"));
        }
        if !self.ended {
            return Err(
                self.table_refusal("expected the complete table body stream at sheetData end")
            );
        }
        if self.written == 0 {
            return Err(self.table_refusal("expected at least one table body row, got zero"));
        }
        if self.totals.is_some() && !self.totals_written {
            return Err(
                self.table_refusal("expected the retained totals row after the streamed body")
            );
        }
        Ok(self.first + self.written)
    }
}

impl super::package::Edits for TableBody<'_, '_> {
    fn keep_raw_subtree(
        &mut self,
        path: &[smol_str::SmolStr],
        tag: &quick_xml::events::BytesStart<'_>,
        namespace: &quick_xml::name::ResolveResult<'_>,
        _: usize,
    ) -> Result<Option<Vec<u8>>> {
        if path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && self.current_row.is_some()
            && self.main.last().copied() == Some(true)
            && super::package::local_name(tag.name().as_ref()) == b"c"
            && Self::family_of(namespace) == self.family
        {
            let Some(row) = self.source_row else {
                return Ok(None);
            };
            let mut reference = None;
            for held in tag.attributes().with_checks(false) {
                let Ok(held) = held else { return Ok(None) };
                if held.key.as_namespace_binding().is_some() {
                    return Ok(None);
                }
                let Ok(value) = held.normalized_value(quick_xml::XmlVersion::Implicit1_0) else {
                    return Ok(None);
                };
                if held.key.as_ref() == b"r" {
                    if reference.is_some() {
                        return Ok(None);
                    }
                    reference = Some(value);
                }
            }
            let Ok(column) =
                super::parser::cell_coordinate(row, self.next_source_column, reference.as_deref())
            else {
                return Ok(None);
            };
            if !self.range.contains_column(column) {
                self.next_source_column = column + 1;
                self.source_cell = None;
                let first = self.range.start().column();
                let inserted = if column >= first && (!self.clearing || self.totals_target) {
                    let through = ((column - first) as usize).min(self.columns.len());
                    self.missing_cells(through)?
                } else {
                    Vec::new()
                };
                return Ok(Some(inserted));
            }
        }
        if path.len() != 2
            || path[0] != "worksheet"
            || path[1] != "sheetData"
            || self.main.last().copied() != Some(true)
            || super::package::local_name(tag.name().as_ref()) != b"row"
            || Self::family_of(namespace) != self.family
        {
            return Ok(None);
        }
        let mut row = None;
        for held in tag.attributes().with_checks(false) {
            let Ok(held) = held else { return Ok(None) };
            if held.key.as_namespace_binding().is_some() {
                return Ok(None);
            }
            // Keep the editor's attribute validation without building its
            // per-element Vec for a row whose complete subtree stays raw.
            let Ok(value) = held.normalized_value(quick_xml::XmlVersion::Implicit1_0) else {
                return Ok(None);
            };
            if held.key.as_ref() == b"r" {
                if row.is_some() {
                    return Ok(None);
                }
                row =
                    super::parser::row_coordinate(self.next_source_row, Some(value.as_ref())).ok();
                if row.is_none() {
                    return Ok(None);
                }
            }
        }
        let row = match row {
            Some(row) => row,
            None => match super::parser::row_coordinate(self.next_source_row, None) {
                Ok(row) => row,
                Err(_) => return Ok(None),
            },
        };
        if row >= self.first && (row < self.after || self.next_row()?) {
            return Ok(None);
        }
        if self.totals.is_some()
            && (Some(row) == self.old_totals || self.ended && row == self.first + self.written)
        {
            if Some(row) == self.old_totals && self.ended && row == self.first + self.written {
                self.totals_written = true;
                self.next_source_row = row + 1;
                return Ok(Some(self.missing_rows_before(row)?));
            }
            return Ok(None);
        }
        self.next_source_row = row + 1;
        Ok(Some(self.missing_rows_before(row)?))
    }

    fn before(
        &mut self,
        path: &[smol_str::SmolStr],
        attributes: &[(smol_str::SmolStr, String)],
        namespace: &quick_xml::name::ResolveResult<'_>,
    ) -> Result<Option<Vec<u8>>> {
        let matching = Self::family_of(namespace) == self.family;
        if path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && self.main.last().copied() == Some(true)
            && matching
        {
            let row = self.row_number(attributes)?;
            self.next_source_row = row + 1;
            self.source_row = Some(row);
            self.next_source_column = 0;
            let inserted = self.missing_rows_before(row)?;
            return Ok((!inserted.is_empty()).then_some(inserted));
        }
        if path.len() == 4
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && path[3] == "c"
            && self.current_row.is_some()
            && self.main.last().copied() == Some(true)
            && matching
        {
            let at = self.cell_number(attributes)?;
            self.next_source_column = at.column() + 1;
            self.source_cell = Some(at);
            let first = self.range.start().column();
            if at.column() >= first && (!self.clearing || self.totals_target) {
                let through = ((at.column() - first) as usize).min(self.columns.len());
                let inserted = self.missing_cells(through)?;
                return Ok((!inserted.is_empty()).then_some(inserted));
            }
        }
        if path.len() == 4
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && self.current_row.is_some()
            && self.main.last().copied() == Some(true)
            && (path[3] != "c" || !matching)
        {
            let width = self.columns.len();
            let inserted = if self.clearing && !self.totals_target {
                Vec::new()
            } else {
                self.missing_cells(width)?
            };
            return Ok((!inserted.is_empty()).then_some(inserted));
        }
        Ok(None)
    }

    fn before_end(&mut self, path: &[smol_str::SmolStr]) -> Result<Option<Vec<u8>>> {
        if path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && self.current_row.is_some()
            && self.closed_main
        {
            let width = self.columns.len();
            let inserted = if self.clearing && !self.totals_target {
                Vec::new()
            } else {
                self.missing_cells(width)?
            };
            return Ok((!inserted.is_empty()).then_some(inserted));
        }
        if path.len() == 2
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && self.saw_data
            && self.closed_main
        {
            let inserted = self.missing_rows_before(MAX_ROWS)?;
            let _ = self.next_row()?;
            return Ok((!inserted.is_empty()).then_some(inserted));
        }
        Ok(None)
    }

    fn inside_empty(&mut self, path: &[smol_str::SmolStr]) -> Result<Option<Vec<u8>>> {
        self.before_end(path)
    }

    fn keep_subtree(
        &mut self,
        path: &[smol_str::SmolStr],
        _: &[(smol_str::SmolStr, String)],
        namespace: &quick_xml::name::ResolveResult<'_>,
    ) -> Result<bool> {
        let matching = Self::family_of(namespace) == self.family;
        if path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && self.main.last().copied() == Some(true)
            && matching
        {
            let row = self.source_row.expect("row resolved before the skip hook");
            let outside = row < self.first || row >= self.after && !self.next_row()?;
            if self.totals.is_some()
                && (Some(row) == self.old_totals || self.ended && row == self.first + self.written)
            {
                if Some(row) == self.old_totals && self.ended && row == self.first + self.written {
                    self.totals_written = true;
                    self.source_row = None;
                    return Ok(true);
                }
                return Ok(false);
            }
            if outside {
                self.source_row = None
            }
            return Ok(outside);
        }
        if path.len() == 4
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && path[3] == "c"
            && self.current_row.is_some()
            && matching
        {
            let at = self
                .source_cell
                .take()
                .expect("cell resolved before the skip hook");
            if !self.range.contains_column(at.column()) {
                return Ok(true);
            }
            self.source_cell = Some(at);
        }
        Ok(false)
    }

    fn start(
        &mut self,
        path: &[smol_str::SmolStr],
        attributes: &[(smol_str::SmolStr, String)],
        namespace: quick_xml::name::ResolveResult<'_>,
    ) -> Result<super::package::Tag> {
        let family = Self::family_of(&namespace);
        if path.len() == 1 {
            if path[0] != "worksheet" || family.is_none() || self.saw_root {
                return Err(self.table_refusal("expected one SpreadsheetML worksheet root"));
            }
            self.family = family;
            self.saw_root = true;
            self.main.push(true);
            return Ok(super::package::Tag::Keep);
        }
        let main = self.main.last().copied().unwrap_or(false) && family == self.family;
        self.main.push(main);
        if main
            && path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "mergeCells"
            && path[2] == "mergeCell"
        {
            if !self.saw_data {
                return Err(self.table_refusal("expected mergeCells after sheetData"));
            }
            let reference = Self::attribute(attributes, "ref")
                .ok_or_else(|| self.table_refusal("expected mergeCell ref"))?;
            let merged: super::cell::CellRange = reference
                .parse()
                .map_err(|_| self.table_refusal("expected a valid mergeCell ref"))?;
            let new_after = self.first + self.written;
            let totals = u32::from(self.totals.is_some());
            let changed = if new_after > self.after {
                Some(super::cell::CellRange::new(
                    CellRef::new(self.after, self.range.start().column()),
                    CellRef::new(new_after + totals - 1, self.range.end().column()),
                ))
            } else if new_after < self.after {
                Some(super::cell::CellRange::new(
                    CellRef::new(new_after, self.range.start().column()),
                    CellRef::new(self.after + totals - 1, self.range.end().column()),
                ))
            } else {
                None
            };
            if changed.is_some_and(|changed| changed.intersects(merged)) {
                return Err(self.table_refusal(format_smolstr!(
                    "expected table resize outside merged range {merged}"
                )));
            }
        }
        if path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "cols"
            && path[2] == "col"
            && main
        {
            if self.saw_data {
                return Err(self.table_refusal("expected column defaults before sheetData"));
            }
            let (span, format) = super::layout::Columns::read_span(
                attributes.iter().map(|(key, value)| {
                    Ok((key.as_bytes(), std::borrow::Cow::Borrowed(value.as_str())))
                }),
                self.next_style_column,
            )?;
            self.next_style_column = span.end;
            if let Some(style) = format.style {
                let first = span.start.max(self.range.start().column());
                let after = span.end.min(self.range.end().column() + 1);
                if first < after {
                    self.default_columns.resize(self.columns.len(), None);
                    let offset = self.range.start().column();
                    self.default_columns[(first - offset) as usize..(after - offset) as usize]
                        .fill(Some(style));
                }
            }
        } else if path.len() == 2 && path[0] == "worksheet" && path[1] == "sheetData" && main {
            if self.saw_data {
                return Err(self.table_refusal("expected one sheetData element"));
            }
            self.saw_data = true;
        } else if path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && main
        {
            let row = self.source_row.expect("row resolved before its start hook");
            if row >= self.first
                && (row < self.after
                    || self.next_row()?
                    || self.totals.is_some() && Some(row) == self.old_totals
                    || self.totals.is_some() && self.ended && row == self.first + self.written)
            {
                let available = self.next_row()?;
                let expected = self.first + self.written;
                if available && row != expected {
                    return Err(self.refusal(
                        CellRef::new(expected, self.range.start().column()),
                        format_smolstr!(
                            "expected table body row {}, got {}",
                            expected + 1,
                            row + 1
                        ),
                    ));
                }
                self.clearing = !available;
                self.totals_target =
                    !available && self.totals.is_some() && row == self.first + self.written;
                let mut format = super::layout::RowFormat::default();
                for (key, value) in attributes {
                    format
                        .read_style_attribute(key.as_bytes(), value)
                        .map_err(|error| {
                            self.refusal(
                                CellRef::new(row, self.range.start().column()),
                                super::cell::wire_reason(&error),
                            )
                        })?;
                }
                self.row_style = format.applied_style();
                if self.totals_target
                    && attributes.iter().any(|(key, _)| {
                        key == "xml:space" || key == "xmlns" || key.starts_with("xmlns:")
                    })
                {
                    return Err(self.refusal(CellRef::new(row, self.range.start().column()),
                        "expected a totals destination row without row-scoped namespace or xml:space"));
                }
                self.current_row = Some(row);
                self.next_column = 0;
                self.seen.fill(false);
            }
        } else if path.len() == 4
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && path[3] == "c"
            && self.current_row.is_some()
            && main
        {
            let at = self
                .source_cell
                .expect("cell resolved before its start hook");
            if self.range.contains_column(at.column()) {
                let offset = (at.column() - self.range.start().column()) as usize;
                if self.seen[offset] || offset != self.next_column {
                    return Err(self.refusal(at, "expected one cell at this table coordinate"));
                }
                for name in ["r", "s", "t"] {
                    if attributes.iter().filter(|(key, _)| key == name).count() > 1 {
                        return Err(self.refusal(
                            at,
                            format_smolstr!("expected one {name} attribute, got duplicates"),
                        ));
                    }
                }
                for (key, _) in attributes {
                    if !matches!(key.as_str(), "r" | "s" | "t")
                        && key != "xmlns"
                        && !key.starts_with("xmlns:")
                    {
                        return Err(self.refusal(
                            at,
                            format_smolstr!("expected a plain table cell, got attribute {key}"),
                        ));
                    }
                }
                let style = Self::attribute(attributes, "s")
                    .map(StyleId::from_attribute)
                    .transpose()
                    .map_err(|error| self.refusal(at, super::cell::wire_reason(&error)))?;
                self.seen[offset] = true;
                self.current_cell = Some((offset, at, style));
                self.extension_cell = at.row() >= self.after && Some(at.row()) != self.old_totals;
            }
        } else if let Some((_, at, _)) = self.current_cell {
            if self.extension_cell
                && path.len() == 5
                && matches!(path[4].as_str(), "v" | "is" | "f")
            {
                return Err(self.refusal(
                    at,
                    "expected an unoccupied cell below the table for body growth",
                ));
            }
            let accepted = main
                && (path.len() == 5 && matches!(path[4].as_str(), "v" | "is" | "f")
                    || path.len() >= 6 && path[4] == "is");
            if !accepted {
                return Err(self.refusal(
                    at,
                    format_smolstr!("expected a plain value cell, got {path:?}"),
                ));
            }
            if path.len() == 5
                && path[4] == "f"
                && attributes.iter().any(|(name, value)| {
                    (name == "t" && matches!(value.as_str(), "shared" | "array" | "dataTable"))
                        || matches!(name.as_str(), "si" | "ref")
                })
            {
                return Err(self.refusal(
                    at,
                    "expected a standalone formula to overwrite, got a grouped formula",
                ));
            }
        }
        Ok(super::package::Tag::Keep)
    }

    fn end(
        &mut self,
        path: &[smol_str::SmolStr],
        _: usize,
        _: usize,
        _: &[(smol_str::SmolStr, String)],
    ) -> super::package::Tag {
        self.closed_main = self.main.pop().unwrap_or(false);
        // The selected body may end at a different row after the stream is
        // consumed. A dimension before sheetData cannot be restated then;
        // omitting it is valid, as the ordinary streaming writer already does.
        if self.closed_main && path.len() == 2 && path[0] == "worksheet" && path[1] == "dimension" {
            return super::package::Tag::Drop;
        }
        if self.closed_main && path.len() == 4 && path[3] == "c" {
            self.source_cell = None;
        }
        if self.closed_main && path.len() == 3 && path[2] == "row" {
            self.source_row = None;
        }
        super::package::Tag::Keep
    }

    fn replacement(
        &mut self,
        path: &[smol_str::SmolStr],
        attributes: &[(smol_str::SmolStr, String)],
        qualified: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        if path.len() == 4
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && path[3] == "c"
            && self.closed_main
        {
            let Some((offset, at, style)) = self.current_cell.take() else {
                return Ok(None);
            };
            let needs_namespace =
                qualified.contains(&b':') || attributes.iter().any(|(name, _)| name == "xmlns");
            let cell = if self.totals_target {
                self.moved_total(
                    offset,
                    at,
                    self.row_style
                        .or_else(|| self.default_columns.get(offset).copied().flatten()),
                )?
            } else if self.clearing {
                let mut blank = Vec::new();
                if style.is_some() {
                    self.reference.clear();
                    at.write_a1(&mut self.reference);
                    let namespace = needs_namespace.then_some(self.family.expect("worksheet root"));
                    write_cell_start(&mut blank, &self.reference, style, namespace, None)?;
                    blank.extend_from_slice(b"/>");
                }
                blank
            } else {
                self.cell(offset, at, style, needs_namespace)?
            };
            self.next_column += 1;
            return Ok(Some(cell));
        }
        if path.len() == 3
            && path[0] == "worksheet"
            && path[1] == "sheetData"
            && path[2] == "row"
            && self.closed_main
            && let Some(row) = self.current_row.take()
        {
            if self.totals_target {
                self.totals_written = true;
                self.totals_target = false;
            } else if !self.clearing {
                if let Some(offset) = self.seen.iter().position(|seen| !seen) {
                    return Err(self.refusal(
                        CellRef::new(row, self.range.start().column() + offset as u32),
                        "expected a present table body cell to overwrite",
                    ));
                }
                self.written += 1;
                self.index += 1;
            }
            self.clearing = false;
        }
        Ok(None)
    }
}
