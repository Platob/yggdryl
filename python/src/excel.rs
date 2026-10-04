//! Python views over the Excel workbook, its sheets and cells.
//!
//! Every value here is the core's: a `Workbook` is [`yggdryl::excel::Workbook`]
//! behind a lock two views may share, a `Sheet` is either a sheet of its own or
//! a live view of one the workbook holds, and a `Cell`, a `CellRef` and a
//! `CellRange` are the core values, frozen. A sheet's rows cross as `Serie`
//! and Arrow through the core's own doors; nothing here reads a cell's bytes.

use std::sync::{Arc, Mutex, MutexGuard};

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyIterator, PyList, PyTuple};
use yggdryl::excel::{
    Cell, CellKind, CellRange, CellRef, DateSystem, ExcelError, Formula, Row, Sheet, SheetState,
    Workbook,
};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::{IOBase, Scalar};

use crate::iobase::PyIOBase;
use crate::scalar::{PyScalar, as_py, from_py};
use crate::serie::{described, serie_from_py};
use crate::{cast_options, compare, python_hash, value_error};

/// A workbook two views may hold: the `Workbook` class and every live sheet
/// view it hands out.
type Shared = Arc<Mutex<Workbook>>;

/// Lock a shared workbook, naming a poisoned one.
fn lock(shared: &Shared) -> PyResult<MutexGuard<'_, Workbook>> {
    shared
        .lock()
        .map_err(|_| PyRuntimeError::new_err("the workbook was poisoned by a panic"))
}

/// Map a core refusal to the Python exception its kind names: an absent
/// sheet is a `KeyError`, everything else the `ValueError` every core
/// refusal is.
fn excel_error(error: yggdryl::Error) -> PyErr {
    match error {
        yggdryl::Error::Absent { .. } => PyKeyError::new_err(error.to_string()),
        other => value_error(other),
    }
}

/// Read a row or column number: a non-negative Python integer.
fn index_from(value: &Bound<'_, PyAny>, what: &str) -> PyResult<u32> {
    if value.is_instance_of::<pyo3::types::PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{what} must be an integer, not bool"
        )));
    }
    value.extract::<u32>().map_err(|_| {
        PyTypeError::new_err(format!(
            "{what} must be a non-negative integer below 2^32, got {}",
            value
                .get_type()
                .name()
                .map_or_else(|_| "?".to_owned(), |name| name.to_string())
        ))
    })
}

/// Read a cell reference: a `CellRef`, its `A1` text, or a `(row, column)`
/// pair of zero-based integers.
pub(crate) fn cell_ref_from(value: &Bound<'_, PyAny>) -> PyResult<CellRef> {
    if let Ok(reference) = value.extract::<PyRef<'_, PyCellRef>>() {
        return Ok(reference.inner);
    }
    if let Ok(text) = value.extract::<&str>() {
        return text.parse::<CellRef>().map_err(value_error);
    }
    if let Ok(pair) = value.cast::<PyTuple>()
        && pair.len() == 2
    {
        let row = index_from(&pair.get_item(0)?, "row")?;
        let column = index_from(&pair.get_item(1)?, "column")?;
        return CellRef::new(row, column)
            .require_in_grid()
            .map_err(value_error);
    }
    Err(PyTypeError::new_err(format!(
        "expected a CellRef, an A1 reference or a (row, column) pair, got {}",
        value.get_type().name()?
    )))
}

/// Read a cell range: a `CellRange`, its `A1:C3` text, or a pair of cell
/// references.
pub(crate) fn cell_range_from(value: &Bound<'_, PyAny>) -> PyResult<CellRange> {
    if let Ok(range) = value.extract::<PyRef<'_, PyCellRange>>() {
        return Ok(range.inner);
    }
    if let Ok(text) = value.extract::<&str>() {
        return text.parse::<CellRange>().map_err(value_error);
    }
    if let Ok(pair) = value.cast::<PyTuple>()
        && pair.len() == 2
    {
        let first = cell_ref_from(&pair.get_item(0)?)?;
        let second = cell_ref_from(&pair.get_item(1)?)?;
        return Ok(CellRange::new(first, second));
    }
    Err(PyTypeError::new_err(format!(
        "expected a CellRange, an A1:C3 reference or a pair of cell references, got {}",
        value.get_type().name()?
    )))
}

/// Read a date system by name: `1900` or `1904`.
fn date_system_from(value: &str) -> PyResult<DateSystem> {
    value.parse().map_err(value_error)
}

/// Read a sheet state by name: `visible`, `hidden` or `veryHidden`.
fn sheet_state_from(value: &str) -> PyResult<SheetState> {
    SheetState::from_attribute(Some(value)).map_err(value_error)
}

/// The type name a message names.
fn type_name(value: &Bound<'_, PyAny>) -> String {
    value
        .get_type()
        .name()
        .map_or_else(|_| "?".to_owned(), |name| name.to_string())
}

/// A hand-back list, as the iterator Python asks a container for.
fn list_iterator<'py, T: IntoPyObject<'py>>(
    py: Python<'py>,
    items: Vec<T>,
) -> PyResult<Bound<'py, PyIterator>> {
    PyList::new(py, items)?.try_iter()
}

/// One cell's position: a zero-based row and column, spelled `A1`.
#[pyclass(
    name = "CellRef",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyCellRef {
    pub(crate) inner: CellRef,
}

