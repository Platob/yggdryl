//! One worksheet held whole: every cell it states, reachable by reference.
//!
//! A [`Sheet`] is the random-access model of a worksheet part - what a
//! caller opens to read `B7`, set `C3`, walk a column, window a range or
//! lay its rows out as a [`Serie`]. It holds its cells sparsely, by row then
//! column, so a cell costs what it holds and nothing else, and it is never
//! on the record path: a record read through [`Excel`](super::Excel)
//! streams the part and builds no cell.
//!
//! Every door takes a resolved [`CellRef`] or [`CellRange`], never A1 text:
//! `sheet.cell("B2".parse()?)` is the text spelling and `sheet.cell((1,
//! 1).into())` the numeric one, and a loop over cells parses nothing.

use std::collections::BTreeMap;
use std::io::Write;
use std::ops::Range;

use smol_str::{SmolStr, format_smolstr};

use crate::media::DEFAULT_ROOT_NAME;
use crate::{ArrowCastOptions, DataType, Error, Field, Result, Scalar, Serie, StructType};

use super::cell::{
    Cell, CellKind, CellRange, CellRef, DateSystem, MAX_COLUMNS, MAX_ROWS, cell_text,
};
use super::parser::RawRow;
use super::shared_strings::{SharedStringTable, SharedStrings};
use super::styles::Styles;

/// Excel's own limit on a sheet name's characters.
pub const MAX_SHEET_NAME: usize = 31;

/// Whether a sheet shows in the workbook's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SheetState {
    /// The tab is shown.
    #[default]
    Visible,
    /// The tab is hidden, and a user can unhide it.
    Hidden,
    /// The tab is hidden and only a macro can unhide it.
    VeryHidden,
}

impl SheetState {
    /// The `state` attribute as the file spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::VeryHidden => "veryHidden",
        }
    }

    /// The state a `state` attribute spells; an absent one is visible.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a value the schema does not list.
    pub fn from_attribute(value: Option<&str>) -> Result<Self> {
        Ok(match value.map(str::trim) {
            None | Some("visible") => Self::Visible,
            Some("hidden") => Self::Hidden,
            Some("veryHidden") => Self::VeryHidden,
            Some(other) => {
                return Err(Error::Parse {
                    target: "sheet state",
                    position: 0,
                    reason: format_smolstr!(
                        "expected visible, hidden or veryHidden for a sheet's state, got {other:?}"
                    ),
                });
            }
        })
    }
}

/// Refuse a sheet name Excel refuses: empty, over [`MAX_SHEET_NAME`]
/// characters, holding any of `\ / ? * [ ] :`, opening or closing with an
/// apostrophe, or the reserved `History`.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] naming the rule and the name.
pub fn validate_sheet_name(name: &str) -> Result<()> {
    let refuse = |reason: SmolStr| Error::InvalidRecord {
        path: SmolStr::new_static("$.sheet"),
        reason,
    };
    if name.is_empty() {
        return Err(refuse(SmolStr::new_static(
            "expected a sheet name, got the empty text",
        )));
    }
    let length = name.chars().count();
    if length > MAX_SHEET_NAME {
        return Err(refuse(format_smolstr!(
            "expected a sheet name of at most {MAX_SHEET_NAME} characters, got {length} in {name:?}"
        )));
    }
    if let Some(forbidden) = name
        .chars()
        .find(|character| matches!(character, '\\' | '/' | '?' | '*' | '[' | ']' | ':'))
    {
        return Err(refuse(format_smolstr!(
            "expected a sheet name without any of \\ / ? * [ ] :, got {forbidden:?} in {name:?}"
        )));
    }
    if name.starts_with('\'') || name.ends_with('\'') {
        return Err(refuse(format_smolstr!(
            "expected a sheet name that neither opens nor closes with an apostrophe, got {name:?}"
        )));
    }
    if name.eq_ignore_ascii_case("History") {
        return Err(refuse(SmolStr::new_static(
            "expected a sheet name other than the reserved `History`",
        )));
    }
    Ok(())
}

