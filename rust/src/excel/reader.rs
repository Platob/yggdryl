//! The record path over one worksheet: rows streamed off the part as they
//! are parsed, each cell landing under the column its header pairs it with.
//!
//! Nothing here builds a [`Cell`](super::Cell) or holds a sheet. A read
//! resolves its pairing once - the header row names the sheet's columns, a
//! declared field's children are matched to them by name (by position
//! without a header), a required child the sheet lacks is refused before the
//! first batch - and then every `<row>` is one positional row `Scalar`
//! handed to the field's contract and widened into batches by the one
//! row-to-batch reader every native record source goes through.
//!
//! Without a declared field the schema is learned by one tally pass over the
//! same parser: per column the header text, whether any value is present,
//! and the one datatype the first present cell's facts prove; a later cell of
//! another datatype is refused naming the sheet and the cell, and the caller
//! declares a field. The read then streams under the inferred field, which
//! costs the part a second time and no `Scalar` in between.

use std::collections::BTreeMap;
use std::io::{BufReader, Read};
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::arrow::BatchReader;
use crate::media::DEFAULT_ROOT_NAME;
use crate::media::IORecordOptions;
use crate::{DataType, Error, Field, Result, Scalar, StructType};

use super::cell::{CellKind, CellRange, CellRef, DateSystem};
use super::options::ExcelOptions;
use super::parser::{RawCell, RawRow, SheetRows};
use super::records::{
    FlatBinding, FlatExtent, Header, HeaderProbe, RowLabel, RowsLayout, RowsWindow,
};
use super::shared_strings::SharedStrings;
use super::styles::StyleSheet;
use crate::RecordHeader;

/// What a record read needs to know about the worksheet it streams.
#[derive(Clone)]
pub(crate) struct Source {
    /// The sheet's name, which the stream is opened by and every refusal
    /// names as Excel does: `Trades!B3`.
    pub(crate) sheet: SmolStr,
    pub(crate) system: DateSystem,
    pub(crate) strings: Arc<SharedStrings>,
    pub(crate) styles: Arc<StyleSheet>,
    /// The cells a read addresses; the whole grid when the options name none.
    pub(crate) range: CellRange,
    /// Whether the range's first row names the columns.
    pub(crate) header: RecordHeader,
    pub(crate) explicit_range: bool,
    /// Authoritative tableColumn names, which consume no physical row here.
    pub(crate) column_names: Option<Arc<[SmolStr]>>,
    /// A table's exact body length, including absent physical rows; None
    /// retains sparse worksheet iteration. Some(0) is a table with no body.
    pub(crate) row_count: Option<u32>,
}

impl Source {
    /// The text of one cell, a shared string resolved.
    fn content<'a>(
        &'a self,
        cell: &'a RawCell,
        reference: CellRef,
    ) -> Result<std::borrow::Cow<'a, str>> {
        cell.resolved_content(&self.strings, &self.sheet, reference)
            .map(std::borrow::Cow::Borrowed)
    }

    fn refuse(&self, reference: CellRef, reason: SmolStr) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("{}!{reference}", self.sheet),
            reason,
        }
    }

    /// Whether a cell states any value at all.
    fn is_present(cell: &RawCell) -> bool {
        cell.has_content && (cell.kind.is_text() || !cell.content.trim().is_empty())
    }
}

/// Decode one raw header row; records owns labels and coordinates.
fn resolve_header(source: &Source, row: Option<&RawRow>) -> Result<Header> {
    let cells = row
        .into_iter()
        .flat_map(|row| row.cells.iter().map(move |cell| (row.index, cell)))
        .filter(|(_, cell)| source.range.contains_column(cell.column))
        .map(|(row, cell)| {
            let label = if matches!(source.header, RecordHeader::Source) {
                Some(std::borrow::Cow::Owned(header_text(
                    source,
                    cell,
                    CellRef::new(row, cell.column),
                )?))
            } else {
                None
            };
            Ok((cell.column, label))
        });
    let extent = if source.row_count.is_some() {
        FlatExtent::Table(source.column_names.as_deref())
    } else {
        FlatExtent::Sparse
    };
    Header::resolve(
        source.range,
        extent,
        matches!(source.header, RecordHeader::Source),
        cells,
    )
}

