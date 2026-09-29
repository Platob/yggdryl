//! JavaScript views over the Excel workbook, its sheets and cells.
//!
//! Every value here is the core's: a `Workbook` is [`yggdryl::excel::Workbook`]
//! behind a lock two views may share, a `Sheet` is either a sheet of its own or
//! a live view of one the workbook holds, and a `Cell`, a `CellRef` and a
//! `CellRange` are the core values. A cell's value crosses as `Scalar`, a
//! sheet's rows as `Serie`; the loader adds the JavaScript natives on top.

use std::sync::{Arc, Mutex, MutexGuard};

use napi::bindgen_prelude::{Buffer, ClassInstance, Either, Either3, Result};
use napi_derive::napi;
use yggdryl::IOBase;
use yggdryl::excel::{
    Cell, CellKind, CellRange, CellRef, DateSystem, Row, Sheet, SheetState, Workbook,
};
use yggdryl::holder::{Buffer as CoreBuffer, Holder};

use crate::field::JsField;
use crate::iobase::{LocationInput, located_from_input};
use crate::serie::JsSerie;
use crate::text::codec::JsScalar;
use crate::{cast_options, napi_error, ordering_value};

/// A workbook two views may hold.
type Shared = Arc<Mutex<Workbook>>;

fn lock(shared: &Shared) -> Result<MutexGuard<'_, Workbook>> {
    shared
        .lock()
        .map_err(|_| napi_error("the workbook was poisoned by a panic"))
}

/// A cell reference: a `CellRef`, its `A1` text, or a `[row, column]` pair.
pub type CellRefInput<'a> = Either3<ClassInstance<'a, JsCellRef>, String, Vec<u32>>;

/// A cell range: a `CellRange`, its `A1:C3` text, or a pair of references.
pub type CellRangeInput<'a> =
    Either3<ClassInstance<'a, JsCellRange>, String, Vec<CellRefInput<'a>>>;

pub(crate) fn cell_ref_from(value: CellRefInput<'_>) -> Result<CellRef> {
    match value {
        Either3::A(reference) => Ok(reference.inner),
        Either3::B(text) => text.parse::<CellRef>().map_err(napi_error),
        Either3::C(pair) => {
            let [row, column] = pair[..] else {
                return Err(napi_error(format!(
                    "expected a [row, column] pair, got {} numbers",
                    pair.len()
                )));
            };
            CellRef::new(row, column)
                .require_in_grid()
                .map_err(napi_error)
        }
    }
}

pub(crate) fn cell_range_from(value: CellRangeInput<'_>) -> Result<CellRange> {
    match value {
        Either3::A(range) => Ok(range.inner),
        Either3::B(text) => text.parse::<CellRange>().map_err(napi_error),
        Either3::C(mut pair) => {
            if pair.len() != 2 {
                return Err(napi_error(format!(
                    "expected a pair of cell references, got {}",
                    pair.len()
                )));
            }
            let second = cell_ref_from(pair.pop().expect("two items"))?;
            let first = cell_ref_from(pair.pop().expect("two items"))?;
            Ok(CellRange::new(first, second))
        }
    }
}

fn date_system_from(value: Option<String>) -> Result<DateSystem> {
    value.map_or(Ok(DateSystem::Year1900), |value| {
        value.parse().map_err(napi_error)
    })
}

fn sheet_state_from(value: Option<String>) -> Result<SheetState> {
    SheetState::from_attribute(value.as_deref()).map_err(napi_error)
}

/// One cell's position: a zero-based row and column, spelled `A1`.
#[napi(js_name = "CellRef")]
#[derive(Clone, Copy)]
pub struct JsCellRef {
    pub(crate) inner: CellRef,
}

#[napi]
impl JsCellRef {
    /// Read a reference: `A1` text with or without `$` anchors, a
    /// `[row, column]` pair of zero-based integers, or another `CellRef`.
    #[napi(constructor)]
    pub fn new(value: CellRefInput<'_>) -> Result<Self> {
        cell_ref_from(value).map(|inner| Self { inner })
    }

    /// The zero-based row.
    #[napi(getter)]
    pub fn row(&self) -> u32 {
        self.inner.row()
    }

    /// The zero-based column.
    #[napi(getter)]
    pub fn column(&self) -> u32 {
        self.inner.column()
    }