/// The cells of one row, by column.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    index: u32,
    cells: BTreeMap<u32, Cell>,
}

impl Row {
    /// The zero-based row.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// The cells present, in column order.
    pub fn cells(&self) -> impl Iterator<Item = &Cell> + '_ {
        self.cells.values()
    }

    /// The cell at zero-based `column`, when present.
    #[must_use]
    pub fn cell(&self, column: u32) -> Option<&Cell> {
        self.cells.get(&column)
    }

    /// How many cells the row holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether the row holds no cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

/// One worksheet, every cell of it in memory.
///
/// Held state: every cell the part stated, until the sheet is dropped; the
/// reason is the random access - a cell at any reference, in either
/// direction, without a second read of the part.
///
/// ```
/// use yggdryl::excel::{CellRange, CellRef, Sheet};
/// use yggdryl::{Field, Scalar, Serie};
///
/// let mut sheet = Sheet::new("Trades")?;
/// sheet.set_cell("A1".parse()?, "symbol")?;
/// sheet.set_cell("B1".parse()?, "price")?;
/// sheet.set_cell((1, 0).into(), "AAPL")?;
/// sheet.set_cell((1, 1).into(), 187.23)?;
///
/// assert_eq!(sheet.scalar((1, 1).into()), Scalar::from(187.23));
/// assert_eq!(sheet.scalar((5, 5).into()), Scalar::Null);
/// assert_eq!(sheet.dimension(), Some("A1:B2".parse::<CellRange>()?));
///
/// // The header row names the columns; every other row is a record.
/// let rows = sheet.clone().into_serie(None, true, Default::default())?;
/// assert_eq!(rows.len(), 1);
/// assert_eq!(rows.child("price").and_then(|price| price.scalar(0).ok()), Some(Scalar::from(187.23)));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    name: SmolStr,
    state: SheetState,
    system: DateSystem,
    rows: BTreeMap<u32, Row>,
}

impl Sheet {
    /// An empty sheet named `name`, under the 1900 date system.
    ///
    /// # Errors
    ///
    /// Returns the name's refusal ([`validate_sheet_name`]).
    pub fn new(name: impl Into<SmolStr>) -> Result<Self> {
        let name = name.into();
        validate_sheet_name(&name)?;
        Ok(Self {
            name,
            state: SheetState::Visible,
            system: DateSystem::Year1900,
            rows: BTreeMap::new(),
        })
    }

    /// This sheet counting its serial dates from `system`.
    #[must_use]
    pub const fn with_date_system(mut self, system: DateSystem) -> Self {
        self.system = system;
        self
    }

    /// This sheet in `state`.
    #[must_use]
    pub const fn with_state(mut self, state: SheetState) -> Self {
        self.state = state;
        self
    }

    /// The sheet's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Rename the sheet.
    ///
    /// # Errors
    ///
    /// Returns the name's refusal ([`validate_sheet_name`]).
    pub fn set_name(&mut self, name: impl Into<SmolStr>) -> Result<()> {
        let name = name.into();
        validate_sheet_name(&name)?;
        self.name = name;
        Ok(())
    }

    /// Whether the sheet shows in the tabs.
    #[must_use]
    pub const fn state(&self) -> SheetState {
        self.state
    }

    /// Count serial dates from `system` when the sheet is written; the
    /// cells hold typed values, so nothing else changes. A workbook sets
    /// every sheet it holds when its own system is set.
    pub const fn set_date_system(&mut self, system: DateSystem) {
        self.system = system;
    }

    /// Show or hide the sheet.
    pub const fn set_state(&mut self, state: SheetState) {
        self.state = state;
    }

    /// The date system the sheet's serials are read and written under.
    #[must_use]
    pub const fn date_system(&self) -> DateSystem {
        self.system
    }