impl PyCellRef {
    pub(crate) const fn from_inner(inner: CellRef) -> Self {
        Self { inner }
    }

    fn stable_hash(&self) -> u64 {
        Scalar::from_sequence([
            Scalar::from(i64::from(self.inner.row())),
            Scalar::from(i64::from(self.inner.column())),
        ])
        .stable_hash()
    }
}

#[pymethods]
impl PyCellRef {
    /// Read a reference: `A1` text with or without `$` anchors, a
    /// `(row, column)` pair of zero-based integers, or another `CellRef`.
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        cell_ref_from(value).map(Self::from_inner)
    }

    /// The zero-based row.
    #[getter]
    fn row(&self) -> u32 {
        self.inner.row()
    }

    /// The zero-based column.
    #[getter]
    fn column(&self) -> u32 {
        self.inner.column()
    }

    /// Whether the reference lies inside the 1,048,576 by 16,384 grid.
    fn is_in_grid(&self) -> bool {
        self.inner.is_in_grid()
    }

    /// The letters of the zero-based `column`: `0` is `A`, `26` is `AA`.
    #[staticmethod]
    fn column_name(column: u32) -> String {
        CellRef::column_name(column).to_string()
    }

    /// The zero-based column the `letters` spell, `None` for anything that
    /// is not letters or lies past `XFD`.
    #[staticmethod]
    fn column_index(letters: &str) -> Option<u32> {
        CellRef::column_index(letters)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("CellRef('{}')", self.inner)
    }

    fn __hash__(&self) -> isize {
        python_hash(self.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        Self { inner: self.inner }
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        Self { inner: self.inner }
    }
}

/// A rectangle of cells, spelled `A1:C3`, `A:C`, `3:5` or `A3:F`.
#[pyclass(
    name = "CellRange",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyCellRange {
    pub(crate) inner: CellRange,
}

impl PyCellRange {
    pub(crate) const fn from_inner(inner: CellRange) -> Self {
        Self { inner }
    }

    fn stable_hash(&self) -> u64 {
        Scalar::from_sequence([
            Scalar::from(i64::from(self.inner.start().row())),
            Scalar::from(i64::from(self.inner.start().column())),
            Scalar::from(i64::from(self.inner.end().row())),
            Scalar::from(i64::from(self.inner.end().column())),
        ])
        .stable_hash()
    }
}

#[pymethods]
impl PyCellRange {
    /// Read a range: `A1:C3` text, an open `A:C`, `3:5` or `A3:F`, a pair of
    /// cell references in any order, or another `CellRange`.
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        cell_range_from(value).map(Self::from_inner)
    }

    /// The whole grid.
    #[staticmethod]
    fn all() -> Self {
        Self::from_inner(CellRange::all())
    }

    /// The top-left cell.
    #[getter]
    fn start(&self) -> PyCellRef {
        PyCellRef::from_inner(self.inner.start())
    }

    /// The bottom-right cell.
    #[getter]
    fn end(&self) -> PyCellRef {
        PyCellRef::from_inner(self.inner.end())
    }

    /// How many rows the range spans; a method, as `IOBase.row_size` is.
    fn row_size(&self) -> u32 {
        self.inner.row_size()
    }

    /// How many columns the range spans; a method, as
    /// `IOBase.column_size` is.
    fn column_size(&self) -> u32 {
        self.inner.column_size()
    }

    /// Whether the range runs to the last row of the grid.
    fn is_row_open(&self) -> bool {
        self.inner.is_row_open()
    }

    /// Whether the range runs to the last column of the grid.
    fn is_column_open(&self) -> bool {
        self.inner.is_column_open()
    }

    /// Whether `cell` lies inside the range.
    fn contains(&self, cell: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.inner.contains(cell_ref_from(cell)?))
    }

    /// Whether the zero-based `row` lies inside the range.
    fn contains_row(&self, row: u32) -> bool {
        self.inner.contains_row(row)
    }

    /// Whether the zero-based `column` lies inside the range.
    fn contains_column(&self, column: u32) -> bool {
        self.inner.contains_column(column)
    }

    fn __contains__(&self, cell: &Bound<'_, PyAny>) -> PyResult<bool> {
        self.contains(cell)
    }

    /// Every cell of the range, row by row.
    fn __iter__(&self) -> PyCellRefIterator {
        PyCellRefIterator {
            inner: Box::new(self.inner.cells()),
        }
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("CellRange('{}')", self.inner)
    }

    fn __hash__(&self) -> isize {
        python_hash(self.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        Self { inner: self.inner }
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        Self { inner: self.inner }
    }
}

/// The cells of a range, one at a time, row by row.
#[pyclass(
    name = "CellRefIterator",
    module = "yggdryl._native",
    unsendable,
    skip_from_py_object
)]
pub(crate) struct PyCellRefIterator {
    inner: Box<dyn Iterator<Item = CellRef>>,
}

#[pymethods]
impl PyCellRefIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<PyCellRef> {
        self.inner.next().map(PyCellRef::from_inner)
    }
}

/// One cell: its reference, what kind of content it states, the number
/// format its style declares, its value, and the formula or error it carries.
#[pyclass(name = "Cell", module = "yggdryl._native", frozen, skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyCell {
    pub(crate) inner: Cell,
}