    /// Whether the reference lies inside the 1,048,576 by 16,384 grid.
    #[napi]
    pub fn is_in_grid(&self) -> bool {
        self.inner.is_in_grid()
    }

    /// The letters of the zero-based `column`: `0` is `A`, `26` is `AA`.
    #[napi]
    pub fn column_name(column: u32) -> String {
        CellRef::column_name(column).to_string()
    }

    /// The zero-based column the `letters` spell, `null` for anything that
    /// is not letters or lies past `XFD`.
    #[napi]
    pub fn column_index(letters: String) -> Option<u32> {
        CellRef::column_index(&letters)
    }

    #[napi]
    pub fn equals(&self, other: &JsCellRef) -> bool {
        self.inner == other.inner
    }

    #[napi]
    pub fn compare(&self, other: &JsCellRef) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    #[napi]
    pub fn clone(&self) -> Self {
        *self
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_string()
    }

    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_string()
    }
}

/// A rectangle of cells, spelled `A1:C3`, `A:C`, `3:5` or `A3:F`.
#[napi(js_name = "CellRange")]
#[derive(Clone, Copy)]
pub struct JsCellRange {
    pub(crate) inner: CellRange,
}

#[napi]
impl JsCellRange {
    /// Read a range: `A1:C3` text, an open `A:C`, `3:5` or `A3:F`, a pair
    /// of cell references in any order, or another `CellRange`.
    #[napi(constructor)]
    pub fn new(value: CellRangeInput<'_>) -> Result<Self> {
        cell_range_from(value).map(|inner| Self { inner })
    }

    /// The whole grid.
    #[napi]
    pub fn all() -> Self {
        Self {
            inner: CellRange::all(),
        }
    }

    /// The top-left cell.
    #[napi(getter)]
    pub fn start(&self) -> JsCellRef {
        JsCellRef {
            inner: self.inner.start(),
        }
    }

    /// The bottom-right cell.
    #[napi(getter)]
    pub fn end(&self) -> JsCellRef {
        JsCellRef {
            inner: self.inner.end(),
        }
    }

    /// How many rows the range spans.
    #[napi(getter)]
    pub fn row_size(&self) -> u32 {
        self.inner.row_size()
    }

    /// How many columns the range spans.
    #[napi(getter)]
    pub fn column_size(&self) -> u32 {
        self.inner.column_size()
    }

    /// Whether the range runs to the last row of the grid.
    #[napi]
    pub fn is_row_open(&self) -> bool {
        self.inner.is_row_open()
    }

    /// Whether the range runs to the last column of the grid.
    #[napi]
    pub fn is_column_open(&self) -> bool {
        self.inner.is_column_open()
    }

    /// Whether `cell` lies inside the range.
    #[napi]
    pub fn contains(&self, cell: CellRefInput<'_>) -> Result<bool> {
        Ok(self.inner.contains(cell_ref_from(cell)?))
    }

    /// Whether the zero-based `row` lies inside the range.
    #[napi]
    pub fn contains_row(&self, row: u32) -> bool {
        self.inner.contains_row(row)
    }

    /// Whether the zero-based `column` lies inside the range.
    #[napi]
    pub fn contains_column(&self, column: u32) -> bool {
        self.inner.contains_column(column)
    }

    #[napi]
    pub fn equals(&self, other: &JsCellRange) -> bool {
        self.inner == other.inner
    }

    #[napi]
    pub fn compare(&self, other: &JsCellRange) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    #[napi]
    pub fn clone(&self) -> Self {
        *self
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_string()
    }

    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_string()
    }
}

/// One cell: its reference, what kind of content it states, the number
/// format its style declares, its value, and the formula or error it carries.
#[napi(js_name = "Cell")]
#[derive(Clone)]
pub struct JsCell {
    pub(crate) inner: Cell,
}

#[napi]
impl JsCell {
    /// A cell holding the native `value` at `reference`, its serials under
    /// `dateSystem`; the loader's `Cell` constructor reads a JavaScript
    /// value into the `Scalar` this takes.
    #[napi(factory, js_name = "_fromScalarNative", skip_typescript)]
    pub fn from_scalar_native(
        reference: CellRefInput<'_>,
        value: &JsScalar,
        date_system: Option<String>,
    ) -> Result<Self> {
        let reference = cell_ref_from(reference)?;
        Cell::from_scalar(
            reference,
            value.inner.clone(),
            date_system_from(date_system)?,
        )
        .map(|inner| Self { inner })
        .map_err(napi_error)
    }

