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

use std::io::{BufReader, Read};
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::arrow::BatchReader;
use crate::media::DEFAULT_ROOT_NAME;
use crate::{DataType, Error, Field, Result, Scalar, StructType};

use super::cell::{CellKind, CellRange, CellRef, DateSystem};
use super::parser::{RawCell, RawRow, SheetRows};
use super::shared_strings::SharedStrings;
use super::styles::Styles;

/// What a record read needs to know about the worksheet it streams.
#[derive(Clone)]
pub(crate) struct Source {
    /// The sheet's name, which the stream is opened by and every refusal
    /// names as Excel does: `Trades!B3`.
    pub(crate) sheet: SmolStr,
    pub(crate) system: DateSystem,
    pub(crate) strings: Arc<SharedStrings>,
    pub(crate) styles: Arc<Styles>,
    /// The cells a read addresses; the whole grid when the options name none.
    pub(crate) range: CellRange,
    /// Whether the range's first row names the columns.
    pub(crate) header: bool,
}

impl Source {
    /// The text of one cell, a shared string resolved.
    fn content<'a>(
        &'a self,
        cell: &'a RawCell,
        reference: CellRef,
    ) -> Result<std::borrow::Cow<'a, str>> {
        if cell.kind == CellKind::SharedString && cell.has_content {
            let index: usize = cell.content.trim().parse().map_err(|_| {
                self.refuse(
                    reference,
                    format_smolstr!("expected a shared string index, got {:?}", cell.content),
                )
            })?;
            return self
                .strings
                .get(index)
                .map(|text| std::borrow::Cow::Borrowed(text.as_str()))
                .ok_or_else(|| {
                    self.refuse(
                        reference,
                        format_smolstr!(
                            "expected a shared string index below {}, got {index}",
                            self.strings.len()
                        ),
                    )
                });
        }
        Ok(std::borrow::Cow::Borrowed(cell.content.as_str()))
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

/// The header of a read: the sheet columns inside the range, named.
struct Header {
    /// The zero-based column of each named sheet column.
    columns: Vec<u32>,
    names: Vec<SmolStr>,
    /// Whether each name came from a header cell rather than its letters.
    from_header: Vec<bool>,
}

impl Header {
    /// Name the columns from the header row, or by their letters from the
    /// first row present.
    fn resolve(source: &Source, row: Option<&RawRow>, header: bool) -> Result<Self> {
        let mut columns = Vec::new();
        let mut names = Vec::new();
        let mut from_header = Vec::new();
        if let Some(row) = row {
            if header {
                for cell in &row.cells {
                    if !source.range.contains_column(cell.column) {
                        continue;
                    }
                    let reference = CellRef::new(row.index, cell.column);
                    let text = header_text(source, cell, reference)?;
                    columns.push(cell.column);
                    from_header.push(!text.is_empty());
                    names.push(if text.is_empty() {
                        CellRef::column_name(cell.column)
                    } else {
                        SmolStr::new(text)
                    });
                }
            } else {
                for cell in &row.cells {
                    if !source.range.contains_column(cell.column) {
                        continue;
                    }
                    columns.push(cell.column);
                    names.push(CellRef::column_name(cell.column));
                    from_header.push(false);
                }
            }
        }
        Ok(Self {
            columns,
            names,
            from_header,
        })
    }

    /// Add a column the header did not name, discovered in a later row.
    fn admit(&mut self, column: u32) -> usize {
        match self.columns.iter().position(|held| *held >= column) {
            Some(at) if self.columns[at] == column => at,
            Some(at) => {
                self.columns.insert(at, column);
                self.names.insert(at, CellRef::column_name(column));
                self.from_header.insert(at, false);
                at
            }
            None => {
                self.columns.push(column);
                self.names.push(CellRef::column_name(column));
                self.from_header.push(false);
                self.columns.len() - 1
            }
        }
    }
}