impl PyCell {
    pub(crate) const fn from_inner(inner: Cell) -> Self {
        Self { inner }
    }

    fn stable_hash(&self) -> u64 {
        Scalar::from_sequence([
            Scalar::from(self.inner.reference().to_string().as_str()),
            Scalar::from(self.inner.kind().as_str().unwrap_or("n")),
            Scalar::from(self.inner.format().as_str()),
            self.inner.value().clone(),
            self.formula().map_or(Scalar::Null, Scalar::from),
            self.error().map_or(Scalar::Null, Scalar::from),
        ])
        .stable_hash()
    }
}

/// Read a cell's value: a `Scalar`, or anything `Scalar.from_` reads.
fn scalar_from(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    if let Ok(scalar) = value.extract::<PyRef<'_, PyScalar>>() {
        return Ok(scalar.inner.clone());
    }
    from_py(value)
}

#[pymethods]
impl PyCell {
    /// A cell holding `value` at `reference`: text is a string cell, a bool a
    /// boolean, a number a number, a date, time, datetime or duration a
    /// serial under `date_system`'s style, `None` an empty cell, and a
    /// nested value its JSON text.
    #[new]
    #[pyo3(signature = (reference, value, *, date_system = "1900"))]
    fn new(
        reference: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        date_system: &str,
    ) -> PyResult<Self> {
        let reference = cell_ref_from(reference)?;
        let system = date_system_from(date_system)?;
        Cell::from_scalar(reference, scalar_from(value)?, system)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Rebuild a pickled cell from its parts.
    #[staticmethod]
    fn _from_parts(
        reference: &str,
        kind: &str,
        format: &str,
        value: &PyScalar,
        formula: Option<&str>,
        error: Option<&str>,
    ) -> PyResult<Self> {
        let mut cell = Cell::new(
            reference.parse().map_err(value_error)?,
            CellKind::from_attribute(kind).map_err(value_error)?,
            format.parse().map_err(value_error)?,
            error.map_or_else(|| value.inner.clone(), Scalar::from),
        );
        if let Some(formula) = formula {
            let formula = Formula::from_file(formula, cell.reference());
            cell = cell.with_formula(formula);
        }
        if let Some(error) = error {
            cell = cell.with_error(ExcelError::from_text(error));
        }
        Ok(Self::from_inner(cell))
    }

    /// Where the cell is.
    #[getter]
    fn reference(&self) -> PyCellRef {
        PyCellRef::from_inner(self.inner.reference())
    }

    /// The zero-based row.
    #[getter]
    fn row(&self) -> u32 {
        self.inner.row()
    }

    /// The zero-based column.
    #[getter]
    fn column(&self) -> u32 {
        self.inner.column()
    }

    /// The `t` attribute the cell states: `n`, `s`, `str`, `inlineStr`,
    /// `b`, `d` or `e`.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind().as_str().unwrap_or("n")
    }