/// The text a header cell names its column with: its displayed text.
fn header_text(source: &Source, cell: &RawCell, reference: CellRef) -> Result<String> {
    if !Source::is_present(cell) || cell.kind == CellKind::Error {
        return Ok(String::new());
    }
    let content = source.content(cell, reference)?;
    if cell.kind.is_text() {
        return Ok(content.into_owned());
    }
    let value = super::cell::wire_scalar(
        cell.kind,
        source.styles.number_format(cell.style),
        source.system,
        &content,
    )
    .and_then(super::cell::WireScalar::into_scalar)
    .map_err(|error| source.refuse(reference, super::cell::wire_reason(&error)))?;
    Ok(super::cell::cell_text(&value).to_string())
}

/// The datatype one cell's facts prove, for the tally.
fn wire_dtype(source: &Source, cell: &RawCell, reference: CellRef) -> Result<Option<DataType>> {
    if !Source::is_present(cell) || cell.kind == CellKind::Error {
        return Ok(None);
    }
    Ok(Some(match cell.kind {
        CellKind::SharedString | CellKind::FormulaString | CellKind::InlineString => {
            DataType::utf8()
        }
        CellKind::Boolean => DataType::Boolean,
        CellKind::Number | CellKind::Date => {
            let content = source.content(cell, reference)?;
            let value = super::cell::wire_scalar(
                cell.kind,
                source.styles.number_format(cell.style),
                source.system,
                &content,
            )
            .and_then(super::cell::WireScalar::into_scalar)
            .map_err(|error| source.refuse(reference, super::cell::wire_reason(&error)))?;
            value
                .dtype()
                .map_err(|error| source.refuse(reference, super::cell::wire_reason(&error)))?
        }
        CellKind::Error => unreachable!("an error cell is absent"),
    }))
}

/// Learn the record field of a sheet by one pass over its rows.
///
/// # Errors
///
/// Returns the parser's refusal, or a refusal naming the first cell whose
/// datatype disagrees with its column's, or a header naming one column twice.
pub(crate) fn infer_field<R: Read>(
    source: &Source,
    rows: SheetRows<BufReader<R>>,
    name: &str,
) -> Result<Field> {
    Ok(probe_header(source, rows, name, None, false)?.field)
}

/// The wire adapter lends typed facts to the shared flat header owner.
/// A named table's exact body count supplies missing physical rows, while
/// its metadata names remain authoritative and consume no header row.
pub(super) fn probe_header<R: Read>(
    source: &Source,
    rows: SheetRows<BufReader<R>>,
    name: &str,
    declared: Option<&Field>,
    safe: bool,
) -> Result<super::records::HeaderResolution> {
    let limit = if source.row_count == Some(0) {
        0
    } else {
        usize::MAX
    };
    let mut probe = HeaderProbe::new(
        source.sheet.clone(),
        source.range,
        source.header,
        source.row_count.map(u64::from),
    );
    let mut first_row = None;
    for row in rows
        .take(limit)
        .take_while(|row| {
            row.as_ref().map_or(true, |row| {
                source.row_count.is_none() || row.index <= source.range.end().row()
            })
        })
        .filter(|row| {
            row.as_ref()
                .map_or(true, |row| source.range.contains_row(row.index))
        })
    {
        let row = row?;
        let first = probe.row(row.index);
        for cell in &row.cells {
            if !source.range.contains_column(cell.column) {
                continue;
            }
            let at = CellRef::new(row.index, cell.column);
            let label =
                if first && matches!(source.header, RecordHeader::Source | RecordHeader::Infer) {
                    let text = if source.header == RecordHeader::Infer && !cell.kind.is_text() {
                        SmolStr::new_static("")
                    } else {
                        SmolStr::new(header_text(source, cell, at)?)
                    };
                    Some((text, cell.kind.is_text() && Source::is_present(cell)))
                } else {
                    None
                };
            let dtype = if first && source.header == RecordHeader::Source {
                None
            } else {
                wire_dtype(source, cell, at)?
            };
            probe.cell(at, dtype, label)?;
        }
        if first && declared.is_some() {
            first_row = Some(row);
        }
    }
    let extent = if source.row_count.is_some() {
        FlatExtent::Table(source.column_names.as_deref())
    } else {
        FlatExtent::Sparse
    };
    let first_row_as_data = declared.and_then(|field| {
        let pairing = probe.first_row_pairing(field)?;
        Some(first_row.as_ref().is_some_and(|row| {
            field
                .fields()
                .iter()
                .zip(&pairing)
                .all(|(child, column)| read_leaf(source, safe, row, child, *column).is_ok())
        }))
    });
    probe.resolve(name, extent, declared, first_row_as_data)
}
/// Count the records of a sheet: every `<row>` inside the range, less the
/// header.
///
/// # Errors
///
/// Returns the parser's refusal.
pub(crate) fn row_count<R: Read>(source: &Source, rows: SheetRows<BufReader<R>>) -> Result<u64> {
    if let Some(count) = source.row_count {
        return Ok(u64::from(count));
    }
    let mut count = 0_u64;
    let mut anchor = source.explicit_range.then_some(source.range.start().row());
    for row in rows {
        let row = row?;
        if !source.range.contains_row(row.index) {
            continue;
        }
        if anchor.is_none() && !row.cells.is_empty() {
            anchor = Some(row.index);
        }
        match source.header {
            RecordHeader::Source | RecordHeader::None => count += 1,
            RecordHeader::Infer => return Err(super::options::ExcelOptions::ambiguous_error()),
            RecordHeader::Rows(levels) => {
                if let Some(first) = anchor {
                    if row.index >= first.saturating_add(levels) {
                        count += 1;
                    }
                }
            }
        }
    }
    Ok(if matches!(source.header, RecordHeader::Source) {
        count.saturating_sub(1)
    } else {
        count
    })
}