    /// How many rows hold a cell.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the sheet holds no cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rectangle the cells span, `None` for an empty sheet.
    #[must_use]
    pub fn dimension(&self) -> Option<CellRange> {
        let first_row = *self.rows.keys().next()?;
        let last_row = *self.rows.keys().next_back()?;
        let mut first_column = u32::MAX;
        let mut last_column = 0;
        for row in self.rows.values() {
            if let Some(column) = row.cells.keys().next() {
                first_column = first_column.min(*column);
            }
            if let Some(column) = row.cells.keys().next_back() {
                last_column = last_column.max(*column);
            }
        }
        Some(CellRange::new(
            CellRef::new(first_row, first_column),
            CellRef::new(last_row, last_column),
        ))
    }

    /// The cell at `reference`, when present.
    #[must_use]
    pub fn cell(&self, reference: CellRef) -> Option<&Cell> {
        self.rows
            .get(&reference.row())?
            .cells
            .get(&reference.column())
    }

    /// The cell at `reference`, mutably, when present.
    pub fn cell_mut(&mut self, reference: CellRef) -> Option<&mut Cell> {
        self.rows
            .get_mut(&reference.row())?
            .cells
            .get_mut(&reference.column())
    }

    /// The value at `reference`: null where no cell is.
    #[must_use]
    pub fn scalar(&self, reference: CellRef) -> Scalar {
        self.cell(reference)
            .map_or(Scalar::Null, |cell| cell.value().clone())
    }

    /// Put `value` at `reference`, answering the cell it replaced.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a reference outside the grid or a
    /// value the cell cannot spell ([`Cell::from_scalar`]).
    pub fn set_cell(
        &mut self,
        reference: CellRef,
        value: impl Into<Scalar>,
    ) -> Result<Option<Cell>> {
        let cell = Cell::from_scalar(reference.require_in_grid()?, value.into(), self.system)?;
        Ok(self.insert(cell))
    }

    /// Put a prebuilt cell - one carrying a formula, say - at its reference,
    /// answering the cell it replaced.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a reference outside the grid.
    pub fn insert_cell(&mut self, cell: Cell) -> Result<Option<Cell>> {
        cell.reference().require_in_grid()?;
        Ok(self.insert(cell))
    }

    fn insert(&mut self, cell: Cell) -> Option<Cell> {
        let reference = cell.reference();
        let row = self.rows.entry(reference.row()).or_insert_with(|| Row {
            index: reference.row(),
            cells: BTreeMap::new(),
        });
        row.cells.insert(reference.column(), cell)
    }

    /// Take the cell at `reference` out, when present.
    pub fn remove_cell(&mut self, reference: CellRef) -> Option<Cell> {
        let row = self.rows.get_mut(&reference.row())?;
        let removed = row.cells.remove(&reference.column());
        if row.cells.is_empty() {
            self.rows.remove(&reference.row());
        }
        removed
    }