    /// The number format the cell's style classifies as: `general`,
    /// `date`, `time`, `datetime`, `datetime_fraction` or `duration`.
    #[getter]
    fn format(&self) -> &'static str {
        self.inner.format().as_str()
    }

    /// The value the cell holds, as the core reads it.
    #[getter]
    fn value(&self) -> PyScalar {
        PyScalar::from_inner(self.inner.value().clone())
    }

    /// The value as a native Python value.
    fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        as_py(py, self.inner.value())
    }

    /// The formula the cell carries, when it states one.
    #[getter]
    fn formula(&self) -> Option<String> {
        self.inner
            .formula()
            .map(|formula| formula.at(self.inner.reference()).to_string())
    }

    /// The error the cell carries, such as `#DIV/0!`, when it is one.
    #[getter]
    fn error(&self) -> Option<&str> {
        self.inner.error().map(|_| self.inner.error_text())
    }

    /// Whether the cell holds no value.
    fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The cell's displayed text.
    fn text(&self) -> String {
        self.inner.text().into_owned()
    }

    /// This cell carrying `formula`.
    fn with_formula(&self, formula: &str) -> PyResult<Self> {
        let formula = Formula::from_entry(formula, self.inner.reference()).map_err(value_error)?;
        Ok(Self::from_inner(self.inner.clone().with_formula(formula)))
    }

    /// This cell as the error `error`.
    fn with_error(&self, error: &str) -> Self {
        let mut cell = Cell::new(
            self.inner.reference(),
            self.inner.kind(),
            self.inner.format(),
            Scalar::from(error),
        )
        .with_style(self.inner.style());
        if let Some(formula) = self.inner.formula() {
            cell = cell.with_formula(formula.clone());
        }
        Self::from_inner(cell.with_error(ExcelError::from_text(error)))
    }

    /// This cell moved to `reference`.
    fn at(&self, reference: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self::from_inner(
            self.inner.clone().at(cell_ref_from(reference)?),
        ))
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Cell('{}', {})",
            self.inner.reference(),
            self.inner.value().into_json().map_err(value_error)?
        ))
    }

    fn __hash__(&self) -> isize {
        python_hash(self.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        let equal = self.inner == other.inner;
        Ok(match operation {
            CompareOp::Eq => equal.into_pyobject(py)?.to_owned().into_any().unbind(),
            CompareOp::Ne => (!equal).into_pyobject(py)?.to_owned().into_any().unbind(),
            _ => py.NotImplemented(),
        })
    }

    #[allow(clippy::type_complexity)]
    fn __reduce__(
        &self,
        py: Python<'_>,
    ) -> PyResult<(
        Py<PyAny>,
        (
            String,
            &'static str,
            &'static str,
            PyScalar,
            Option<String>,
            Option<String>,
        ),
    )> {
        let restore = py.get_type::<Self>().getattr("_from_parts")?.unbind();
        Ok((
            restore,
            (
                self.inner.reference().to_string(),
                self.kind(),
                self.format(),
                self.value(),
                self.formula(),
                self.error().map(ToOwned::to_owned),
            ),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One row of a sheet: its zero-based index and the cells it holds, in
/// column order.
#[pyclass(
    name = "ExcelRow",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyRow {
    inner: Row,
}

#[pymethods]
impl PyRow {
    /// The zero-based row index.
    #[getter]
    fn index(&self) -> u32 {
        self.inner.index()
    }

    /// The cells of the row, in column order.
    fn cells(&self) -> Vec<PyCell> {
        self.inner
            .cells()
            .cloned()
            .map(PyCell::from_inner)
            .collect()
    }

    /// The cell at the zero-based `column`, when the row holds one.
    fn cell(&self, column: u32) -> Option<PyCell> {
        self.inner.cell(column).cloned().map(PyCell::from_inner)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyIterator>> {
        list_iterator(py, self.cells())
    }

    fn __repr__(&self) -> String {
        format!(
            "ExcelRow({}, {} cells)",
            self.inner.index(),
            self.inner.len()
        )
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
/// with `insert_sheet`, after which the same object views it there.
#[pyclass(name = "Sheet", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PySheet {
    held: Held,
}

impl PySheet {
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

    /// Read through to the sheet, wherever it lives.
    fn read<R>(&self, read: impl FnOnce(&Sheet) -> PyResult<R>) -> PyResult<R> {
        match &self.held {
            Held::Own(sheet) => read(sheet),
            Held::Shared { workbook, name } => {
                let workbook = lock(workbook)?;
                read(workbook.sheet(name).map_err(excel_error)?)
            }
        }
    }

    /// Write through to the sheet, wherever it lives.
    fn write<R>(&mut self, write: impl FnOnce(&mut Sheet) -> PyResult<R>) -> PyResult<R> {
        match &mut self.held {
            Held::Own(sheet) => write(sheet),
            Held::Shared { workbook, name } => {
                let mut workbook = lock(workbook)?;
                write(workbook.sheet_mut(name).map_err(excel_error)?)
            }
        }
    }

    /// Structural edits follow the workbook's package-wide reference graph.
    fn edit(
        &mut self,
        edit: impl FnOnce(&mut Workbook, &str) -> yggdryl::Result<()>,
    ) -> PyResult<()> {
        match &mut self.held {
            Held::Own(sheet) => {
                let name = sheet.name().to_owned();
                let mut workbook = Workbook::new();
                workbook.set_date_system(sheet.date_system());
                workbook.insert_sheet(sheet.clone()).map_err(excel_error)?;
                edit(&mut workbook, &name).map_err(excel_error)?;
                *sheet = workbook.sheet(&name).map_err(excel_error)?.clone();
                Ok(())
            }
            Held::Shared { workbook, name } => {
                let mut workbook = lock(workbook)?;
                edit(&mut workbook, name).map_err(excel_error)
            }
        }
    }

    /// A copy of the sheet as it stands.
    fn snapshot(&self) -> PyResult<Sheet> {
        self.read(|sheet| Ok(sheet.clone()))
    }
}

/// Read the `Serie` a Python value lays out as.
fn serie_from(value: &Bound<'_, PyAny>) -> PyResult<yggdryl::Serie> {
    serie_from_py(value, None, yggdryl::ArrowCastOptions::new())
}

#[pymethods]
impl PySheet {
    /// An empty sheet named `name`, under `date_system` and `state`.
    #[new]
    #[pyo3(signature = (name = "Sheet1", *, date_system = "1900", state = "visible"))]
    fn new(name: &str, date_system: &str, state: &str) -> PyResult<Self> {
        let sheet = Sheet::new(name)
            .map_err(value_error)?
            .with_date_system(date_system_from(date_system)?)
            .with_state(sheet_state_from(state)?);
        Ok(Self::own(sheet))
    }

    /// Rebuild a pickled sheet from its parts.
    #[staticmethod]
    fn _from_parts(
        name: &str,
        date_system: &str,
        state: &str,
        cells: Vec<PyRef<'_, PyCell>>,
    ) -> PyResult<Self> {
        let mut sheet = Self::new(name, date_system, state)?;
        for cell in cells {
            sheet.write(|sheet| sheet.insert_cell(cell.inner.clone()).map_err(value_error))?;
        }
        Ok(sheet)
    }

    /// The sheet's name, its tab's when a workbook holds it.
    #[getter]
    fn name(&self) -> PyResult<String> {
        self.read(|sheet| Ok(sheet.name().to_owned()))
    }

    /// Rename the sheet; a workbook renames its tab and refuses a name
    /// another sheet has.
    #[setter]
    fn set_name(&mut self, name: &str) -> PyResult<()> {
        match &mut self.held {
            Held::Own(sheet) => sheet.set_name(name).map_err(value_error),
            Held::Shared {
                workbook,
                name: held,
            } => {
                lock(workbook)?
                    .rename_sheet(held, name)
                    .map_err(excel_error)?;
                name.clone_into(held);
                Ok(())
            }
        }
    }

    /// `visible`, `hidden` or `veryHidden`.
    #[getter]
    fn state(&self) -> PyResult<&'static str> {
        self.read(|sheet| Ok(sheet.state().as_str()))
    }

    #[setter]
    fn set_state(&mut self, state: &str) -> PyResult<()> {
        let state = sheet_state_from(state)?;
        self.write(|sheet| {
            sheet.set_state(state);
            Ok(())
        })
    }

    /// The date system serials are read and written in: `1900` or `1904`.
    #[getter]
    fn date_system(&self) -> PyResult<&'static str> {
        self.read(|sheet| Ok(sheet.date_system().as_str()))
    }

    /// How many rows hold a cell.
    fn __len__(&self) -> PyResult<usize> {
        self.read(|sheet| Ok(sheet.len()))
    }

    /// Whether the sheet states no cell.
    fn is_empty(&self) -> PyResult<bool> {
        self.read(|sheet| Ok(sheet.is_empty()))
    }

    /// The rectangle holding every stated cell, `None` for an empty sheet.
    #[getter]
    fn dimension(&self) -> PyResult<Option<PyCellRange>> {
        self.read(|sheet| Ok(sheet.dimension().map(PyCellRange::from_inner)))
    }

    /// The cell at `reference`, when the sheet states one.
    fn cell(&self, reference: &Bound<'_, PyAny>) -> PyResult<Option<PyCell>> {
        let reference = cell_ref_from(reference)?;
        self.read(|sheet| Ok(sheet.cell(reference).cloned().map(PyCell::from_inner)))
    }

    /// The value at `reference`, null where the sheet states no cell.
    fn scalar(&self, reference: &Bound<'_, PyAny>) -> PyResult<PyScalar> {
        let reference = cell_ref_from(reference)?;
        self.read(|sheet| Ok(PyScalar::from_inner(sheet.scalar(reference))))
    }

    /// Put `value` at `reference`, answering the cell it replaced; a `Cell`
    /// is put as it is, moved to `reference`.
    fn set_cell(
        &mut self,
        reference: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Option<PyCell>> {
        let reference = cell_ref_from(reference)?;
        if let Ok(cell) = value.extract::<PyRef<'_, PyCell>>() {
            let cell = cell.inner.clone().at(reference);
            return self.write(|sheet| {
                sheet
                    .insert_cell(cell)
                    .map_err(value_error)
                    .map(|old| old.map(PyCell::from_inner))
            });
        }
        let value = scalar_from(value)?;
        self.write(|sheet| {
            sheet
                .set_cell(reference, value)
                .map_err(value_error)
                .map(|old| old.map(PyCell::from_inner))
        })
    }

    /// Put `cell` where its reference says, answering the cell it replaced.
    fn insert_cell(&mut self, cell: &PyCell) -> PyResult<Option<PyCell>> {
        let cell = cell.inner.clone();
        self.write(|sheet| {
            sheet
                .insert_cell(cell)
                .map_err(value_error)
                .map(|old| old.map(PyCell::from_inner))
        })
    }

    /// Take the cell at `reference` out, answering it.
    fn remove_cell(&mut self, reference: &Bound<'_, PyAny>) -> PyResult<Option<PyCell>> {
        let reference = cell_ref_from(reference)?;
        self.write(|sheet| Ok(sheet.remove_cell(reference).map(PyCell::from_inner)))
    }

    /// `sheet["A1"]` is the cell there or `None`; `sheet["A1:C3"]` the
    /// cells inside the range as a sheet of their own.
    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let py = key.py();
        if let Ok(reference) = cell_ref_from(key) {
            return Ok(self
                .read(|sheet| Ok(sheet.cell(reference).cloned().map(PyCell::from_inner)))?
                .into_pyobject(py)?
                .into_any()
                .unbind());
        }
        let range = cell_range_from(key).map_err(|_| {
            PyTypeError::new_err(format!(
                "expected a cell reference or a cell range, got {}",
                type_name(key)
            ))
        })?;
        let sliced = self.read(|sheet| Ok(sheet.slice(range)))?;
        Ok(Py::new(py, Self::own(sliced))?.into_any())
    }

    fn __setitem__(
        &mut self,
        reference: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        self.set_cell(reference, value).map(drop)
    }

    fn __delitem__(&mut self, reference: &Bound<'_, PyAny>) -> PyResult<()> {
        let reference = cell_ref_from(reference)?;
        self.write(|sheet| {
            sheet.remove_cell(reference).map_or_else(
                || Err(PyKeyError::new_err(reference.to_string())),
                |_| Ok(()),
            )
        })
    }

    fn __contains__(&self, reference: &Bound<'_, PyAny>) -> PyResult<bool> {
        let reference = cell_ref_from(reference)?;
        self.read(|sheet| Ok(sheet.cell(reference).is_some()))
    }

    /// The rows holding at least one cell, in order.
    fn rows(&self) -> PyResult<Vec<PyRow>> {
        self.read(|sheet| {
            Ok(sheet
                .rows()
                .map(|row| PyRow { inner: row.clone() })
                .collect())
        })
    }

    /// The row at the zero-based `index`, when it holds a cell.
    fn row(&self, index: u32) -> PyResult<Option<PyRow>> {
        self.read(|sheet| Ok(sheet.row(index).map(|row| PyRow { inner: row.clone() })))
    }

    /// Every stated cell, row by row.
    fn cells(&self) -> PyResult<Vec<PyCell>> {
        self.read(|sheet| Ok(sheet.cells().cloned().map(PyCell::from_inner).collect()))
    }

    /// The stated cells inside `range`, row by row.
    fn cells_in(&self, range: &Bound<'_, PyAny>) -> PyResult<Vec<PyCell>> {
        let range = cell_range_from(range)?;
        self.read(|sheet| {
            Ok(sheet
                .cells_in(range)
                .cloned()
                .map(PyCell::from_inner)
                .collect())
        })
    }

    /// The stated cells of the zero-based `column`, top down.
    fn column(&self, column: u32) -> PyResult<Vec<PyCell>> {
        self.read(|sheet| {
            Ok(sheet
                .column(column)
                .cloned()
                .map(PyCell::from_inner)
                .collect())
        })
    }

    /// The cells inside `range`, as a sheet of their own keeping their
    /// references.
    fn slice(&self, range: &Bound<'_, PyAny>) -> PyResult<Self> {
        let range = cell_range_from(range)?;
        self.read(|sheet| Ok(Self::own(sheet.slice(range))))
    }

    /// Open `count` empty rows at the zero-based row `at`, moving the rows
    /// from there down.
    fn insert_rows(&mut self, at: u32, count: u32) -> PyResult<()> {
        self.edit(|workbook, name| workbook.insert_rows(name, at, count))
    }

    /// Drop the rows from `start` up to `stop`, moving the rows below up.
    fn remove_rows(&mut self, start: u32, stop: u32) -> PyResult<()> {
        self.edit(|workbook, name| workbook.remove_rows(name, start..stop))
    }

    /// A sheet named `name` laid out from the rows of `value` - anything
    /// `Serie.from_` reads - a header row of the column names first when
    /// `header`.
    #[staticmethod]
    #[pyo3(signature = (name, value, *, header = true))]
    fn from_serie(name: &str, value: &Bound<'_, PyAny>, header: bool) -> PyResult<Self> {
        let serie = serie_from(value)?;
        Sheet::from_serie(name, &serie, header.into())
            .map(Self::own)
            .map_err(value_error)
    }

    /// Lay the rows of `value` out with their top-left cell at `anchor`, a
    /// header row of the column names first when `header`.
    #[pyo3(signature = (anchor, value, *, header = true))]
    fn write_serie(
        &mut self,
        anchor: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        header: bool,
    ) -> PyResult<()> {
        let anchor = cell_ref_from(anchor)?;
        let serie = serie_from(value)?;
        self.write(|sheet| {
            sheet
                .write_serie(anchor, &serie, header.into())
                .map_err(value_error)
        })
    }

    /// Lay the rows of `value` out under the last stated row.
    fn extend_from_serie(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let serie = serie_from(value)?;
        self.write(|sheet| sheet.extend_from_serie(&serie).map_err(value_error))
    }

    /// The sheet's rows as one record `Serie`: the first row naming the
    /// columns when `header`, each column typed by its first value, or by
    /// `field` when one is declared.
    #[allow(clippy::wrong_self_convention)] // Binding `into_*` methods do not consume wrappers.
    #[pyo3(signature = (field = None, *, header = true, safe = true, representation = "value"))]
    fn into_serie(
        &self,
        py: Python<'_>,
        field: Option<&Bound<'_, PyAny>>,
        header: bool,
        safe: bool,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, representation)?;
        let field = field
            .map(|field| {
                crate::iomedia::core_root_field_from_value(field, yggdryl::media::DEFAULT_ROOT_NAME)
            })
            .transpose()?;
        let sheet = self.snapshot()?;
        let serie = py
            .detach(move || sheet.into_serie(field.as_ref(), header.into(), options))
            .map_err(value_error)?;
        described(py, serie)
    }

    fn __repr__(&self) -> PyResult<String> {
        self.read(|sheet| Ok(format!("Sheet('{}', {} rows)", sheet.name(), sheet.len())))
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        let equal = self.snapshot()? == other.snapshot()?;
        Ok(match operation {
            CompareOp::Eq => equal.into_pyobject(py)?.to_owned().into_any().unbind(),
            CompareOp::Ne => (!equal).into_pyobject(py)?.to_owned().into_any().unbind(),
            _ => py.NotImplemented(),
        })
    }

    #[allow(clippy::type_complexity)]
    fn __reduce__(
        &self,
        py: Python<'_>,
    ) -> PyResult<(Py<PyAny>, (String, &'static str, &'static str, Vec<PyCell>))> {
        let restore = py.get_type::<Self>().getattr("_from_parts")?.unbind();
        self.read(|sheet| {
            Ok((
                restore,
                (
                    sheet.name().to_owned(),
                    sheet.date_system().as_str(),
                    sheet.state().as_str(),
                    sheet.cells().cloned().map(PyCell::from_inner).collect(),
                ),
            ))
        })
    }

    fn __copy__(&self) -> PyResult<Self> {
        self.snapshot().map(Self::own)
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.snapshot().map(Self::own)
    }
}

/// Read a handle out of what Python names one with: an `IOBase`, or anything
/// its constructor reads - a path, a URL, bytes.
fn holder_from_value(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Holder> {
    if let Ok(handle) = value.extract::<PyRef<'_, PyIOBase>>() {
        return owned_holder(&handle);
    }
    // A string names a location rather than holding the bytes it spells.
    if !value.is_instance_of::<pyo3::types::PyString>()
        && let Ok(bytes) = value.extract::<Vec<u8>>()
    {
        return Ok(Holder::buffer(Buffer::from_bytes(bytes)));
    }
    let built = py
        .import("yggdryl._native")?
        .getattr("IOBase")?
        .call1((value,))?;
    let mut handle = built.extract::<PyRefMut<'_, PyIOBase>>()?;
    handle.take()
}

/// A handle of the binding's own over the resource `handle` holds: rebuilt
/// at its location, or, for an in-memory buffer - which answers an identity
/// rather than a location - a copy of its bytes.
fn owned_holder(handle: &PyIOBase) -> PyResult<Holder> {
    let inner = handle.inner()?;
    if !matches!(inner, Holder::Buffer(_)) {
        return handle.rebuilt();
    }
    let bytes = inner.read_all_bytes().map_err(value_error)?;
    let mut buffer = Buffer::from_bytes(bytes);
    buffer.set_media_type(inner.media_type().clone());
    Ok(Holder::buffer(buffer))
}

/// An Office Open XML workbook: its sheets in tab order, each parsed on first
/// access, written back as one package.
#[pyclass(name = "Workbook", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyWorkbook {
    inner: Shared,
}

impl PyWorkbook {
    fn from_inner(inner: Workbook) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    fn lock(&self) -> PyResult<MutexGuard<'_, Workbook>> {
        lock(&self.inner)
    }

    /// The live view of the sheet `name`, once the workbook has proven it.
    fn view(&self, name: &str) -> PySheet {
        PySheet::shared(Arc::clone(&self.inner), name.to_owned())
    }
}