/// The text a header cell names its column with: its displayed text.
fn header_text(source: &Source, cell: &RawCell, reference: CellRef) -> Result<String> {
    if !Source::is_present(cell) || cell.kind == CellKind::Error {
        return Ok(String::new());
    }
    let content = source.content(cell, reference)?;
    if cell.kind.is_text() {
        return Ok(content.trim().to_owned());
    }
    let value = super::cell::wire_scalar(
        cell.kind,
        source.styles.format(cell.style),
        source.system,
        &content,
    )
    .map_err(|error| source.refuse(reference, super::cell::wire_reason(&error)))?;
    Ok(super::cell::cell_text(&value).trim().to_owned())
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
                source.styles.format(cell.style),
                source.system,
                &content,
            )
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
    let mut rows = rows.filter(|row| {
        row.as_ref()
            .map_or(true, |row| source.range.contains_row(row.index))
    });
    let first = match rows.next() {
        Some(row) => Some(row?),
        None => None,
    };
    let mut header = Header::resolve(source, first.as_ref(), source.header)?;
    let mut dtypes: Vec<Option<DataType>> = vec![None; header.columns.len()];
    let mut present = vec![false; header.columns.len()];
    let mut nullable = vec![false; header.columns.len()];
    // Data rows observed so far: a column first seen after one lacked it
    // there, and is nullable for it.
    let mut observed = 0_usize;
    let mut observe = |header: &mut Header,
                       dtypes: &mut Vec<Option<DataType>>,
                       present: &mut Vec<bool>,
                       nullable: &mut Vec<bool>,
                       row: &RawRow|
     -> Result<()> {
        let lacked_before = observed > 0;
        observed += 1;
        let mut seen = vec![false; header.columns.len()];
        for cell in &row.cells {
            if !source.range.contains_column(cell.column) {
                continue;
            }
            let reference = CellRef::new(row.index, cell.column);
            let Some(dtype) = wire_dtype(source, cell, reference)? else {
                continue;
            };
            let at = header.admit(cell.column);
            if at >= seen.len() {
                seen.resize(header.columns.len(), false);
                dtypes.resize(header.columns.len(), None);
                present.resize(header.columns.len(), false);
                nullable.resize(header.columns.len(), lacked_before);
            } else if header.columns.len() > seen.len() {
                seen.insert(at, false);
                dtypes.insert(at, None);
                present.insert(at, false);
                nullable.insert(at, lacked_before);
            }
            seen[at] = true;
            present[at] = true;
            match &dtypes[at] {
                None => dtypes[at] = Some(dtype),
                Some(known) if *known != dtype => {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{}!{reference}", source.sheet),
                        reason: format_smolstr!(
                            "expected {known} like the column's first value, got {dtype}; declare a \
                             field to read the column as one datatype"
                        ),
                    });
                }
                Some(_) => {}
            }
        }
        for (at, seen) in seen.iter().enumerate() {
            if !seen {
                nullable[at] = true;
            }
        }
        Ok(())
    };
    if !source.header
        && let Some(first) = &first
    {
        observe(&mut header, &mut dtypes, &mut present, &mut nullable, first)?;
    }
    for row in rows {
        observe(&mut header, &mut dtypes, &mut present, &mut nullable, &row?)?;
    }
    let mut fields = Vec::with_capacity(header.columns.len());
    for at in 0..header.columns.len() {
        // A blank header over no value is a styled blank past the data.
        if !present[at] && !header.from_header[at] {
            continue;
        }
        fields.push(Field::new(
            header.names[at].clone(),
            dtypes[at].clone().unwrap_or(DataType::Null),
            nullable[at] || !present[at],
        ));
    }
    let fields = StructType::from_fields(fields).map_err(|error| Error::InvalidRecord {
        path: format_smolstr!("{}!{}", source.sheet, source.range.start()),
        reason: format_smolstr!("the header names no valid columns: {error}"),
    })?;
    Ok(Field::new(name, DataType::from(fields), false))
}

/// Count the records of a sheet: every `<row>` inside the range, less the
/// header.
///
/// # Errors
///
/// Returns the parser's refusal.
pub(crate) fn row_count<R: Read>(source: &Source, rows: SheetRows<BufReader<R>>) -> Result<u64> {
    let mut count = 0_u64;
    for row in rows {
        let row = row?;
        if source.range.contains_row(row.index) {
            count += 1;
        }
    }
    Ok(count.saturating_sub(u64::from(source.header)))
}