    /// The rows holding a cell, in order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> + '_ {
        self.rows.values()
    }

    /// The row at zero-based `index`, when it holds a cell.
    #[must_use]
    pub fn row(&self, index: u32) -> Option<&Row> {
        self.rows.get(&index)
    }

    /// Every cell, row by row.
    pub fn cells(&self) -> impl Iterator<Item = &Cell> + '_ {
        self.rows.values().flat_map(|row| row.cells.values())
    }

    /// The cells inside `range`, row by row.
    pub fn cells_in(&self, range: CellRange) -> impl Iterator<Item = &Cell> + '_ {
        self.rows
            .range(range.start().row()..=range.end().row())
            .flat_map(move |(_, row)| {
                row.cells
                    .range(range.start().column()..=range.end().column())
                    .map(|(_, cell)| cell)
            })
    }

    /// The cells of zero-based `column`, top to bottom.
    pub fn column(&self, column: u32) -> impl Iterator<Item = &Cell> + '_ {
        self.rows
            .values()
            .filter_map(move |row| row.cells.get(&column))
    }

    /// The cells inside `range`, as a sheet of their own, references kept.
    #[must_use]
    pub fn slice(&self, range: CellRange) -> Self {
        let mut sliced = Self {
            name: self.name.clone(),
            state: self.state,
            system: self.system,
            rows: BTreeMap::new(),
        };
        for cell in self.cells_in(range) {
            sliced.insert(cell.clone());
        }
        sliced
    }

    /// Open `count` empty rows at zero-based `at`, moving every row from
    /// `at` down. Formulas are not rewritten.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when a moved row would leave the grid.
    pub fn insert_rows(&mut self, at: u32, count: u32) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        if let Some(last) = self.rows.keys().next_back().copied() {
            if last >= at
                && last
                    .checked_add(count)
                    .is_none_or(|moved| moved >= MAX_ROWS)
            {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!(
                        "expected the moved rows to stay within {MAX_ROWS} rows, got row {} moving by {count}",
                        last + 1
                    ),
                });
            }
        }
        let moved: Vec<(u32, Row)> = self
            .rows
            .split_off(&at)
            .into_iter()
            .map(|(index, row)| (index + count, row))
            .collect();
        for (index, mut row) in moved {
            row.index = index;
            row.cells = row
                .cells
                .into_iter()
                .map(|(column, cell)| (column, cell.at(CellRef::new(index, column))))
                .collect();
            self.rows.insert(index, row);
        }
        Ok(())
    }

    /// Remove the rows in `range`, moving every row below up. Formulas are
    /// not rewritten.
    pub fn remove_rows(&mut self, range: Range<u32>) {
        if range.is_empty() {
            return;
        }
        let count = range.end - range.start;
        let below = self.rows.split_off(&range.end);
        self.rows.split_off(&range.start);
        for (index, mut row) in below {
            let index = index - count;
            row.index = index;
            row.cells = row
                .cells
                .into_iter()
                .map(|(column, cell)| (column, cell.at(CellRef::new(index, column))))
                .collect();
            self.rows.insert(index, row);
        }
    }

    /// A sheet holding the rows of `serie` from `A1`: the column names in the
    /// first row when `header`, then one row per record.
    ///
    /// A record column is laid out column by column; any other column is the
    /// one column of a record named as it is, the rule
    /// [`SerieReader::from_serie`](crate::SerieReader::from_serie) states;
    /// a run, which names no columns, is refused.
    ///
    /// # Errors
    ///
    /// Returns the name's refusal, a run, or a value no cell spells.
    pub fn from_serie(name: impl Into<SmolStr>, serie: &Serie, header: bool) -> Result<Self> {
        let mut sheet = Self::new(name)?;
        sheet.write_serie(CellRef::new(0, 0), serie, header)?;
        Ok(sheet)
    }

    /// Write the rows of `serie` with their top-left cell at `anchor`: the
    /// column names in the anchor's row when `header`, then one row per
    /// record; cells already there are replaced.
    ///
    /// # Errors
    ///
    /// Returns a refusal for a run, for rows or columns that would leave
    /// the grid, or for a value no cell spells.
    pub fn write_serie(&mut self, anchor: CellRef, serie: &Serie, header: bool) -> Result<()> {
        let (root, columns) = record_columns(serie)?;
        let rows = serie.len() + usize::from(header);
        let last_row = u64::from(anchor.row()) + rows as u64;
        let last_column = u64::from(anchor.column()) + columns.len() as u64;
        if last_row > u64::from(MAX_ROWS) || last_column > u64::from(MAX_COLUMNS) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected {rows} rows and {} columns from {anchor} to fit {MAX_ROWS} rows by \
                     {MAX_COLUMNS} columns",
                    columns.len()
                ),
            });
        }
        let mut row = anchor.row();
        if header {
            for (offset, field) in root.fields().iter().enumerate() {
                let reference = CellRef::new(row, anchor.column() + offset as u32);
                self.insert(Cell::from_scalar(
                    reference,
                    Scalar::from(field.name()),
                    self.system,
                )?);
            }
            row += 1;
        }
        for index in 0..serie.len() {
            for (offset, column) in columns.iter().enumerate() {
                let value = column.scalar(index)?;
                let reference = CellRef::new(row, anchor.column() + offset as u32);
                // A null is a cell holding nothing: what was there goes.
                if value.is_null() {
                    self.remove_cell(reference);
                    continue;
                }
                self.insert(Cell::from_scalar(reference, value, self.system)?);
            }
            row += 1;
        }
        Ok(())
    }

    /// Append the rows of `serie` below the last row present, column by
    /// column from the sheet's first column, with no header.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::write_serie`] returns.
    pub fn extend_from_serie(&mut self, serie: &Serie) -> Result<()> {
        let anchor = match self.dimension() {
            Some(span) => CellRef::new(span.end().row() + 1, span.start().column()),
            None => CellRef::new(0, 0),
        };
        self.write_serie(anchor, serie, false)
    }

    /// Lay the sheet's rows out as one record column.
    ///
    /// The first row present names the columns when `header`, else the
    /// columns are named by their letters; every later row is one record,
    /// null where it holds no cell. With no `field` each column's datatype is
    /// the one its first present value proves - `float64` for a number, the
    /// temporal a date style spells, `boolean`, `utf8` - and a later value of
    /// another datatype is refused naming the cell, so the caller declares
    /// the field. With a `field`, each of its columns is read from the sheet
    /// column of the same header name (or the same position, without a
    /// header) through the field's value contract; a value the column cannot
    /// hold is null under `options.safe` and refused naming the cell
    /// otherwise; a column the sheet lacks is null, or refused when the field
    /// requires it.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming the sheet and the cell.
    pub fn into_serie(
        self,
        field: Option<&Field>,
        header: bool,
        options: ArrowCastOptions,
    ) -> Result<Serie> {
        let Some(span) = self.dimension() else {
            let root = match field {
                Some(field) => field.clone().with_nullable(false),
                None => empty_root()?,
            };
            return Serie::from_scalars(root, std::iter::empty());
        };
        let first_column = span.start().column();
        let width = span.column_size() as usize;
        let mut rows = self.rows.values();
        let header_row = if header { rows.next() } else { None };
        let mut names: Vec<SmolStr> = (0..width)
            .map(|offset| CellRef::column_name(first_column + offset as u32))
            .collect();
        // Which columns a header cell names, as against their letters.
        let mut from_header = vec![false; width];
        if let Some(header_row) = header_row {
            for (offset, name) in names.iter_mut().enumerate() {
                if let Some(cell) = header_row.cell(first_column + offset as u32) {
                    let text = cell.text();
                    if !text.is_empty() {
                        *name = SmolStr::new(text.as_ref());
                        from_header[offset] = true;
                    }
                }
            }
        }
        let body: Vec<&Row> = rows.collect();
        match field {
            Some(field) => self.rows_under(field, &names, header, first_column, &body, options),
            None => self.rows_inferred(&names, &from_header, span.start(), &body),
        }
    }

    /// Rows typed by what their cells prove, the datatype of each column
    /// its first present value's.
    fn rows_inferred(
        &self,
        names: &[SmolStr],
        from_header: &[bool],
        anchor: CellRef,
        body: &[&Row],
    ) -> Result<Serie> {
        let first_column = anchor.column();
        let width = names.len();
        let mut dtypes: Vec<Option<DataType>> = vec![None; width];
        let mut nullable = vec![false; width];
        // A column an empty header names over no value below it is not a
        // column at all: Excel writes styled blanks past the data.
        let mut present = vec![false; width];
        for row in body {
            for offset in 0..width {
                let column = first_column + offset as u32;
                let Some(cell) = row.cell(column).filter(|cell| !cell.is_null()) else {
                    nullable[offset] = true;
                    continue;
                };
                present[offset] = true;
                let dtype = cell
                    .value()
                    .dtype()
                    .map_err(|error| located(&self.name, cell.reference(), error))?;
                match &dtypes[offset] {
                    None => dtypes[offset] = Some(dtype),
                    Some(known) if *known != dtype => {
                        return Err(Error::InvalidRecord {
                            path: format_smolstr!("{}!{}", self.name, cell.reference()),
                            reason: format_smolstr!(
                                "expected {known} like the column's first value, got {dtype}; \
                                 declare a field to read the column as one datatype"
                            ),
                        });
                    }
                    Some(_) => {}
                }
            }
        }
        let mut fields = Vec::with_capacity(width);
        let mut kept = Vec::with_capacity(width);
        for offset in 0..width {
            if !present[offset] && !from_header[offset] {
                continue;
            }
            let dtype = dtypes[offset].clone().unwrap_or(DataType::Null);
            fields.push(Field::new(
                names[offset].clone(),
                dtype,
                nullable[offset] || !present[offset],
            ));
            kept.push(offset);
        }
        let root = Field::new(
            DEFAULT_ROOT_NAME,
            DataType::from(StructType::from_fields(fields).map_err(|error| {
                Error::InvalidRecord {
                    path: format_smolstr!("{}!{anchor}", self.name),
                    reason: format_smolstr!("the header names no valid columns: {error}"),
                }
            })?),
            false,
        );
        let rows = body.iter().map(|row| {
            Scalar::from_sequence(kept.iter().map(|offset| {
                row.cell(first_column + *offset as u32)
                    .map_or(Scalar::Null, |cell| cell.value().clone())
            }))
        });
        Serie::from_scalars(root, rows)
    }

    /// Rows laid out under a declared field, each cell through the column's
    /// value contract.
    fn rows_under(
        &self,
        field: &Field,
        names: &[SmolStr],
        header: bool,
        first_column: u32,
        body: &[&Row],
        options: ArrowCastOptions,
    ) -> Result<Serie> {
        let root = field.clone().with_nullable(false);
        // A header pairs a declared column with the sheet column of its
        // name and nothing else; without one, position pairs them.
        let pairing: Vec<Option<u32>> = root
            .fields()
            .iter()
            .enumerate()
            .map(|(index, child)| {
                if header {
                    names.iter().position(|name| name == child.name())
                } else {
                    (index < names.len()).then_some(index)
                }
                .map(|offset| first_column + offset as u32)
            })
            .collect();
        let safe = options.is_safe();
        let mut rows = Vec::with_capacity(body.len());
        // One buffer for every row, drained into the run each row becomes.
        let mut values = Vec::with_capacity(pairing.len());
        for row in body {
            values.clear();
            for (child, column) in root.fields().iter().zip(&pairing) {
                let cell = column.and_then(|column| row.cell(column));
                values.push(match cell {
                    Some(cell) => cell_under(cell, child, safe)
                        .map_err(|error| located(&self.name, cell.reference(), error))?,
                    None => Scalar::Null,
                });
            }
            rows.push(Scalar::from_sequence(values.drain(..)));
        }
        Serie::from_scalars(root, rows)
    }

    /// Write the worksheet part: the span, then every row and cell in
    /// ascending order, text interned into `strings`, each cell's style the
    /// crate's own index plus `style_offset`.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure or a value no cell spells.
    pub(crate) fn write_xml<W: Write>(
        &self,
        writer: &mut W,
        strings: &mut SharedStringTable,
        style_offset: u32,
    ) -> Result<()> {
        write!(
            writer,
            "<worksheet xmlns=\"{}\" xmlns:r=\"{}\">",
            super::NAMESPACE,
            super::RELATIONSHIPS_NAMESPACE
        )?;
        if let Some(span) = self.dimension() {
            write!(writer, "<dimension ref=\"{span}\"/>")?;
        }
        write!(writer, "<sheetData>")?;
        let mut reference = String::with_capacity(12);
        for row in self.rows.values() {
            if row.cells.is_empty() {
                continue;
            }
            write!(writer, "<row r=\"{}\">", row.index + 1)?;
            for cell in row.cells.values() {
                reference.clear();
                cell.reference().write_a1(&mut reference);
                write_cell(writer, cell, &reference, strings, style_offset, self.system)?;
            }
            write!(writer, "</row>")?;
        }
        write!(writer, "</sheetData></worksheet>")?;
        Ok(())
    }

    /// Build the sheet from the rows of its part.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming the sheet and the cell.
    pub(crate) fn from_rows(
        name: SmolStr,
        state: SheetState,
        system: DateSystem,
        rows: impl Iterator<Item = Result<RawRow>>,
        strings: &SharedStrings,
        styles: &Styles,
    ) -> Result<Self> {
        let mut sheet = Self {
            name,
            state,
            system,
            rows: BTreeMap::new(),
        };
        for row in rows {
            let row = row?;
            for raw in row.cells {
                let reference = CellRef::new(row.index, raw.column);
                let format = styles.format(raw.style);
                // A shared string's content is its index, resolved to the
                // table's text; nothing is copied either way.
                let content: &str = if raw.kind == CellKind::SharedString && raw.has_content {
                    let index: usize =
                        raw.content
                            .trim()
                            .parse()
                            .map_err(|_| Error::InvalidRecord {
                                path: format_smolstr!("{}!{reference}", sheet.name),
                                reason: format_smolstr!(
                                    "expected a shared string index, got {:?}",
                                    raw.content
                                ),
                            })?;
                    strings
                        .get(index)
                        .ok_or_else(|| Error::InvalidRecord {
                            path: format_smolstr!("{}!{reference}", sheet.name),
                            reason: format_smolstr!(
                                "expected a shared string index below {}, got {index}",
                                strings.len()
                            ),
                        })?
                        .as_str()
                } else {
                    raw.content.as_str()
                };
                let value = if raw.has_content || raw.kind.is_text() {
                    super::cell::wire_scalar(raw.kind, format, system, content).map_err(
                        |error| Error::InvalidRecord {
                            path: format_smolstr!("{}!{reference}", sheet.name),
                            reason: super::cell::wire_reason(&error),
                        },
                    )?
                } else {
                    Scalar::Null
                };
                let mut cell = Cell::new(reference, raw.kind, format, value);
                if raw.kind == CellKind::Error {
                    cell = cell.with_error(content.trim());
                }
                if let Some(formula) = raw.formula {
                    cell = cell.with_formula(formula);
                }
                sheet.insert(cell);
            }
        }
        Ok(sheet)
    }
}