#[pymethods]
impl PyWorkbook {
    /// An empty workbook with no sheet.
    #[new]
    fn new() -> Self {
        Self::from_inner(Workbook::new())
    }

    /// Open the package `value` holds: an `IOBase` handle, a path, a URL or
    /// bytes. The archive is indexed and the workbook documents read; no
    /// sheet is parsed until it is asked for. A handle holding nothing
    /// opens as an empty workbook.
    #[staticmethod]
    fn open(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let holder = holder_from_value(py, value)?;
        py.detach(move || Workbook::open(holder))
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Open the package `data` holds.
    #[staticmethod]
    fn from_bytes(py: Python<'_>, data: Vec<u8>) -> PyResult<Self> {
        py.detach(move || Workbook::from_bytes(data))
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// The date system serials are read and written in: `1900` or `1904`.
    #[getter]
    fn date_system(&self) -> PyResult<&'static str> {
        Ok(self.lock()?.date_system().as_str())
    }

    #[setter]
    fn set_date_system(&self, value: &str) -> PyResult<()> {
        let system = date_system_from(value)?;
        self.lock()?.set_date_system(system);
        Ok(())
    }

    /// Every sheet's name, in tab order.
    #[getter]
    fn sheet_names(&self) -> PyResult<Vec<String>> {
        Ok(self
            .lock()?
            .sheet_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect())
    }

    /// `worksheet`, `chartsheet` or `dialogsheet` for the sheet `name`,
    /// `None` when no sheet has it.
    fn sheet_kind(&self, name: &str) -> PyResult<Option<&'static str>> {
        Ok(self
            .lock()?
            .sheet_kind(name)
            .map(yggdryl::excel::SheetKind::as_str))
    }