/// The rows of a sheet under a field, one positional `Scalar` per `<row>`.
struct RowStream<R: Read> {
    source: Source,
    rows: std::iter::Peekable<SheetRows<BufReader<R>>>,
    root: Arc<Field>,
    /// Per field child, the sheet column it reads, `None` for a column the
    /// sheet lacks.
    pairing: Vec<Option<u32>>,
    layout: Option<RowsLayout>,
    merges: Vec<CellRange>,
    /// The cells of the row being read, one buffer drained into every run.
    values: Vec<Scalar>,
    safe: bool,
    /// Whether the header row has been consumed.
    started: bool,
    /// Whether the sheet holds no row at all, so nothing is paired or read.
    empty: bool,
    failed: bool,
    /// Dense table rows emitted so far; no cells are retained for an absent row.
    emitted_rows: u32,
}

impl<R: Read> RowStream<R> {
    /// Pair the field's children with the sheet's columns, reading the
    /// header row when there is one.
    fn start(&mut self) -> Result<()> {
        if let RecordHeader::Rows(levels) = self.source.header {
            return start_rows(self, levels);
        }
        self.started = true;
        let header = if matches!(self.source.header, RecordHeader::Source) {
            loop {
                match self.rows.peek() {
                    Some(Ok(row)) if !self.source.range.contains_row(row.index) => {
                        self.rows.next();
                    }
                    Some(Ok(_)) => break self.rows.next().transpose()?,
                    Some(Err(_)) => break self.rows.next().transpose()?,
                    None => break None,
                }
            }
        } else {
            None
        };
        // A sheet with no row states no shape: zero rows under the field,
        // whatever the field requires.
        if matches!(self.source.header, RecordHeader::Source) && header.is_none() {
            self.empty = true;
            return Ok(());
        }
        let header = resolve_header(&self.source, header.as_ref())?;
        self.pairing = header.pairing(
            &self.root,
            FlatBinding {
                sheet: &self.source.sheet,
                range: self.source.range,
                by_name: matches!(self.source.header, RecordHeader::Source)
                    || self.source.column_names.is_some(),
                authoritative: self.source.column_names.is_some(),
            },
        )?;
        self.empty = self.source.row_count == Some(0);
        Ok(())
    }