/// A cell's value under the column `field` declares.
///
/// The value crosses the field's own contract; where that refuses a value
/// the cell's text spelling is tried through the text door every document
/// leaf crosses, so `7` written as a number lands in an `int64` column and
/// `2024-01-02` written as text in a `date32` one. What both refuse is null
/// under `safe` on a nullable column, else the refusal.
fn cell_under(cell: &Cell, field: &Field, safe: bool) -> Result<Scalar> {
    if let Some(error) = cell.error() {
        let refused = Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("expected a {} value, got the error {error}", field.dtype()),
        };
        return if safe && field.is_nullable() {
            Ok(Scalar::Null)
        } else {
            Err(refused)
        };
    }
    let value = cell.value();
    if value.is_null() {
        return field.scalar(Scalar::Null);
    }
    let direct = if field.dtype().string_parameters().is_some() {
        field.scalar(Scalar::from(cell_text(value).into_owned()))
    } else {
        field.scalar(value.clone())
    };
    let read = direct
        .or_else(|_| crate::text::prepare_text(Scalar::from(cell_text(value).into_owned()), field));
    match read {
        Ok(value) => Ok(value),
        Err(_) if safe && field.is_nullable() => Ok(Scalar::Null),
        Err(error) => Err(error),
    }
}

/// Name the sheet and cell an error is about.
fn located(sheet: &str, reference: CellRef, error: Error) -> Error {
    match error {
        Error::InvalidRecord { reason, .. } => Error::InvalidRecord {
            path: format_smolstr!("{sheet}!{reference}"),
            reason,
        },
        other => other,
    }
}