    /// How many sheets the workbook holds.
    fn __len__(&self) -> PyResult<usize> {
        Ok(self.lock()?.len())
    }

    /// Whether the workbook holds no sheet.
    fn is_empty(&self) -> PyResult<bool> {
        Ok(self.lock()?.is_empty())
    }

    fn __contains__(&self, name: &str) -> PyResult<bool> {
        Ok(self.lock()?.sheet_kind(name).is_some())
    }

    /// The worksheets, in tab order.
    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyIterator>> {
        let names: Vec<String> = self.sheet_names()?;
        let sheets: Vec<PySheet> = names.iter().map(|name| self.view(name)).collect();
        list_iterator(py, sheets)
    }

    /// The worksheet `name`, parsed on first access, as a live view.
    fn sheet(&self, name: &str) -> PyResult<PySheet> {
        self.lock()?.sheet(name).map_err(excel_error)?;
        Ok(self.view(name))
    }

    /// The worksheet `name` as a live view, `None` when no sheet has it.
    fn get_sheet(&self, name: &str) -> PyResult<Option<PySheet>> {
        Ok(self
            .lock()?
            .get_sheet(name)
            .map_err(excel_error)?
            .map(|_| self.view(name)))
    }

    /// The sheet at the zero-based `index` in tab order, `None` past the
    /// last.
    fn sheet_at(&self, index: usize) -> PyResult<Option<PySheet>> {
        let name = {
            let workbook = self.lock()?;
            let Some(sheet) = workbook.sheet_at(index).map_err(excel_error)? else {
                return Ok(None);
            };
            sheet.name().to_owned()
        };
        Ok(Some(self.view(&name)))
    }