    /// A cell from its parts, as `toJSON` spells them.
    #[napi(factory)]
    pub fn from_parts(
        reference: CellRefInput<'_>,
        kind: String,
        format: String,
        value: &JsScalar,
        formula: Option<String>,
        error: Option<String>,
    ) -> Result<Self> {
        let mut cell = Cell::new(
            cell_ref_from(reference)?,
            CellKind::from_attribute(&kind).map_err(napi_error)?,
            format.parse().map_err(napi_error)?,
            value.inner.clone(),
        );
        if let Some(formula) = formula {
            cell = cell.with_formula(formula);
        }
        if let Some(error) = error {
            cell = cell.with_error(error);
        }
        Ok(Self { inner: cell })
    }

    /// Where the cell is.
    #[napi(getter)]
    pub fn reference(&self) -> JsCellRef {
        JsCellRef {
            inner: self.inner.reference(),
        }
    }

    /// The zero-based row.
    #[napi(getter)]
    pub fn row(&self) -> u32 {
        self.inner.row()
    }

    /// The zero-based column.
    #[napi(getter)]
    pub fn column(&self) -> u32 {
        self.inner.column()
    }

    /// The `t` attribute the cell states: `n`, `s`, `str`, `inlineStr`,
    /// `b`, `d` or `e`.
    #[napi(getter)]
    pub fn kind(&self) -> &'static str {
        self.inner.kind().as_str().unwrap_or("n")
    }

    /// The number format the cell's style classifies as: `general`,
    /// `date`, `time`, `datetime`, `datetime_fraction` or `duration`.
    #[napi(getter)]
    pub fn format(&self) -> &'static str {
        self.inner.format().as_str()
    }

    /// The value the cell holds, as the core reads it.
    #[napi(getter)]
    pub fn value(&self) -> JsScalar {
        JsScalar::from_core(self.inner.value().clone())
    }

    /// The formula the cell carries, when it states one.
    #[napi(getter)]
    pub fn formula(&self) -> Option<String> {
        self.inner.formula().map(ToOwned::to_owned)
    }

    /// The error the cell carries, such as `#DIV/0!`, when it is one.
    #[napi(getter)]
    pub fn error(&self) -> Option<String> {
        self.inner.error().map(ToOwned::to_owned)
    }

    /// Whether the cell holds no value.
    #[napi]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The cell's displayed text.
    #[napi]
    pub fn text(&self) -> String {
        self.inner.text().into_owned()
    }

    /// This cell carrying `formula`.
    #[napi]
    pub fn with_formula(&self, formula: String) -> Self {
        Self {
            inner: self.inner.clone().with_formula(formula),
        }
    }

    /// This cell as the error `error`.
    #[napi]
    pub fn with_error(&self, error: String) -> Self {
        Self {
            inner: self.inner.clone().with_error(error),
        }
    }

    /// This cell moved to `reference`.
    #[napi]
    pub fn at(&self, reference: CellRefInput<'_>) -> Result<Self> {
        Ok(Self {
            inner: self.inner.clone().at(cell_ref_from(reference)?),
        })
    }

    #[napi]
    pub fn equals(&self, other: &JsCell) -> bool {
        self.inner == other.inner
    }

    #[napi]
    pub fn clone(&self) -> Self {
        Clone::clone(self)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        format!("{}={}", self.inner.reference(), self.inner.text())
    }

    /// The cell's parts, as `Cell.fromParts` reads them.
    #[napi(js_name = "_partsNative", skip_typescript)]
    pub fn parts_native(&self) -> CellParts {
        CellParts {
            reference: self.inner.reference().to_string(),
            kind: self.kind().to_owned(),
            format: self.format().to_owned(),
            formula: self.inner.formula().map(ToOwned::to_owned),
            error: self.inner.error().map(ToOwned::to_owned),
        }
    }
}

/// A cell's parts beside its value.
#[napi(object)]
pub struct CellParts {
    pub reference: String,
    pub kind: String,
    pub format: String,
    pub formula: Option<String>,
    pub error: Option<String>,
}

/// One row of a sheet: its zero-based index and the cells it holds, in
/// column order.
#[napi(js_name = "ExcelRow")]
#[derive(Clone)]
pub struct JsRow {
    inner: Row,
}

#[napi]
impl JsRow {
    /// The zero-based row index.
    #[napi(getter)]
    pub fn index(&self) -> u32 {
        self.inner.index()
    }