    /// The next record: one positional row under the field.
    fn read(&mut self) -> Result<Option<Scalar>> {
        if !self.started {
            self.start()?;
        }
        if self.empty {
            return Ok(None);
        }
        let row = if let Some(count) = self.source.row_count {
            if self.emitted_rows == count {
                return Ok(None);
            }
            // Named-table intake proved that the whole extent is in the grid.
            let index = self.source.range.start().row() + self.emitted_rows;
            self.emitted_rows += 1;
            loop {
                match self.rows.peek() {
                    Some(Ok(row)) if row.index < index => {
                        self.rows.next();
                    }
                    Some(Ok(row)) if row.index == index => {
                        break self.rows.next().expect("peeked row")?;
                    }
                    Some(Err(_)) => {
                        return Err(self.rows.next().expect("peeked refusal").unwrap_err());
                    }
                    _ => {
                        break RawRow {
                            index,
                            cells: Vec::new(),
                            format: None,
                        };
                    }
                }
            }
        } else {
            loop {
                match self.rows.next() {
                    Some(row) => {
                        let row = row?;
                        if self.source.range.contains_row(row.index)
                            && self
                                .layout
                                .as_ref()
                                .is_none_or(|layout| row.index >= layout.body_start())
                        {
                            break row;
                        }
                    }
                    None => return Ok(None),
                }
            }
        };
        let mut values = std::mem::take(&mut self.values);
        values.clear();
        let result = if let Some(layout) = &self.layout {
            for leaf in layout.leaves() {
                values.push(read_leaf(
                    &self.source,
                    self.safe,
                    &row,
                    &leaf.field,
                    leaf.column,
                )?);
            }
            layout.assemble(&mut values, &self.source.sheet, row.index)?
        } else {
            for (child, column) in self.root.fields().iter().zip(&self.pairing) {
                values.push(read_leaf(&self.source, self.safe, &row, child, *column)?);
            }
            Scalar::from_sequence(values.drain(..))
        };
        self.values = values;
        Ok(Some(result))
    }
}

impl<R: Read> Iterator for RowStream<R> {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.read() {
            Ok(row) => row.map(Ok),
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }
}

/// Stream the records of a sheet under `root` as batches.
///
/// `member` is the part's decoded stream; `root` the field the rows land
/// under, declared or inferred; the batch bounds are the options'.
///
/// # Errors
///
/// Returns the pairing's refusal or the row widening's.
pub(super) fn batch_reader(
    source: Source,
    member: Box<dyn Read + Send>,
    root: &Field,
    layout: Option<RowsLayout>,
    merges: Vec<CellRange>,
    options: &ExcelOptions,
) -> crate::arrow::Result<BatchReader> {
    let rows = SheetRows::new(
        BufReader::with_capacity(crate::DEFAULT_FETCH_BYTE_SIZE, member),
        source.sheet.clone(),
    );
    let stream = RowStream {
        source,
        rows: rows.peekable(),
        root: Arc::new(root.clone().with_nullable(false)),
        pairing: Vec::new(),
        layout,
        merges,
        values: Vec::new(),
        safe: options.safe(),
        started: false,
        empty: false,
        failed: false,
        emitted_rows: 0,
    };
    crate::arrow::rows::result_reader(
        root,
        stream,
        options.batch_row_size(),
        options.batch_byte_size(),
        None,
    )
}

/// The empty record field a sheet with no cell states.
pub(crate) fn empty_root(name: &str) -> Result<Field> {
    Ok(Field::new(
        if name.is_empty() {
            DEFAULT_ROOT_NAME
        } else {
            name
        },
        DataType::from(StructType::from_fields(std::iter::empty::<Field>())?),
        false,
    ))
}

/// Raw cells are decoded once at the wire edge; the owner receives literal
/// labels. Source's trimming stays in records::Header, Rows preserves text.
fn row_labels(source: &Source, row: &RawRow, into: &mut Vec<RowLabel>) -> Result<()> {
    for cell in &row.cells {
        if !source.range.contains_column(cell.column) {
            continue;
        }
        let at = CellRef::new(row.index, cell.column);
        into.push(RowLabel {
            at,
            text: SmolStr::new(header_text(source, cell, at)?),
        });
    }
    Ok(())
}