    /// `workbook["Trades"]` and `workbook[0]` are the sheet by name or by
    /// tab position.
    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<PySheet> {
        if let Ok(name) = key.extract::<&str>() {
            return self.sheet(name);
        }
        if let Ok(index) = key.extract::<isize>() {
            let length = self.__len__()?;
            let resolved = if index < 0 {
                length.checked_sub(index.unsigned_abs())
            } else {
                usize::try_from(index).ok()
            };
            return match resolved.and_then(|at| (at < length).then_some(at)) {
                Some(at) => self.sheet_at(at)?.ok_or_else(|| PyKeyError::new_err(index)),
                None => Err(pyo3::exceptions::PyIndexError::new_err(index)),
            };
        }
        Err(PyTypeError::new_err(format!(
            "expected a sheet name or a tab index, got {}",
            type_name(key)
        )))
    }

    /// Add an empty worksheet named `name` after the last tab, as a live
    /// view; a name a sheet already has is refused.
    fn add_sheet(&self, name: &str) -> PyResult<PySheet> {
        self.lock()?.add_sheet(name).map_err(value_error)?;
        Ok(self.view(name))
    }

    /// Put `sheet` in the workbook: in place of the sheet of the same name,
    /// answering it, or after the last tab. The object then views the
    /// sheet in this workbook.
    fn insert_sheet(&self, mut sheet: PyRefMut<'_, PySheet>) -> PyResult<Option<PySheet>> {
        let snapshot = sheet.snapshot()?;
        let name = snapshot.name().to_owned();
        let previous = self.lock()?.insert_sheet(snapshot).map_err(value_error)?;
        sheet.held = Held::Shared {
            workbook: Arc::clone(&self.inner),
            name,
        };
        Ok(previous.map(PySheet::own))
    }