    /// The cells of the row, in column order.
    #[napi]
    pub fn cells(&self) -> Vec<JsCell> {
        self.inner
            .cells()
            .cloned()
            .map(|inner| JsCell { inner })
            .collect()
    }

    /// The cell at the zero-based `column`, when the row holds one.
    #[napi]
    pub fn cell(&self, column: u32) -> Option<JsCell> {
        self.inner
            .cell(column)
            .cloned()
            .map(|inner| JsCell { inner })
    }

    /// How many cells the row holds.
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }
}

/// What a `Sheet` object stands for: a sheet of its own, or a live view of
/// the one a workbook holds under `name`.
enum Held {
    Own(Sheet),
    Shared { workbook: Shared, name: String },
}

/// One worksheet: every cell it states, by reference, with the rows read and
/// written through `Serie`.
///
/// A sheet handed out by a `Workbook` is a live view: what is set on it is
/// what the workbook writes. A sheet built on its own is put in a workbook
/// with `insertSheet`, after which the same object views it there.
#[napi(js_name = "Sheet")]
pub struct JsSheet {
    held: Held,
}

impl JsSheet {
    fn own(sheet: Sheet) -> Self {
        Self {
            held: Held::Own(sheet),
        }
    }

    fn shared(workbook: Shared, name: String) -> Self {
        Self {
            held: Held::Shared { workbook, name },
        }
    }

    fn read<R>(&self, read: impl FnOnce(&Sheet) -> Result<R>) -> Result<R> {
        match &self.held {
            Held::Own(sheet) => read(sheet),
            Held::Shared { workbook, name } => {
                let workbook = lock(workbook)?;
                read(workbook.sheet(name).map_err(napi_error)?)
            }
        }
    }

    fn write<R>(&mut self, write: impl FnOnce(&mut Sheet) -> Result<R>) -> Result<R> {
        match &mut self.held {
            Held::Own(sheet) => write(sheet),
            Held::Shared { workbook, name } => {
                let mut workbook = lock(workbook)?;
                write(workbook.sheet_mut(name).map_err(napi_error)?)
            }
        }
    }

    fn snapshot(&self) -> Result<Sheet> {
        self.read(|sheet| Ok(sheet.clone()))
    }
}

/// The settings a sheet is built with.
#[napi(object)]
pub struct SheetOptions {
    /// `1900` or `1904`; `1900` by default.
    pub date_system: Option<String>,
    /// `visible`, `hidden` or `veryHidden`; `visible` by default.
    pub state: Option<String>,
}

#[napi]
impl JsSheet {
    /// An empty sheet named `name`, `Sheet1` by default.
    #[napi(constructor)]
    pub fn new(name: Option<String>, options: Option<SheetOptions>) -> Result<Self> {
        let (date_system, state) = options
            .map(|options| (options.date_system, options.state))
            .unwrap_or_default();
        let sheet =
            Sheet::new(name.unwrap_or_else(|| yggdryl::excel::DEFAULT_SHEET_NAME.to_owned()))
                .map_err(napi_error)?
                .with_date_system(date_system_from(date_system)?)
                .with_state(sheet_state_from(state)?);
        Ok(Self::own(sheet))
    }

    /// A sheet named `name` laid out from the rows of `serie`, a header row
    /// of the column names first unless `header` is false.
    #[napi(factory, js_name = "_fromSerieNative", skip_typescript)]
    pub fn from_serie_native(name: String, serie: &JsSerie, header: Option<bool>) -> Result<Self> {
        Sheet::from_serie(name, &serie.inner, header.unwrap_or(true))
            .map(Self::own)
            .map_err(napi_error)
    }

    /// The sheet's name, its tab's when a workbook holds it.
    #[napi(getter)]
    pub fn name(&self) -> Result<String> {
        self.read(|sheet| Ok(sheet.name().to_owned()))
    }

    /// Rename the sheet; a workbook renames its tab and refuses a name
    /// another sheet has.
    #[napi(setter)]
    pub fn set_name(&mut self, name: String) -> Result<()> {
        match &mut self.held {
            Held::Own(sheet) => sheet.set_name(name).map_err(napi_error),
            Held::Shared {
                workbook,
                name: held,
            } => {
                lock(workbook)?
                    .rename_sheet(held, name.as_str())
                    .map_err(napi_error)?;
                *held = name;
                Ok(())
            }
        }
    }