/// The rows of a sheet under a field, one positional `Scalar` per `<row>`.
struct RowStream<R: Read> {
    source: Source,
    rows: std::iter::Peekable<SheetRows<BufReader<R>>>,
    root: Arc<Field>,
    /// Per field child, the sheet column it reads, `None` for a column the
    /// sheet lacks.
    pairing: Vec<Option<u32>>,
    /// The cells of the row being read, one buffer drained into every run.
    values: Vec<Scalar>,
    safe: bool,
    /// Whether the header row has been consumed.
    started: bool,
    /// Whether the sheet holds no row at all, so nothing is paired or read.
    empty: bool,
    failed: bool,
}

impl<R: Read> RowStream<R> {
    /// Pair the field's children with the sheet's columns, reading the
    /// header row when there is one.
    fn start(&mut self) -> Result<()> {
        self.started = true;
        let header = if self.source.header {
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
        if self.source.header && header.is_none() {
            self.empty = true;
            return Ok(());
        }
        let header = Header::resolve(&self.source, header.as_ref(), self.source.header)?;
        let mut pairing = Vec::with_capacity(self.root.fields().len());
        for (index, child) in self.root.fields().iter().enumerate() {
            let column = if self.source.header {
                // A header cell of the child's name; else the column its
                // letters name, which is how a column with no header cell
                // over its values is named.
                header
                    .names
                    .iter()
                    .position(|name| name == child.name())
                    .map(|at| header.columns[at])
                    .or_else(|| {
                        CellRef::column_index(child.name())
                            .filter(|column| self.source.range.contains_column(*column))
                    })
            } else {
                Some(self.source.range.start().column() + index as u32)
            };
            if column.is_none() && !child.is_nullable() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{}", child.name()),
                    reason: format_smolstr!(
                        "expected the column in the header of {}, got [{}]",
                        self.source.sheet,
                        header
                            .names
                            .iter()
                            .map(SmolStr::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                });
            }
            pairing.push(column);
        }
        self.pairing = pairing;
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
        let row = loop {
            match self.rows.next() {
                Some(row) => {
                    let row = row?;
                    if self.source.range.contains_row(row.index) {
                        break row;
                    }
                }
                None => return Ok(None),
            }
        };
        // One buffer for every row: the run a row becomes is the one
        // allocation the row costs, and the buffer is drained into it.
        let mut values = std::mem::take(&mut self.values);
        values.clear();
        for (child, column) in self.root.fields().iter().zip(&self.pairing) {
            let cell = column.and_then(|column| {
                row.cells
                    .binary_search_by_key(&column, |cell| cell.column)
                    .ok()
                    .map(|at| &row.cells[at])
            });
            let value = match cell {
                Some(cell) if Source::is_present(cell) => {
                    let reference = CellRef::new(row.index, cell.column);
                    let content = self.source.content(cell, reference)?;
                    let format = self.source.styles.format(cell.style);
                    match super::cell::field_scalar(
                        child,
                        cell.kind,
                        format,
                        self.source.system,
                        &content,
                    ) {
                        Ok(value) => value,
                        Err(_) if self.safe && child.is_nullable() => Scalar::Null,
                        Err(error) => {
                            return Err(self
                                .source
                                .refuse(reference, super::cell::wire_reason(&error)));
                        }
                    }
                }
                // A required column with no cell in this row is refused by
                // the cell that is not there, before the row is widened.
                _ if !child.is_nullable() => {
                    let reference = CellRef::new(row.index, column.unwrap_or(0));
                    return Err(self.source.refuse(
                        reference,
                        format_smolstr!(
                            "expected a value for the required column {}, got no cell",
                            child.name()
                        ),
                    ));
                }
                _ => Scalar::Null,
            };
            values.push(value);
        }
        let row = Scalar::from_sequence(values.drain(..));
        self.values = values;
        Ok(Some(row))
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
pub(crate) fn batch_reader(
    source: Source,
    member: Box<dyn Read + Send>,
    root: &Field,
    safe: bool,
    batch_row_size: Option<usize>,
    batch_byte_size: Option<u64>,
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
        values: Vec::new(),
        safe,
        started: false,
        empty: false,
        failed: false,
    };
    crate::arrow::rows::result_reader(root, stream, batch_row_size, batch_byte_size, None)
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