    /// Take the sheet `name` out, answering it as a sheet of its own, or
    /// `None` when no sheet has it.
    fn remove_sheet(&self, name: &str) -> PyResult<Option<PySheet>> {
        Ok(self
            .lock()?
            .remove_sheet(name)
            .map_err(value_error)?
            .map(PySheet::own))
    }

    /// Rename the sheet `name` to `new_name`, keeping its place.
    fn rename_sheet(&self, name: &str, new_name: &str) -> PyResult<()> {
        self.lock()?
            .rename_sheet(name, new_name)
            .map_err(excel_error)
    }

    fn __delitem__(&self, name: &str) -> PyResult<()> {
        match self.remove_sheet(name)? {
            Some(_) => Ok(()),
            None => Err(PyKeyError::new_err(name.to_owned())),
        }
    }

    /// The package as bytes: every sheet written, and every other part of
    /// an opened package kept as it was.
    #[allow(clippy::wrong_self_convention)] // Binding `into_*` methods do not consume wrappers.
    fn into_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let shared = Arc::clone(&self.inner);
        let bytes = py
            .detach(move || {
                let workbook = shared.lock().map_err(|_| {
                    yggdryl::Error::unsupported("writing a poisoned workbook", "workbook")
                })?;
                workbook.into_bytes()
            })
            .map_err(value_error)?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Write the package into `target`: an `IOBase` handle, a path or a URL.
    fn write_into(&self, py: Python<'_>, target: &Bound<'_, PyAny>) -> PyResult<()> {
        let bytes = self.into_bytes(py)?;
        let bytes = bytes.as_bytes();
        if let Ok(mut handle) = target.extract::<PyRefMut<'_, PyIOBase>>() {
            return handle
                .inner_mut()?
                .write_all_bytes(bytes)
                .map_err(value_error);
        }
        let built = py
            .import("yggdryl._native")?
            .getattr("IOBase")?
            .call1((target,))?;
        let mut handle = built.extract::<PyRefMut<'_, PyIOBase>>()?;
        handle
            .inner_mut()?
            .write_all_bytes(bytes)
            .map_err(value_error)
    }

    /// Calls the package's handle has answered so far, in `IOBase` calls.
    #[getter]
    fn handle_reads(&self) -> PyResult<u64> {
        Ok(self.lock()?.handle_reads())
    }

    fn __repr__(&self) -> PyResult<String> {
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

/// Register the workbook classes.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyCellRef>()?;
    module.add_class::<PyCellRange>()?;
    module.add_class::<PyCellRefIterator>()?;
    module.add_class::<PyCell>()?;
    module.add_class::<PyRow>()?;
    module.add_class::<PySheet>()?;
    module.add_class::<PyWorkbook>()?;
    module.add("EXCEL_MAX_ROWS", yggdryl::excel::MAX_ROWS)?;
    module.add("EXCEL_MAX_COLUMNS", yggdryl::excel::MAX_COLUMNS)?;
    module.add("EXCEL_MAX_CELL_TEXT", yggdryl::excel::MAX_CELL_TEXT)?;
    module.add("EXCEL_MAX_SHEET_NAME", yggdryl::excel::MAX_SHEET_NAME)?;
    module.add(
        "EXCEL_DEFAULT_SHEET_NAME",
        yggdryl::excel::DEFAULT_SHEET_NAME,
    )?;
    Ok(())
}