/// The record root and the columns `serie` lays out: a record column's own
/// children, or any other column as the one child of a record named as it is.
fn record_columns(serie: &Serie) -> Result<(Field, Vec<Serie>)> {
    let field = serie.require_field()?;
    if let Some(record) = serie.as_struct() {
        return Ok((
            field.clone().with_nullable(false),
            record.children().to_vec(),
        ));
    }
    let root = Field::new(
        DEFAULT_ROOT_NAME,
        DataType::from(StructType::from_fields([field.clone()])?),
        false,
    );
    Ok((root, vec![serie.clone()]))
}

/// The record root of a sheet with no cell: no columns.
fn empty_root() -> Result<Field> {
    Ok(Field::new(
        DEFAULT_ROOT_NAME,
        DataType::from(StructType::from_fields(std::iter::empty::<Field>())?),
        false,
    ))
}

/// Write one `<c>` for `cell`.
///
/// Text is interned; a cell carrying a formula writes it first and types its
/// cached text `str`, never `s`; a temporal writes its serial under its
/// format's style; a boolean `b`; an error `e`.
pub(crate) fn write_cell<W: Write>(
    writer: &mut W,
    cell: &Cell,
    reference: &str,
    strings: &mut SharedStringTable,
    style_offset: u32,
    system: DateSystem,
) -> Result<()> {
    let value = cell.value();
    write!(writer, "<c r=\"{reference}\"")?;
    if cell.format().is_temporal() {
        write!(
            writer,
            " s=\"{}\"",
            cell.format().style_index() + style_offset
        )?;
    }
    if let Some(error) = cell.error() {
        write!(writer, " t=\"e\">")?;
        write_formula(writer, cell)?;
        write!(writer, "<v>")?;
        crate::xml::write_element_text(writer, error)?;
        write!(writer, "</v></c>")?;
        return Ok(());
    }
    match cell.kind() {
        CellKind::Boolean => {
            write!(writer, " t=\"b\">")?;
            write_formula(writer, cell)?;
            write!(
                writer,
                "<v>{}</v></c>",
                u8::from(value.as_bool().unwrap_or(false))
            )?;
        }
        CellKind::Number | CellKind::Date => {
            if value.is_null() {
                write!(writer, ">")?;
                write_formula(writer, cell)?;
                write!(writer, "</c>")?;
                return Ok(());
            }
            let serial = system.serial_of(value)?;
            write!(writer, ">")?;
            write_formula(writer, cell)?;
            write!(writer, "<v>")?;
            match serial {
                Some((serial, _)) => write!(writer, "{}", super::cell::serial_text(serial))?,
                None => super::cell::write_cell_text(writer, value)?,
            }
            write!(writer, "</v></c>")?;
        }
        CellKind::SharedString | CellKind::InlineString | CellKind::FormulaString => {
            let text = cell.text();
            if cell.formula().is_some() {
                write!(writer, " t=\"str\">")?;
                write_formula(writer, cell)?;
                write!(writer, "<v>")?;
                crate::xml::write_element_text(writer, &super::shared_strings::encode(&text))?;
                write!(writer, "</v></c>")?;
            } else {
                let index = strings.intern(&text);
                write!(writer, " t=\"s\"><v>{index}</v></c>")?;
            }
        }
        CellKind::Error => unreachable!("an error cell carries its error"),
    }
    Ok(())
}

/// Write the cell's `<f>`, when it carries one.
fn write_formula<W: Write>(writer: &mut W, cell: &Cell) -> Result<()> {
    if let Some(formula) = cell.formula() {
        write!(writer, "<f>")?;
        crate::xml::write_element_text(writer, formula)?;
        write!(writer, "</f>")?;
    }
    Ok(())
}