/// A declared Field may cast heterogeneous body values. Infer still needs
/// positive header evidence before a Rows layout can consume physical rows.
/// The metadata pass already validated the worksheet envelope and supplied
/// merge spans. This pass stops at the first typed body fact; the subsequent
/// row stream validates every selected value under the declared Field.
pub(super) fn inferred_rows_evidence<R: Read>(
    source: &Source,
    rows: SheetRows<BufReader<R>>,
    levels: u32,
) -> Result<()> {
    let header = RowsWindow::header_range(source.range, levels)?;
    let mut body_rows = 0_u64;
    for row in rows {
        let row = row?;
        if !source.range.contains_row(row.index) {
            continue;
        }
        if row.index <= header.end().row() {
            for cell in &row.cells {
                if !source.range.contains_column(cell.column) {
                    continue;
                }
                let at = CellRef::new(row.index, cell.column);
                RowsWindow::require_inferred_label(
                    &source.sheet,
                    at,
                    Source::is_present(cell),
                    cell.kind.is_text(),
                )?;
            }
        } else {
            body_rows += 1;
            for cell in &row.cells {
                if !source.range.contains_column(cell.column) {
                    continue;
                }
                let at = CellRef::new(row.index, cell.column);
                if wire_dtype(source, cell, at)?.is_some_and(|dtype| dtype != DataType::utf8()) {
                    return Ok(());
                }
            }
        }
    }
    RowsWindow::require_inferred_body(body_rows, false)
}

/// Inferred Rows(n): one complete worksheet parse both tallies body cells
/// and captures the trailing mergeCells. The second parse streams data.
pub(super) fn infer_rows_layout<R: Read>(
    source: &Source,
    mut rows: SheetRows<BufReader<R>>,
    name: &str,
    levels: u32,
    known: Option<(CellRange, Vec<CellRange>)>,
) -> Result<RowsLayout> {
    let infer_text_labels = known.is_some();
    let mut labels = Vec::new();
    // (first datatype, number of stored body rows with a present value).
    let mut observations = BTreeMap::<u32, (DataType, u64)>::new();
    let mut body_rows = 0_u64;
    let mut anchor = source.explicit_range.then_some(source.range.start().row());
    for row in &mut rows {
        let row = row?;
        if !source.range.contains_row(row.index) {
            continue;
        }
        if anchor.is_none() && !row.cells.is_empty() {
            anchor = Some(row.index);
        }
        let Some(first) = anchor else { continue };
        let end = first
            .checked_add(levels.checked_sub(1).ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: SmolStr::new_static("expected a positive Rows depth, got zero"),
            })?)
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: format_smolstr!("expected header rows inside the Excel grid, got {levels}"),
            })?;
        if row.index <= end {
            if infer_text_labels {
                for cell in &row.cells {
                    if !source.range.contains_column(cell.column) {
                        continue;
                    }
                    let at = CellRef::new(row.index, cell.column);
                    RowsWindow::require_inferred_label(
                        &source.sheet,
                        at,
                        Source::is_present(cell),
                        cell.kind.is_text(),
                    )?;
                }
            }
            row_labels(source, &row, &mut labels)?;
            continue;
        }
        body_rows += 1;
        for cell in &row.cells {
            if !source.range.contains_column(cell.column) {
                continue;
            }
            let at = CellRef::new(row.index, cell.column);
            let Some(dtype) = wire_dtype(source, cell, at)? else {
                continue;
            };
            match observations.entry(cell.column) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert((dtype, 1));
                }
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    let (first, seen) = slot.get_mut();
                    if *first != dtype {
                        return Err(source.refuse(at, format_smolstr!(
                            "expected {first} like the column's first value, got {dtype}; declare a field to read the column as one datatype")));
                    }
                    *seen += 1;
                }
            }
        }
    }
    if infer_text_labels {
        RowsWindow::require_inferred_body(
            body_rows,
            observations
                .values()
                .any(|(dtype, _)| *dtype != DataType::utf8()),
        )?;
    }
    let (dimension, merges) = match known {
        Some((extent, merges)) => (Some(extent), merges),
        None => rows.into_header_geometry()?,
    };
    let range = if source.explicit_range {
        source.range
    } else {
        RowsWindow::implicit_range(
            dimension.ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: SmolStr::new_static("expected selected header cells, got an empty sheet"),
            })?,
            levels,
            merges.iter().copied(),
        )?
    };
    let stats = observations
        .iter()
        .map(|(column, (dtype, seen))| (*column, (dtype.clone(), *seen != body_rows)))
        .collect::<BTreeMap<_, _>>();
    let columns = if source.explicit_range {
        (range.start().column()..=range.end().column()).collect()
    } else {
        observations.keys().copied().collect()
    };
    RowsLayout::inferred(
        RowsWindow {
            sheet: source.sheet.clone(),
            range,
            levels,
            columns,
            labels,
            merges,
        },
        name,
        &stats,
    )
}