    /// `visible`, `hidden` or `veryHidden`.
    #[napi(getter)]
    pub fn state(&self) -> Result<&'static str> {
        self.read(|sheet| Ok(sheet.state().as_str()))
    }

    #[napi(setter)]
    pub fn set_state(&mut self, state: String) -> Result<()> {
        let state = sheet_state_from(Some(state))?;
        self.write(|sheet| {
            sheet.set_state(state);
            Ok(())
        })
    }

    /// The date system serials are read and written in: `1900` or `1904`.
    #[napi(getter)]
    pub fn date_system(&self) -> Result<&'static str> {
        self.read(|sheet| Ok(sheet.date_system().as_str()))
    }

    /// How many rows hold a cell.
    #[napi(getter)]
    pub fn size(&self) -> Result<u32> {
        self.read(|sheet| Ok(u32::try_from(sheet.len()).unwrap_or(u32::MAX)))
    }

    /// Whether the sheet states no cell.
    #[napi]
    pub fn is_empty(&self) -> Result<bool> {
        self.read(|sheet| Ok(sheet.is_empty()))
    }

    /// The rectangle holding every stated cell, `null` for an empty sheet.
    #[napi(getter)]
    pub fn dimension(&self) -> Result<Option<JsCellRange>> {
        self.read(|sheet| Ok(sheet.dimension().map(|inner| JsCellRange { inner })))
    }

    /// The cell at `reference`, when the sheet states one.
    #[napi]
    pub fn cell(&self, reference: CellRefInput<'_>) -> Result<Option<JsCell>> {
        let reference = cell_ref_from(reference)?;
        self.read(|sheet| Ok(sheet.cell(reference).cloned().map(|inner| JsCell { inner })))
    }

    /// The value at `reference`, null where the sheet states no cell.
    #[napi]
    pub fn scalar(&self, reference: CellRefInput<'_>) -> Result<JsScalar> {
        let reference = cell_ref_from(reference)?;
        self.read(|sheet| Ok(JsScalar::from_core(sheet.scalar(reference))))
    }

    /// Whether the sheet states a cell at `reference`.
    #[napi]
    pub fn has(&self, reference: CellRefInput<'_>) -> Result<bool> {
        let reference = cell_ref_from(reference)?;
        self.read(|sheet| Ok(sheet.cell(reference).is_some()))
    }

    /// Put the native `value` at `reference`, answering the cell it
    /// replaced; the loader's `setCell` reads a JavaScript value into the
    /// `Scalar` this takes.
    #[napi(js_name = "_setCellNative", skip_typescript)]
    pub fn set_cell_native(
        &mut self,
        reference: CellRefInput<'_>,
        value: &JsScalar,
    ) -> Result<Option<JsCell>> {
        let reference = cell_ref_from(reference)?;
        let value = value.inner.clone();
        self.write(|sheet| {
            sheet
                .set_cell(reference, value)
                .map(|old| old.map(|inner| JsCell { inner }))
                .map_err(napi_error)
        })
    }

    /// Put `cell` where its reference says, answering the cell it replaced.
    #[napi]
    pub fn insert_cell(&mut self, cell: &JsCell) -> Result<Option<JsCell>> {
        let cell = cell.inner.clone();
        self.write(|sheet| {
            sheet
                .insert_cell(cell)
                .map(|old| old.map(|inner| JsCell { inner }))
                .map_err(napi_error)
        })
    }

    /// Take the cell at `reference` out, answering it.
    #[napi]
    pub fn remove_cell(&mut self, reference: CellRefInput<'_>) -> Result<Option<JsCell>> {
        let reference = cell_ref_from(reference)?;
        self.write(|sheet| Ok(sheet.remove_cell(reference).map(|inner| JsCell { inner })))
    }

    /// The rows holding at least one cell, in order.
    #[napi]
    pub fn rows(&self) -> Result<Vec<JsRow>> {
        self.read(|sheet| {
            Ok(sheet
                .rows()
                .map(|row| JsRow { inner: row.clone() })
                .collect())
        })
    }

    /// The row at the zero-based `index`, when it holds a cell.
    #[napi]
    pub fn row(&self, index: u32) -> Result<Option<JsRow>> {
        self.read(|sheet| Ok(sheet.row(index).map(|row| JsRow { inner: row.clone() })))
    }

    /// Every stated cell, row by row.
    #[napi]
    pub fn cells(&self) -> Result<Vec<JsCell>> {
        self.read(|sheet| {
            Ok(sheet
                .cells()
                .cloned()
                .map(|inner| JsCell { inner })
                .collect())
        })
    }

    /// The stated cells inside `range`, row by row.
    #[napi]
    pub fn cells_in(&self, range: CellRangeInput<'_>) -> Result<Vec<JsCell>> {
        let range = cell_range_from(range)?;
        self.read(|sheet| {
            Ok(sheet
                .cells_in(range)
                .cloned()
                .map(|inner| JsCell { inner })
                .collect())
        })
    }

    /// The stated cells of the zero-based `column`, top down.
    #[napi]
    pub fn column(&self, column: u32) -> Result<Vec<JsCell>> {
        self.read(|sheet| {
            Ok(sheet
                .column(column)
                .cloned()
                .map(|inner| JsCell { inner })
                .collect())
        })
    }

    /// The cells inside `range`, as a sheet of their own keeping their
    /// references.
    #[napi]
    pub fn slice(&self, range: CellRangeInput<'_>) -> Result<JsSheet> {
        let range = cell_range_from(range)?;
        self.read(|sheet| Ok(Self::own(sheet.slice(range))))
    }

    /// Open `count` empty rows at the zero-based row `at`, moving the rows
    /// from there down.
    #[napi]
    pub fn insert_rows(&mut self, at: u32, count: u32) -> Result<()> {
        self.write(|sheet| sheet.insert_rows(at, count).map_err(napi_error))
    }

    /// Drop the rows from `start` up to `stop`, moving the rows below up.
    #[napi]
    pub fn remove_rows(&mut self, start: u32, stop: u32) -> Result<()> {
        self.write(|sheet| {
            sheet.remove_rows(start..stop);
            Ok(())
        })
    }

    /// Lay the rows of `serie` out with their top-left cell at `anchor`, a
    /// header row of the column names first unless `header` is false.
    #[napi(js_name = "_writeSerieNative", skip_typescript)]
    pub fn write_serie_native(
        &mut self,
        anchor: CellRefInput<'_>,
        serie: &JsSerie,
        header: Option<bool>,
    ) -> Result<()> {
        let anchor = cell_ref_from(anchor)?;
        let serie = serie.inner.clone();
        self.write(|sheet| {
            sheet
                .write_serie(anchor, &serie, header.unwrap_or(true))
                .map_err(napi_error)
        })
    }

    /// Lay the rows of `serie` out under the last stated row.
    #[napi(js_name = "_extendFromSerieNative", skip_typescript)]
    pub fn extend_from_serie_native(&mut self, serie: &JsSerie) -> Result<()> {
        let serie = serie.inner.clone();
        self.write(|sheet| sheet.extend_from_serie(&serie).map_err(napi_error))
    }

    /// The sheet's rows as one record `Serie`: the first row naming the
    /// columns unless `header` is false, each column typed by its first
    /// value, or by `field` when one is declared.
    #[napi]
    pub fn into_serie(
        &self,
        field: Option<Either<ClassInstance<'_, JsField>, String>>,
        options: Option<IntoSerieOptions>,
    ) -> Result<JsSerie> {
        let options = options.unwrap_or_default();
        let cast = cast_options(options.safe, options.representation.as_deref())?;
        let field = field
            .map(|field| crate::iceberg::field_from_input(field))
            .transpose()?;
        let sheet = self.snapshot()?;
        sheet
            .into_serie(field.as_ref(), options.header.unwrap_or(true), cast)
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// Whether the cells equal another sheet's, name and state included.
    #[napi]
    pub fn equals(&self, other: &JsSheet) -> Result<bool> {
        Ok(self.snapshot()? == other.snapshot()?)
    }

    /// A sheet of its own holding a copy of the cells.
    #[napi]
    pub fn clone(&self) -> Result<JsSheet> {
        self.snapshot().map(Self::own)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> Result<String> {
        self.read(|sheet| Ok(format!("Sheet('{}', {} rows)", sheet.name(), sheet.len())))
    }
}

/// The settings `intoSerie` takes.
#[napi(object)]
#[derive(Default)]
pub struct IntoSerieOptions {
    /// Whether the first row names the columns; true by default.
    pub header: Option<bool>,
    /// Whether a cast may null a value it cannot convert; true by default.
    pub safe: Option<bool>,
    /// `value` or `bits`; `value` by default.
    pub representation: Option<String>,
}

/// Where a workbook opens from: a handle or location, or the package's
/// bytes.
pub type WorkbookInput<'a> = Either<Buffer, LocationInput<'a>>;

fn holder_from_input(value: WorkbookInput<'_>) -> Result<Holder> {
    match value {
        Either::A(bytes) => Ok(Holder::buffer(CoreBuffer::from_bytes(bytes.to_vec()))),
        // An in-memory buffer answers an identity rather than a location, so
        // it is copied once into a buffer of the workbook's own, as the
        // core's record doors copy one; a located handle is reopened at its
        // location.
        Either::B(napi::bindgen_prelude::Either6::A(handle))
            if matches!(handle.core(), Holder::Buffer(_)) =>
        {
            let bytes = handle.core().read_all_bytes().map_err(napi_error)?;
            let mut buffer = CoreBuffer::from_bytes(bytes);
            buffer.set_media_type(handle.core().media_type().clone());
            Ok(Holder::buffer(buffer))
        }
        Either::B(location) => located_from_input(location),
    }
}

/// An Office Open XML workbook: its sheets in tab order, each parsed on first
/// access, written back as one package.
#[napi(js_name = "Workbook")]
pub struct JsWorkbook {
    inner: Shared,
}

impl JsWorkbook {
    fn from_core(inner: Workbook) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Workbook>> {
        lock(&self.inner)
    }

    fn view(&self, name: &str) -> JsSheet {
        JsSheet::shared(Arc::clone(&self.inner), name.to_owned())
    }
}

#[napi]
impl JsWorkbook {
    /// An empty workbook with no sheet.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self::from_core(Workbook::new())
    }

    /// Open the package `value` holds: an `IOBase` handle, a location, or
    /// the package's bytes. The archive is indexed and the workbook
    /// documents read; no sheet is parsed until it is asked for. A handle
    /// holding nothing opens as an empty workbook.
    #[napi(factory)]
    pub fn open(value: WorkbookInput<'_>) -> Result<Self> {
        let holder = holder_from_input(value)?;
        Workbook::open(holder)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Open the package `bytes` hold.
    #[napi(factory)]
    pub fn from_bytes(bytes: Buffer) -> Result<Self> {
        Workbook::from_bytes(bytes.to_vec())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The date system serials are read and written in: `1900` or `1904`.
    #[napi(getter)]
    pub fn date_system(&self) -> Result<&'static str> {
        Ok(self.lock()?.date_system().as_str())
    }

    #[napi(setter)]
    pub fn set_date_system(&self, value: String) -> Result<()> {
        let system = date_system_from(Some(value))?;
        self.lock()?.set_date_system(system);
        Ok(())
    }

    /// Every sheet's name, in tab order.
    #[napi(getter)]
    pub fn sheet_names(&self) -> Result<Vec<String>> {
        Ok(self
            .lock()?
            .sheet_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect())
    }

    /// `worksheet`, `chartsheet` or `dialogsheet` for the sheet `name`,
    /// `null` when no sheet has it.
    #[napi]
    pub fn sheet_kind(&self, name: String) -> Result<Option<&'static str>> {
        Ok(self.lock()?.sheet_kind(&name).map(|kind| kind.as_str()))
    }

    /// How many sheets the workbook holds.
    #[napi(getter)]
    pub fn size(&self) -> Result<u32> {
        Ok(u32::try_from(self.lock()?.len()).unwrap_or(u32::MAX))
    }

    /// Whether the workbook holds no sheet.
    #[napi]
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.lock()?.is_empty())
    }

    /// Whether a sheet has the name, compared without case.
    #[napi]
    pub fn has(&self, name: String) -> Result<bool> {
        Ok(self.lock()?.sheet_kind(&name).is_some())
    }

    /// The worksheets, in tab order, as live views.
    #[napi]
    pub fn sheets(&self) -> Result<Vec<JsSheet>> {
        Ok(self
            .sheet_names()?
            .iter()
            .map(|name| self.view(name))
            .collect())
    }

    /// The worksheet `name`, parsed on first access, as a live view.
    #[napi]
    pub fn sheet(&self, name: String) -> Result<JsSheet> {
        self.lock()?.sheet(&name).map_err(napi_error)?;
        Ok(self.view(&name))
    }

    /// The worksheet `name` as a live view, `null` when no sheet has it.
    #[napi]
    pub fn get_sheet(&self, name: String) -> Result<Option<JsSheet>> {
        Ok(self
            .lock()?
            .get_sheet(&name)
            .map_err(napi_error)?
            .map(|_| self.view(&name)))
    }

    /// The sheet at the zero-based `index` in tab order, `null` past the
    /// last.
    #[napi]
    pub fn sheet_at(&self, index: u32) -> Result<Option<JsSheet>> {
        let name = {
            let workbook = self.lock()?;
            let Some(sheet) = workbook.sheet_at(index as usize).map_err(napi_error)? else {
                return Ok(None);
            };
            sheet.name().to_owned()
        };
        Ok(Some(self.view(&name)))
    }

    /// Add an empty worksheet named `name` after the last tab, as a live
    /// view; a name a sheet already has is refused.
    #[napi]
    pub fn add_sheet(&self, name: String) -> Result<JsSheet> {
        self.lock()?.add_sheet(name.as_str()).map_err(napi_error)?;
        Ok(self.view(&name))
    }

    /// Put `sheet` in the workbook: in place of the sheet of the same name,
    /// answering it, or after the last tab. The object then views the
    /// sheet in this workbook.
    #[napi]
    pub fn insert_sheet(&self, sheet: &mut JsSheet) -> Result<Option<JsSheet>> {
        let snapshot = sheet.snapshot()?;
        let name = snapshot.name().to_owned();
        let previous = self.lock()?.insert_sheet(snapshot).map_err(napi_error)?;
        sheet.held = Held::Shared {
            workbook: Arc::clone(&self.inner),
            name,
        };
        Ok(previous.map(JsSheet::own))
    }

    /// Take the sheet `name` out, answering it as a sheet of its own, or
    /// `null` when no sheet has it.
    #[napi]
    pub fn remove_sheet(&self, name: String) -> Result<Option<JsSheet>> {
        Ok(self
            .lock()?
            .remove_sheet(&name)
            .map_err(napi_error)?
            .map(JsSheet::own))
    }

    /// Rename the sheet `name` to `newName`, keeping its place.
    #[napi]
    pub fn rename_sheet(&self, name: String, new_name: String) -> Result<()> {
        self.lock()?
            .rename_sheet(&name, new_name)
            .map_err(napi_error)
    }

    /// The package as bytes: every sheet written, and every other part of
    /// an opened package kept as it was.
    #[napi]
    pub fn into_bytes(&self) -> Result<Buffer> {
        self.lock()?
            .into_bytes()
            .map(Into::into)
            .map_err(napi_error)
    }

    /// Write the package into `target`: an `IOBase` handle or a location.
    #[napi]
    pub fn write_into(&self, target: LocationInput<'_>) -> Result<()> {
        let bytes = self.lock()?.into_bytes().map_err(napi_error)?;
        let mut holder = located_from_input(target)?;
        holder.write_all_bytes(&bytes).map_err(napi_error)
    }

    /// Calls the package's handle has answered so far, in `IOBase` calls.
    #[napi(getter)]
    pub fn handle_reads(&self) -> Result<f64> {
        Ok(self.lock()?.handle_reads() as f64)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> Result<String> {
        Ok(format!(
            "Workbook([{}])",
            self.sheet_names()?
                .iter()
                .map(|name| format!("'{name}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// The grid's bounds and the crate's defaults, for the loader.
#[napi(object)]
pub struct ExcelLimits {
    pub max_rows: u32,
    pub max_columns: u32,
    pub max_cell_text: u32,
    pub max_sheet_name: u32,
    pub default_sheet_name: String,
}

/// The grid's bounds and the crate's defaults.
#[napi(js_name = "_excelLimitsNative", skip_typescript)]
pub fn excel_limits_native() -> ExcelLimits {
    ExcelLimits {
        max_rows: yggdryl::excel::MAX_ROWS,
        max_columns: yggdryl::excel::MAX_COLUMNS,
        max_cell_text: u32::try_from(yggdryl::excel::MAX_CELL_TEXT).unwrap_or(u32::MAX),
        max_sheet_name: u32::try_from(yggdryl::excel::MAX_SHEET_NAME).unwrap_or(u32::MAX),
        default_sheet_name: yggdryl::excel::DEFAULT_SHEET_NAME.to_owned(),
    }
}

impl Default for JsWorkbook {
    fn default() -> Self {
        Self::new()
    }
}