/// Called only on a declared read. This metadata-only pass produces no
/// RawRow/RawCell. RowStream's later pass decodes the header labels and body.
pub(super) fn observed_merges<R: Read>(
    source: &Source,
    member: R,
    levels: u32,
) -> Result<(CellRange, Vec<CellRange>)> {
    let input = BufReader::with_capacity(crate::DEFAULT_FETCH_BYTE_SIZE, member);
    let mut observer = if source.explicit_range {
        let header = RowsWindow::header_range(source.range, levels)?;
        SheetRows::observing_header_merges(input, source.sheet.clone(), header)
    } else {
        SheetRows::observing_header_geometry(input, source.sheet.clone(), levels)
    };
    for row in &mut observer {
        row?; // metadata mode yields no rows
    }
    let (dimension, merges) = observer.into_header_geometry()?;
    let range = if source.explicit_range {
        source.range
    } else {
        RowsWindow::implicit_range(
            dimension.ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: SmolStr::new_static("expected selected header cells, got an empty sheet"),
            })?,
            levels,
            merges.iter().copied(),
        )?
    };
    RowsWindow::header_range(range, levels)?;
    Ok((range, merges))
}

fn start_rows<R: Read>(stream: &mut RowStream<R>, levels: u32) -> Result<()> {
    if stream.layout.is_some() {
        stream.started = true;
        return Ok(());
    }
    let header = RowsWindow::header_range(stream.source.range, levels)?;
    let mut labels = Vec::new();
    while let Some(next) = stream.rows.peek() {
        match next {
            Ok(row) if row.index > header.end().row() => break,
            _ => {
                let row = stream.rows.next().expect("peeked row")?;
                if stream.source.range.contains_row(row.index) {
                    row_labels(&stream.source, &row, &mut labels)?;
                }
            }
        }
    }
    let columns = if stream.source.explicit_range {
        (stream.source.range.start().column()..=stream.source.range.end().column()).collect()
    } else {
        Vec::new() // RowsWindow adds nonblank labels and merge columns.
    };
    let window = RowsWindow {
        sheet: stream.source.sheet.clone(),
        range: stream.source.range,
        levels,
        columns,
        labels,
        merges: std::mem::take(&mut stream.merges),
    };
    stream.layout = Some(RowsLayout::declared(window, stream.root.as_ref())?);
    stream.started = true;
    Ok(())
}

fn read_leaf(
    source: &Source,
    safe: bool,
    row: &RawRow,
    child: &Field,
    column: Option<u32>,
) -> Result<Scalar> {
    let cell = column.and_then(|column| {
        row.cells
            .binary_search_by_key(&column, |cell| cell.column)
            .ok()
            .map(|at| &row.cells[at])
    });
    match cell {
        Some(cell) if Source::is_present(cell) => {
            let at = CellRef::new(row.index, cell.column);
            let content = source.content(cell, at)?;
            let format = source.styles.number_format(cell.style);
            match super::cell::field_scalar(child, cell.kind, format, source.system, &content) {
                Ok(value) => Ok(value),
                Err(_) if safe && child.is_nullable() => Ok(Scalar::Null),
                Err(error) => Err(source.refuse(at, super::cell::wire_reason(&error))),
            }
        }
        _ if !child.is_nullable() => {
            let at = CellRef::new(row.index, column.unwrap_or(0));
            Err(source.refuse(
                at,
                format_smolstr!(
                    "expected a value for the required column {}, got no cell",
                    child.name()
                ),
            ))
        }
        _ => Ok(Scalar::Null),
    }
}
