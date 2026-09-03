//! The typed Python record adapters into the Rust streaming surface.
//!
//! Each public media method accepts exactly the representation named by the
//! method: an Arrow C Stream reader, a `pyarrow.Table`, one
//! `pyarrow.RecordBatch`, or row records. They all become the same core
//! [`BatchReader`](yggdryl::arrow::BatchReader), while reads return a
//! `pyarrow.RecordBatchReader`. The Arrow paths therefore share buffers across
//! the boundary instead of copying or rebuilding batches.
//!
//! [`PyRecordOptions`] is the settings value every record call takes. It is the
//! core [`RecordOptions`] and not a Python model of one, so the encoding a
//! handle uses is still derived from its media type rather than guessed here.
//!
//! # Internal widening
//!
//! [`batch_reader_from_any`] remains an internal conversion funnel for
//! non-media utilities that intentionally accept several Python shapes. The
//! media API does not call it: broad inference belongs to the generic mode
//! dispatcher, while the intent-specific entry points stay statically and
//! dynamically honest about their input shape.
//!
//! Nothing that could stream is collected: a generator is pulled one item at a
//! time and each item is drained before the next is asked for, so a sequence of
//! tables larger than memory writes exactly as a reader would.
//!
//! # Foreign frames
//!
//! `pandas` and `polars` are neither dependencies of this package nor imported
//! by it at load time. An incoming value is recognized by *its type's* module
//! and qualified name, which reads attributes that are already there rather
//! than importing anything, so a caller who has never installed polars pays
//! nothing for its support and never sees an `ImportError` raised by a library
//! they are not using. The import happens inside the one call that cannot
//! proceed without it - reading rows *into* a frame - and its absence is
//! reported as the missing dependency it is.

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_array::ffi_stream::ArrowArrayStreamReader;
use arrow_pyarrow::{FromPyArrow, IntoPyArrow};
use arrow_schema::{Schema as ArrowSchema, SchemaRef};
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyImportError, PyStopIteration, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyByteArray, PyBytes, PyDict, PyList, PyMapping, PyMemoryView, PyString, PyType,
};

use yggdryl::arrow::BatchReader;
use yggdryl::generic::{IORecordOptions, RecordOptions};
use yggdryl::{ArrowCast, Field as CoreField, Level, Metadata};

use crate::datatype::{PyDataType, core_dtype_from_value};
use crate::field::{PyField, core_field_from_value, core_schema_from_pyarrow};
use crate::media::{PyMimeType, core_media_type_from_value};
use crate::value_error;

/// Read a core root Field out of anything Python describes rows with.
///
/// A root is a non-null Struct Field, and Python spells one four ways: the
/// native wrapper, a field expression, a `pyarrow.Schema`, or a
/// `pyarrow.Field`. `name` names the struct when the spelling carries no
/// name of its own, because Arrow names columns and never the record.
pub(crate) fn core_root_field_from_value(
    value: &Bound<'_, PyAny>,
    name: &str,
) -> PyResult<CoreField> {
    if value.extract::<PyRef<'_, PyField>>().is_ok() || value.extract::<&str>().is_ok() {
        return core_field_from_value(value);
    }
    if is_pyarrow_schema(value) {
        let schema = core_schema_from_pyarrow(value)?;
        return CoreField::from_arrow_schema(name, &schema).map_err(value_error);
    }
    core_field_from_value(value)
}

/// Report whether a value is a `pyarrow.Schema`.
///
/// A Schema and a Field both export `__arrow_c_schema__`, and a Schema exported
/// that way arrives as an unnamed struct Field, so the two have to be told
/// apart before the import rather than after it.
fn is_pyarrow_schema(value: &Bound<'_, PyAny>) -> bool {
    value
        .py()
        .import("pyarrow")
        .and_then(|module| module.getattr("Schema"))
        .and_then(|class| value.is_instance(&class))
        .unwrap_or(false)
}

/// Read a core batch reader out of anything `PyArrow` exports batches as.
///
/// The C Stream interface is the fast path and the one every current `PyArrow`
/// container implements, so a reader, a table, and a batch all arrive without a
/// copy. A plain sequence of batches is accepted too, because that is what a
/// caller who built rows one batch at a time is holding.
///
/// # Errors
///
/// Returns a `TypeError` when the value exports no batches, or a `ValueError`
/// when a sequence is empty and therefore names no schema.
pub(crate) fn batch_reader_from_value(value: &Bound<'_, PyAny>) -> PyResult<BatchReader> {
    if value.hasattr("__arrow_c_stream__")? {
        let reader = ArrowArrayStreamReader::from_pyarrow_bound(value)?;
        return Ok(Box::new(reader));
    }
    if let Ok(batches) = value.try_iter() {
        let mut collected: Vec<RecordBatch> = Vec::new();
        for batch in batches {
            collected.push(RecordBatch::from_pyarrow_bound(&batch?)?);
        }
        let schema = collected.first().map(RecordBatch::schema).ok_or_else(|| {
            PyValueError::new_err(
                "expected at least one batch to take a schema from, got an empty sequence",
            )
        })?;
        return Ok(yggdryl::arrow::batch_reader(schema, collected));
    }
    Err(PyTypeError::new_err(
        "expected a pyarrow.RecordBatchReader, Table, RecordBatch, Arrow C stream exporter, or \
         iterable of RecordBatch",
    ))
}

/// Read an Arrow stream while refusing shapes with their own typed adapter.
///
/// A foreign object implementing the Arrow C stream protocol is a reader at
/// this boundary. A `pyarrow.Table` and `pyarrow.RecordBatch` are refused even
/// though Arrow can export them as streams: their dedicated methods redirect
/// through the core's held-table or held-batch path instead.
pub(crate) fn batch_reader_from_arrow_reader(value: &Bound<'_, PyAny>) -> PyResult<BatchReader> {
    let pyarrow = value.py().import("pyarrow")?;
    let table = pyarrow.getattr("Table")?;
    let batch = pyarrow.getattr("RecordBatch")?;
    if value.is_instance(&table)?
        || value.is_instance(&batch)?
        || Frames::Pandas.holds(value)
        || Frames::Polars.holds(value)
    {
        return Err(PyTypeError::new_err(format!(
            "expected a pyarrow.RecordBatchReader or Arrow C stream reader, got {}",
            type_name(value)
        )));
    }
    let reader = pyarrow.getattr("RecordBatchReader")?;
    if value.is_instance(&reader)? || value.hasattr("__arrow_c_stream__")? {
        return batch_reader_from_value(value);
    }
    Err(PyTypeError::new_err(format!(
        "expected a pyarrow.RecordBatchReader or Arrow C stream reader, got {}",
        type_name(value)
    )))
}

/// Read exactly one `pyarrow.Table` as a zero-copy Arrow stream.
pub(crate) fn batch_reader_from_arrow_table(value: &Bound<'_, PyAny>) -> PyResult<BatchReader> {
    let table = value.py().import("pyarrow")?.getattr("Table")?;
    if !value.is_instance(&table)? {
        return Err(PyTypeError::new_err(format!(
            "expected a pyarrow.Table, got {}",
            type_name(value)
        )));
    }
    batch_reader_from_value(value)
}

/// Import exactly one `pyarrow.RecordBatch` for the core's held-batch path.
pub(crate) fn record_batch_from_value(value: &Bound<'_, PyAny>) -> PyResult<RecordBatch> {
    let batch = value.py().import("pyarrow")?.getattr("RecordBatch")?;
    if !value.is_instance(&batch)? {
        return Err(PyTypeError::new_err(format!(
            "expected a pyarrow.RecordBatch, got {}",
            type_name(value)
        )));
    }
    RecordBatch::from_pyarrow_bound(value)
}

/// The rows one batch holds when a caller streaming plain rows sets no bound.
///
/// Mappings and schema-shaped sequences use the same bounded grouping.
const DEFAULT_ROWS_PER_BATCH: usize = yggdryl::generic::DEFAULT_RECORD_BATCH_ROW_SIZE;

/// A `DataFrame` library this boundary converts to and from.
///
/// Neither is a dependency of this package. The variant selects which package
/// name an incoming value is recognized against and which one a read imports,
/// and nothing is imported until a call actually needs it.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Frames {
    /// `pandas`, whose one frame type is `pandas.DataFrame`.
    Pandas,
    /// `polars`, whose frame types are `polars.DataFrame` and `LazyFrame`.
    Polars,
}

impl Frames {
    /// The package a frame of this kind is declared by, and imported from.
    const fn package(self) -> &'static str {
        match self {
            Self::Pandas => "pandas",
            Self::Polars => "polars",
        }
    }

    /// Report whether `value` is one of this library's frames.
    ///
    /// A `polars.LazyFrame` counts: it names rows this library can produce,
    /// and producing them is what a write asks it for.
    fn holds(self, value: &Bound<'_, PyAny>) -> bool {
        match self {
            Self::Pandas => declared_by(value, "pandas", "DataFrame"),
            Self::Polars => {
                declared_by(value, "polars", "DataFrame")
                    || declared_by(value, "polars", "LazyFrame")
            }
        }
    }
}

/// Report whether `module` is `package` itself or a module inside it.
fn inside_package(module: &str, package: &str) -> bool {
    module
        .strip_prefix(package)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
}

/// Report whether a value's type is `qualname`, declared inside `package`.
///
/// This reads the type's own `__module__` and `__qualname__` rather than
/// importing `package` to compare against its classes, which is what keeps a
/// caller who never installed that package from paying an import for it - and
/// from being handed an `ImportError` about a library they are not using.
pub(crate) fn declared_by(value: &Bound<'_, PyAny>, package: &str, qualname: &str) -> bool {
    let class = value.get_type();
    let named = class
        .qualname()
        .and_then(|name| name.extract::<String>())
        .is_ok_and(|name| name == qualname);
    named
        && class
            .module()
            .and_then(|module| module.extract::<String>())
            .is_ok_and(|module| inside_package(&module, package))
}

/// Import a frame library, naming it when it is not installed.
///
/// The conversion that needs it would otherwise fail with whatever `PyArrow`
/// raises several frames deep, so the dependency is named here instead.
///
/// # Errors
///
/// Returns an `ImportError` carrying the original failure.
fn import_frames(py: Python<'_>, library: Frames) -> PyResult<Bound<'_, PyModule>> {
    let package = library.package();
    py.import(package).map_err(|error| {
        PyImportError::new_err(format!(
            "reading rows as {package} frames needs {package} installed, and importing it \
             failed: {error}"
        ))
    })
}

/// Convert one foreign frame into the `PyArrow` table it exports.
///
/// A polars frame exports Arrow itself, so nothing is imported to convert one.
/// A pandas frame does not, so `PyArrow` - which is a dependency - converts it,
/// and that one path behaves the same on every pandas release rather than
/// depending on whether the installed one implements the C stream protocol.
///
/// # Errors
///
/// Returns whatever the library's own conversion raised.
fn frame_to_arrow<'py>(frame: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if declared_by(frame, "polars", "LazyFrame") {
        // A lazy frame has not computed its rows yet, and polars offers no way
        // to hand them over a batch at a time, so collecting is what asking it
        // for rows means.
        return frame.call_method0("collect")?.call_method0("to_arrow");
    }
    if declared_by(frame, "polars", "DataFrame") {
        return polars_to_arrow(frame);
    }
    frame
        .py()
        .import("pyarrow")?
        .getattr("Table")?
        .call_method1("from_pandas", (frame,))
}

/// Hand a polars frame over as Arrow at the newest compat level.
///
/// The newest level keeps polars' view arrays as Arrow view types instead of
/// downgrading them to the offset layouts, so the crossing moves no bytes.
pub(crate) fn polars_to_arrow<'py>(frame: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let py = frame.py();
    let level = py
        .import("polars")
        .and_then(|polars| polars.getattr("CompatLevel"))?
        .call_method0("newest")?;
    let kwargs = pyo3::types::PyDict::new(py);
    kwargs.set_item("compat_level", level)?;
    frame.call_method("to_arrow", (), Some(&kwargs))
}

/// Read a core batch reader out of anything Python holds rows in.
///
/// This is the widest of the three inference points and the one the generic
/// entry points use. It accepts, in order: a foreign `pandas` or `polars`
/// frame, anything exporting an Arrow C stream, a `pyarrow.dataset.Scanner` or
/// `Dataset`, an iterable of any of those, and an iterable of plain rows.
///
/// Every iterable is consumed lazily: one item is pulled, drained, and dropped
/// before the next is asked for, so a generator that could stream is never
/// collected into a list.
///
/// # Errors
///
/// Returns a `TypeError` naming the shapes that are accepted when the value is
/// none of them, or a `ValueError` when an iterable yields nothing and
/// therefore names no schema.
pub(crate) fn batch_reader_from_any(
    value: &Bound<'_, PyAny>,
    options: &RecordOptions,
) -> PyResult<BatchReader> {
    if let Some(reader) = columnar_reader(value)? {
        return Ok(reader);
    }
    // Iterating a mapping yields its keys, so one handed in directly would
    // arrive here as a sequence of strings and fail several frames later. A
    // mapping is either one row or a set of columns, and only the caller knows
    // which, so both spellings are named instead of one being guessed.
    if value.cast::<PyMapping>().is_ok() && value.hasattr("keys")? {
        return Err(PyTypeError::new_err(
            "expected rows, got one mapping; wrap it in a list to write it as a single row, or \
             pass pyarrow.table(mapping) to write it as columns",
        ));
    }
    let Ok(items) = value.try_iter() else {
        return Err(PyTypeError::new_err(
            "expected rows as a pyarrow RecordBatchReader, Table, RecordBatch, Dataset, Scanner, \
             an Arrow C stream exporter, a pandas or polars frame, an iterable of any of those, \
             or an iterable of mappings",
        ));
    };
    chained_reader(&items, options, None)
}

/// Build one streamed reader from Python row records.
///
/// A decorated dataclass instance supplies its class's cached
/// `field()` when no field was declared explicitly. Empty input
/// cannot infer a shape, so it is accepted only when `options.field` already
/// names one.
pub(crate) fn batch_reader_from_records(
    value: &Bound<'_, PyAny>,
    options: &mut RecordOptions,
) -> PyResult<BatchReader> {
    if value.cast::<PyMapping>().is_ok() && value.hasattr("keys")? {
        return Err(PyTypeError::new_err(
            "expected an iterable of records, got one mapping; wrap it in a list",
        ));
    }
    let items = value.try_iter().map_err(|_| {
        PyTypeError::new_err(format!(
            "expected an iterable of mapping, sequence, or dataclass records, got {}",
            type_name(value)
        ))
    })?;
    let Some(first) = next_item(&items)? else {
        let field = options.field().ok_or_else(|| {
            PyValueError::new_err("empty records cannot infer a field; declare options.field")
        })?;
        let schema = field.clone().into_arrow_schema().map_err(value_error)?;
        return Ok(yggdryl::arrow::batch_reader(schema, []));
    };
    if options.field().is_none() && is_dataclass_instance(&first)? {
        let field = core_root_field_from_value(&first, options.name())?;
        options.set_field(field);
    }
    row_reader(&items, &first, options)
}

/// Report whether `value` is a dataclass instance rather than its class.
fn is_dataclass_instance(value: &Bound<'_, PyAny>) -> PyResult<bool> {
    if value.is_instance_of::<PyType>() {
        return Ok(false);
    }
    value
        .py()
        .import("dataclasses")?
        .getattr("is_dataclass")?
        .call1((value,))?
        .extract::<bool>()
}

/// Read a batch reader out of a value that is already columnar, if it is one.
///
/// `None` means the value names rows some other way, which is what leaves the
/// iterable paths to the caller. Nothing here consumes an iterator.
///
/// # Errors
///
/// Returns whatever an attribute lookup or a library conversion raised.
fn columnar_reader(value: &Bound<'_, PyAny>) -> PyResult<Option<BatchReader>> {
    // A frame is recognized before the stream protocol so that the conversion
    // is the library's own on every release of it, rather than the C stream on
    // the releases that grew one.
    if Frames::Pandas.holds(value) || Frames::Polars.holds(value) {
        return batch_reader_from_value(&frame_to_arrow(value)?).map(Some);
    }
    if value.hasattr("__arrow_c_stream__")? {
        return batch_reader_from_value(value).map(Some);
    }
    // A `Scanner` already describes one pass over rows, and a `Dataset` makes
    // one on request. Both hand back a reader, so neither is materialized.
    if value.hasattr("to_reader")? {
        return batch_reader_from_value(&value.call_method0("to_reader")?).map(Some);
    }
    if value.hasattr("scanner")? {
        let scanner = value.call_method0("scanner")?;
        return batch_reader_from_value(&scanner.call_method0("to_reader")?).map(Some);
    }
    Ok(None)
}

/// Build a reader over an iterator whose items are readers or rows.
///
/// The first item decides which: an item that is columnar makes this a chain of
/// streams, and anything else makes it a stream of rows. Exactly one item is
/// pulled to answer that question, so an iterator that could stream still does.
///
/// `only` restricts every item to one library's frames, which is what the named
/// frame entry points hold callers to; `None` accepts everything columnar.
///
/// # Errors
///
/// Returns a `ValueError` when the iterator is empty, or whatever converting
/// its first item raised.
fn chained_reader(
    items: &Bound<'_, PyAny>,
    options: &RecordOptions,
    only: Option<Frames>,
) -> PyResult<BatchReader> {
    let Some(first) = next_item(items)? else {
        return Err(PyValueError::new_err(
            "expected at least one item to take a schema from, got an empty iterable",
        ));
    };
    let reader = match only {
        Some(library) => frame_reader(&first, library)?,
        None => match columnar_reader(&first)? {
            Some(reader) => reader,
            None => return row_reader(items, &first, options),
        },
    };
    let root = CoreField::from_arrow_schema(options.name(), reader.schema().as_ref())
        .map_err(value_error)?;
    Ok(Box::new(Chained {
        items: items.clone().unbind(),
        schema: reader.schema(),
        root,
        safe: options.safe(),
        current: Some(reader),
        only,
        drained: false,
    }))
}

/// Read a batch reader out of one value, which must be one library's frame.
///
/// # Errors
///
/// Returns a `TypeError` naming the library and what arrived instead.
fn frame_reader(value: &Bound<'_, PyAny>, library: Frames) -> PyResult<BatchReader> {
    if !library.holds(value) {
        return Err(PyTypeError::new_err(format!(
            "expected one {} frame, got {}",
            library.package(),
            type_name(value)
        )));
    }
    batch_reader_from_value(&frame_to_arrow(value)?)
}

/// Name a value's type the way an error should show it.
fn type_name(value: &Bound<'_, PyAny>) -> String {
    value.get_type().fully_qualified_name().map_or_else(
        |_| "an unnameable value".to_owned(),
        |name| name.to_string(),
    )
}

/// Pull one item from a Python iterator, or report that it is exhausted.
///
/// # Errors
///
/// Returns whatever the iterator raised, other than its own exhaustion.
fn next_item<'py>(items: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    match items.call_method0("__next__") {
        Ok(item) => Ok(Some(item)),
        Err(error) if error.is_instance_of::<PyStopIteration>(items.py()) => Ok(None),
        Err(error) => Err(error),
    }
}

/// A reader over a Python iterator whose items are each a stream of batches.
///
/// One item is held at a time: it is drained before the next is pulled, and
/// dropped as soon as it is. That is what lets a generator of tables describe
/// more rows than memory holds.
struct Chained {
    /// The Python iterator the items are pulled from.
    items: Py<PyAny>,
    /// The Arrow schema the first item declared, which this reader reports.
    schema: SchemaRef,
    /// The same schema as a root Field, for casting an item that differs.
    root: CoreField,
    /// Whether a cast may null a value it cannot convert.
    safe: bool,
    /// The item currently being drained.
    current: Option<BatchReader>,
    /// The one library every item must be a frame of, when a caller named one.
    only: Option<Frames>,
    /// Whether the iterator has already reported exhaustion.
    drained: bool,
}

impl Chained {
    /// Cast one item's batch to the shape the first item set, if it differs.
    ///
    /// Two tables in one sequence may order or type their columns differently
    /// and still describe the same rows, so the declared root is applied rather
    /// than the disagreement being refused. A batch the root cannot hold is the
    /// core's error, with the columns it names.
    fn conform(&self, batch: RecordBatch) -> Result<RecordBatch, arrow_schema::ArrowError> {
        if batch.schema() == self.schema {
            return Ok(batch);
        }
        self.root
            .cast_arrow_batch(batch, self.safe)
            .map_err(|error| arrow_schema::ArrowError::ExternalError(Box::new(error)))
    }
}

impl Iterator for Chained {
    type Item = Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current) = self.current.as_mut() {
                match current.next() {
                    Some(Ok(batch)) => return Some(self.conform(batch)),
                    Some(Err(error)) => return Some(Err(error)),
                    None => self.current = None,
                }
            }
            if self.drained {
                return None;
            }
            let pulled = Python::attach(|py| -> PyResult<Option<BatchReader>> {
                let items = self.items.bind(py);
                let Some(item) = next_item(items)? else {
                    return Ok(None);
                };
                if let Some(library) = self.only {
                    return frame_reader(&item, library).map(Some);
                }
                columnar_reader(&item)?.map(Some).ok_or_else(|| {
                    PyTypeError::new_err(format!(
                        "expected every item of a sequence of batches to name batches too, got {}",
                        type_name(&item)
                    ))
                })
            });
            match pulled {
                Ok(Some(reader)) => self.current = Some(reader),
                Ok(None) => {
                    self.drained = true;
                    return None;
                }
                Err(error) => {
                    self.drained = true;
                    return Some(Err(arrow_schema::ArrowError::ExternalError(Box::new(
                        error,
                    ))));
                }
            }
        }
    }
}

impl arrow_array::RecordBatchReader for Chained {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Build a reader over an iterator of plain rows.
///
/// `first` is the row already pulled to decide this, and it is put back at the
/// head rather than being lost. Rows are grouped into batches of at most
/// [`DEFAULT_ROWS_PER_BATCH`], or of whatever the options bound them to, so an
/// unbounded generator of rows writes without ever being a list.
///
/// # Errors
///
/// Returns a `TypeError` when a row is neither a mapping nor a sequence a
/// declared schema names the columns of.
fn row_reader(
    items: &Bound<'_, PyAny>,
    first: &Bound<'_, PyAny>,
    options: &RecordOptions,
) -> PyResult<BatchReader> {
    let py = items.py();
    let declared = match options.field() {
        Some(field) => Some(
            field
                .clone()
                .into_arrow_schema()
                .map_err(value_error)?
                .as_ref()
                .clone()
                .into_pyarrow(py)?,
        ),
        None => None,
    };
    let mut rows = Rows {
        items: items.clone().unbind(),
        from_pylist: py
            .import("pyarrow")?
            .getattr("RecordBatch")?
            .getattr("from_pylist")?
            .unbind(),
        columns: declared.map(Bound::unbind),
        names: None,
        schema: Arc::new(ArrowSchema::empty()),
        per_batch: options
            .batch_row_size()
            .unwrap_or(DEFAULT_ROWS_PER_BATCH)
            .max(1),
        // Conversion must stop at each exact publication boundary. A fixed
        // `min(batch_row_size, commit_row_size)` is not enough when the two do not
        // divide: for batch 1,024 and commit 1,500 the second conversion must
        // stop after 476 rows, publish, and only then inspect row 1,501.
        commit_row_size: options.commit_row_size().filter(|rows| *rows != 0),
        commit_progress: 0,
        remaining: options.max_row_size(),
        pending: Some(first.clone().unbind()),
        drained: false,
    };
    // The first batch is built here so the reader can declare a schema before
    // anything reads it, which is what a `RecordBatchReader` promises.
    let head = rows.fill()?.ok_or_else(|| {
        PyValueError::new_err("expected at least one row to take a schema from, got none")
    })?;
    rows.schema = head.schema();
    Ok(Box::new(Head {
        head: Some(head),
        rows,
    }))
}

/// A reader that yields one already-built batch before the rest.
///
/// The first batch has to exist before the reader does, because that is where
/// an inferred schema comes from; this is what hands it back in order.
struct Head {
    /// The batch built while the schema was being inferred.
    head: Option<RecordBatch>,
    /// The rest of the rows.
    rows: Rows,
}

impl Iterator for Head {
    type Item = Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(head) = self.head.take() {
            return Some(Ok(head));
        }
        self.rows.next()
    }
}

impl arrow_array::RecordBatchReader for Head {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.rows.schema)
    }
}

/// A reader that groups plain Python rows into batches as they arrive.
struct Rows {
    /// The Python iterator the rows are pulled from.
    items: Py<PyAny>,
    /// `pyarrow.RecordBatch.from_pylist`, resolved once rather than per batch.
    from_pylist: Py<PyAny>,
    /// The `pyarrow.Schema` every batch is built under, when one was declared.
    columns: Option<Py<PyAny>>,
    /// The column names positional rows are zipped against.
    names: Option<Py<PyList>>,
    /// The schema the first batch settled, which every later one is built to.
    schema: SchemaRef,
    /// The most rows one batch holds.
    per_batch: usize,
    /// The exact publication cadence rows must not be converted across.
    commit_row_size: Option<usize>,
    /// Rows converted since the last exact publication boundary.
    commit_progress: usize,
    /// Rows the global positive write limit still admits.
    remaining: Option<u64>,
    /// The row pulled to decide this was a stream of rows at all.
    pending: Option<Py<PyAny>>,
    /// Whether the iterator has already reported exhaustion.
    drained: bool,
}

impl Rows {
    /// Pull up to one batch worth of rows and build the batch they make.
    ///
    /// `None` means the rows ran out with nothing left to build.
    ///
    /// # Errors
    ///
    /// Returns whatever the iterator raised, or a `TypeError` naming a row
    /// shape that cannot become one.
    fn fill(&mut self) -> PyResult<Option<RecordBatch>> {
        if self.remaining == Some(0) {
            self.drained = true;
            return Ok(None);
        }
        Python::attach(|py| {
            // The iterator is held owned for the length of the chunk: binding
            // it in place would borrow the reader that the row conversion below
            // has to be able to update.
            let items = self.items.clone_ref(py).into_bound(py);
            let from_pylist = self.from_pylist.clone_ref(py).into_bound(py);
            let chunk = PyList::empty(py);
            let mut target = self.commit_row_size.map_or(self.per_batch, |commit| {
                self.per_batch.min(commit - self.commit_progress)
            });
            if let Some(remaining) = self.remaining {
                target = target.min(usize::try_from(remaining).unwrap_or(usize::MAX));
            }
            if let Some(pending) = self.pending.take() {
                let row = pending.into_bound(py);
                chunk.append(self.mapping(py, &row)?)?;
            }
            while chunk.len() < target && !self.drained {
                match next_item(&items)? {
                    Some(row) => chunk.append(self.mapping(py, &row)?)?,
                    None => self.drained = true,
                }
            }
            if chunk.is_empty() {
                return Ok(None);
            }
            let built = match self.columns.as_ref() {
                Some(columns) => {
                    let arguments = PyDict::new(py);
                    arguments.set_item("schema", columns.bind(py))?;
                    from_pylist.call((chunk,), Some(&arguments))?
                }
                None => from_pylist.call1((chunk,))?,
            };
            let batch = RecordBatch::from_pyarrow_bound(&built)?;
            // The first batch is what names the columns; every later one is
            // built against it, so an inference that saw only nulls in one
            // chunk cannot disagree with the chunk before it.
            if self.columns.is_none() {
                self.columns = Some(built.getattr("schema")?.unbind());
            }
            if let Some(commit) = self.commit_row_size {
                self.commit_progress = (self.commit_progress + batch.num_rows()) % commit;
            }
            if let Some(remaining) = self.remaining.as_mut() {
                *remaining = remaining.saturating_sub(batch.num_rows() as u64);
            }
            Ok(Some(batch))
        })
    }

    /// Return the row as the mapping `from_pylist` reads.
    ///
    /// A mapping is already one. A sequence is one only once something names
    /// its columns, and the only thing that can is a declared field, so a
    /// positional row without one is refused rather than guessed at.
    ///
    /// # Errors
    ///
    /// Returns a `TypeError` naming what the row would need to be usable.
    fn mapping<'py>(
        &mut self,
        py: Python<'py>,
        row: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        if row.cast::<PyMapping>().is_ok() && row.hasattr("keys")? {
            return Ok(row.clone());
        }
        if is_dataclass_instance(row)? {
            return py
                .import("yggdryl.fields._classes")?
                .getattr("into_dict")?
                .call1((row,));
        }
        if row.cast::<PyString>().is_ok() {
            return Err(PyTypeError::new_err(
                "expected a row as a mapping or a sequence of its values, got a string",
            ));
        }
        let names = self.column_names(py)?;
        let paired = PyDict::new(py);
        let mut values = row.try_iter().map_err(|_| {
            PyTypeError::new_err(
                "expected a row as a mapping of column to value, or as a sequence of values, got \
                 a value that is neither",
            )
        })?;
        for name in names.bind(py) {
            let Some(value) = values.next() else {
                return Err(PyValueError::new_err(format!(
                    "expected a value for every declared column, got a row missing {}",
                    name.repr()?
                )));
            };
            paired.set_item(name, value?)?;
        }
        if values.next().is_some() {
            return Err(PyValueError::new_err(
                "expected a value for every declared column, got a row with more values than the \
                 schema has columns",
            ));
        }
        Ok(paired.into_any())
    }

    /// Return the declared column names positional rows are zipped against.
    ///
    /// # Errors
    ///
    /// Returns a `TypeError` when no field was declared, because nothing else
    /// in a bare sequence of values says which column each one is.
    fn column_names(&mut self, py: Python<'_>) -> PyResult<Py<PyList>> {
        if let Some(names) = self.names.as_ref() {
            return Ok(names.clone_ref(py));
        }
        let columns = self.columns.as_ref().ok_or_else(|| {
            PyTypeError::new_err(
                "expected a row as a mapping, got a sequence of values and no field on the \
                 options naming the columns they fill",
            )
        })?;
        let names = columns
            .bind(py)
            .getattr("names")?
            .extract::<Vec<String>>()?;
        let names = PyList::new(py, names)?.unbind();
        self.names = Some(names.clone_ref(py));
        Ok(names)
    }
}

impl Iterator for Rows {
    type Item = Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.fill() {
            Ok(Some(batch)) => Some(Ok(batch)),
            Ok(None) => None,
            Err(error) => {
                self.drained = true;
                Some(Err(arrow_schema::ArrowError::ExternalError(Box::new(
                    error,
                ))))
            }
        }
    }
}

impl arrow_array::RecordBatchReader for Rows {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Read a core batch reader out of one library's frames and nothing else.
///
/// The named entry points are strict on purpose: `overwrite_pandas` handed a
/// `polars` frame is a mistake worth naming. Each held shape has one explicit
/// adapter before it reaches the native reader primitive.
///
/// # Errors
///
/// Returns a `TypeError` naming the library when the value is not one of its
/// frames, or an iterable of them.
pub(crate) fn frames_batch_reader(
    value: &Bound<'_, PyAny>,
    library: Frames,
    options: &RecordOptions,
) -> PyResult<BatchReader> {
    if library.holds(value) {
        return batch_reader_from_value(&frame_to_arrow(value)?);
    }
    let package = library.package();
    // A frame of *another* library is iterable over its own columns, so the
    // iterable path has to stay restricted to this one: an item that is not
    // one of its frames is named rather than taken apart.
    let items = value.try_iter().map_err(|_| {
        PyTypeError::new_err(format!(
            "expected a {package} frame or an iterable of them, got {}",
            type_name(value)
        ))
    })?;
    chained_reader(&items, options, Some(library))
}

/// Read a core batch reader out of exactly one of a library's frames.
///
/// # Errors
///
/// Returns a `TypeError` naming the library when the value is not one frame.
pub(crate) fn frame_batch_reader(
    value: &Bound<'_, PyAny>,
    library: Frames,
) -> PyResult<BatchReader> {
    frame_reader(value, library)
}

/// Hand a core reader to Python as a lazy iterator of one library's frames.
///
/// The iterator is `map` over the `PyArrow` reader, so one batch is converted
/// when it is asked for and the resource stays streamable: a table larger than
/// memory reads frame by frame.
///
/// # Errors
///
/// Returns an `ImportError` when the library is not installed.
pub(crate) fn frames_from_reader(
    py: Python<'_>,
    reader: BatchReader,
    library: Frames,
) -> PyResult<Bound<'_, PyAny>> {
    let module = import_frames(py, library)?;
    let convert = match library {
        // A batch converts itself, so the callable is the method rather than a
        // Python lambda this extension would have to define.
        Frames::Pandas => py
            .import("operator")?
            .call_method1("methodcaller", ("to_pandas",))?,
        Frames::Polars => module.getattr("from_arrow")?,
    };
    let batches = batch_reader_to_pyarrow(py, reader)?;
    py.import("builtins")?
        .call_method1("map", (convert, batches))
}

/// Hand a core reader to Python as one frame holding every row.
///
/// # Errors
///
/// Returns an `ImportError` when the library is not installed, or whatever the
/// read or the conversion raised.
pub(crate) fn frame_from_reader(
    py: Python<'_>,
    reader: BatchReader,
    library: Frames,
) -> PyResult<Bound<'_, PyAny>> {
    let module = import_frames(py, library)?;
    let table = batch_reader_to_pyarrow(py, reader)?.call_method0("read_all")?;
    match library {
        Frames::Pandas => table.call_method0("to_pandas"),
        Frames::Polars => module.call_method1("from_arrow", (table,)),
    }
}

/// Hand a core batch reader to Python as a `pyarrow.RecordBatchReader`.
///
/// The reader stays lazy across the boundary: `PyArrow` pulls one batch at a time
/// through the C stream, so a resource larger than memory is readable from
/// Python exactly as it is from Rust.
pub(crate) fn batch_reader_to_pyarrow(
    py: Python<'_>,
    reader: BatchReader,
) -> PyResult<Bound<'_, PyAny>> {
    reader.into_pyarrow(py)
}

/// Chain two readers onto the root their two schemas merge into.
///
/// The lazy crossing is preserved in both directions: the merge is derived
/// from the two schemas alone, which a `RecordBatchReader` answers without
/// pulling a batch, and the result is handed back over the C stream so `PyArrow`
/// pulls one batch at a time.
///
/// Columns unite by name (ASCII case-insensitively), left's order first and
/// right-only columns after; a column present in only one side becomes
/// nullable because the other side's rows have no value for it; a shared
/// column whose datatype or `PARQUET:field_id` disagrees is refused naming both
/// sides rather than silently widened. Passing `schema` declares the root both
/// sides cast onto instead of deriving one.
#[pyfunction]
#[pyo3(signature = (left, right, schema = None, *, safe = true))]
pub(crate) fn combined<'py>(
    py: Python<'py>,
    left: &Bound<'_, PyAny>,
    right: &Bound<'_, PyAny>,
    schema: Option<&Bound<'_, PyAny>>,
    safe: bool,
) -> PyResult<Bound<'py, PyAny>> {
    let left = batch_reader_from_value(left)?;
    let right = batch_reader_from_value(right)?;
    let reader = match schema {
        Some(schema) => {
            let root = core_root_field_from_value(schema, "row")?;
            yggdryl::arrow::combined_as(left, right, &root, safe).map_err(value_error)?
        }
        None => yggdryl::arrow::combined(left, right).map_err(value_error)?,
    };
    batch_reader_to_pyarrow(py, reader)
}

/// Read `(key, value)` string pairs out of a mapping or an iterable of pairs.
///
/// These are the two shapes `IOBase.children_where` already accepts for the
/// same vocabulary, so a filter is spelled the same everywhere.
pub(crate) fn string_pairs_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<(String, String)>> {
    let items = if value.hasattr("items")? {
        value.call_method0("items")?
    } else {
        value.clone()
    };
    let mut pairs = Vec::new();
    for item in items.try_iter()? {
        pairs.push(item?.extract::<(String, String)>()?);
    }
    Ok(pairs)
}

/// Set the row-per-batch bound, refusing a bound of nothing.
///
/// A batch of zero rows is not a small batch: the readers chunk by this number,
/// so it turns a read of a hundred rows into a successful read of none. `None`
/// is how "no bound" is spelled, and it is already available.
fn set_batch_row_size_option(
    options: &mut RecordOptions,
    batch_row_size: Option<usize>,
) -> PyResult<()> {
    if batch_row_size == Some(0) {
        return Err(PyValueError::new_err(
            "expected a positive row count for batch_row_size, got 0; pass None for no bound",
        ));
    }
    options.set_batch_row_size(batch_row_size);
    Ok(())
}

/// Read root metadata out of a mapping, an iterable of pairs, or nothing.
///
/// `None` and an empty collection both spell the empty snapshot, which is how
/// the core clears metadata.
fn metadata_from_value(value: Option<&Bound<'_, PyAny>>) -> PyResult<Metadata> {
    match value {
        Some(value) => Metadata::from_entries(string_pairs_from_value(value)?).map_err(value_error),
        None => Ok(Metadata::new()),
    }
}

/// Snapshot metadata into a plain `dict`, the shape `key_value_metadata` uses.
fn metadata_into_dict<'py>(py: Python<'py>, metadata: &Metadata) -> PyResult<Bound<'py, PyDict>> {
    let pairs = PyDict::new(py);
    for (key, value) in metadata {
        pairs.set_item(key, value)?;
    }
    Ok(pairs)
}

/// Copy one Python byte-buffer value without accepting an integer sequence as
/// an accidental synchronization marker.
fn bytes_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(value) = value.cast::<PyBytes>() {
        return Ok(value.as_bytes().to_vec());
    }
    if let Ok(value) = value.cast::<PyByteArray>() {
        return Ok(value.to_vec());
    }
    if value.cast::<PyMemoryView>().is_ok() {
        return Ok(value
            .call_method0("tobytes")?
            .cast_into::<PyBytes>()?
            .as_bytes()
            .to_vec());
    }
    Err(PyTypeError::new_err(
        "sync_marker must be bytes, bytearray, memoryview, or None",
    ))
}

/// Read one required key from private `RecordOptions` pickle state.
fn required_record_pickle_item<'py>(
    state: &Bound<'py, PyDict>,
    name: &str,
) -> PyResult<Bound<'py, PyAny>> {
    state
        .get_item(name)?
        .ok_or_else(|| PyValueError::new_err(format!("native pickle state is missing {name:?}")))
}

/// The settings one record read or write takes.
#[pyclass(
    name = "RecordOptions",
    module = "yggdryl._native",
    skip_from_py_object
)]
pub(crate) struct PyRecordOptions {
    pub(crate) inner: RecordOptions,
    hash_locked: bool,
}

impl Clone for PyRecordOptions {
    fn clone(&self) -> Self {
        Self::from_core(self.inner.clone())
    }
}

impl PyRecordOptions {
    pub(crate) fn from_core(inner: RecordOptions) -> Self {
        Self {
            inner,
            hash_locked: false,
        }
    }

    fn require_mutable(&self) -> PyResult<()> {
        if self.hash_locked {
            Err(PyTypeError::new_err(
                "hashed RecordOptions are frozen; copy them before mutation",
            ))
        } else {
            Ok(())
        }
    }

    fn pickle_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let state = PyDict::new(py);
        state.set_item("media_type", self.inner.mime_type().as_str())?;
        state.set_item("name", self.inner.name())?;
        state.set_item(
            "dtype",
            self.inner.dtype().cloned().map(PyDataType::from_inner),
        )?;
        state.set_item("metadata", metadata_into_dict(py, self.inner.metadata())?)?;
        state.set_item("safe", self.inner.safe())?;
        state.set_item("batch_row_size", self.inner.batch_row_size())?;
        state.set_item("commit_row_size", self.inner.commit_row_size())?;
        state.set_item("max_row_size", self.inner.max_row_size())?;
        state.set_item("max_byte_size", self.inner.max_byte_size())?;
        state.set_item("level", self.inner.level().get())?;
        state.set_item("merge_by_names", self.inner.merge_by_names().to_vec())?;
        state.set_item("select_by_names", self.inner.select_by_names().to_vec())?;
        state.set_item("filter_partitions", self.inner.filter_partitions().to_vec())?;
        if let Some(block_codec) = self.inner.avro_block_codec() {
            state.set_item("block_codec", block_codec)?;
        }
        if let Some(marker) = self.inner.avro_sync_marker() {
            state.set_item("sync_marker", PyBytes::new(py, marker))?;
        }
        if let Some(compression) = self.inner.parquet_compression_name() {
            state.set_item("compression", compression)?;
        }
        if let Some(rows) = self.inner.parquet_max_row_group_size() {
            state.set_item("max_row_group_size", rows)?;
        }
        if let Some(metadata) = self.inner.parquet_key_value_metadata() {
            state.set_item("key_value_metadata", metadata.to_vec())?;
        }
        Ok(state)
    }
}

/// Read core record options out of a value, or derive them from a media type.
///
/// A caller who only wants to name the encoding passes the media type itself,
/// which is the same derivation `IOBase.record_options` performs.
pub(crate) fn core_record_options_from_value(value: &Bound<'_, PyAny>) -> PyResult<RecordOptions> {
    if let Ok(options) = value.extract::<PyRef<'_, PyRecordOptions>>() {
        return Ok(options.inner.clone());
    }
    RecordOptions::for_media_type(&core_media_type_from_value(value)?).map_err(value_error)
}

#[pymethods]
impl PyRecordOptions {
    /// Derive the options for the encoding a media type names.
    #[new]
    fn new(media_type: &Bound<'_, PyAny>) -> PyResult<Self> {
        RecordOptions::for_media_type(&core_media_type_from_value(media_type)?)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Derive the options for the encoding a media type names.
    #[classmethod]
    fn for_media_type(_cls: &Bound<'_, PyType>, media_type: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::new(media_type)
    }

    /// Rebuild the complete configuration without carrying a transient hash lock.
    #[staticmethod]
    fn _from_pickle(state: &Bound<'_, PyDict>) -> PyResult<Self> {
        let media_type = required_record_pickle_item(state, "media_type")?;
        let mut options = Self::new(&media_type)?;

        let name = required_record_pickle_item(state, "name")?.extract::<String>()?;
        options.set_name(&name)?;
        let dtype = required_record_pickle_item(state, "dtype")?;
        options.set_dtype((!dtype.is_none()).then_some(&dtype))?;
        let metadata = required_record_pickle_item(state, "metadata")?;
        options.set_metadata(Some(&metadata))?;
        options.set_safe(required_record_pickle_item(state, "safe")?.extract()?)?;
        options
            .set_batch_row_size(required_record_pickle_item(state, "batch_row_size")?.extract()?)?;
        let commit_row_size =
            required_record_pickle_item(state, "commit_row_size")?.extract::<Option<usize>>()?;
        options.inner.set_commit_row_size(commit_row_size);
        options.set_max_row_size(required_record_pickle_item(state, "max_row_size")?.extract()?)?;
        options
            .set_max_byte_size(required_record_pickle_item(state, "max_byte_size")?.extract()?)?;
        options.set_level(required_record_pickle_item(state, "level")?.extract()?)?;
        options
            .set_merge_by_names(required_record_pickle_item(state, "merge_by_names")?.extract()?)?;
        options.set_select_by_names(
            required_record_pickle_item(state, "select_by_names")?.extract()?,
        )?;
        options.set_filter_partitions(
            required_record_pickle_item(state, "filter_partitions")?.extract()?,
        )?;

        if let Some(value) = state.get_item("block_codec")? {
            options.set_block_codec(value.extract()?)?;
        }
        if let Some(value) = state.get_item("sync_marker")? {
            options.set_sync_marker(Some(&value))?;
        }
        if let Some(value) = state.get_item("compression")? {
            options.set_compression(value.extract()?)?;
        }
        if let Some(value) = state.get_item("max_row_group_size")? {
            options.set_max_row_group_size(value.extract()?)?;
        }
        if let Some(value) = state.get_item("key_value_metadata")? {
            options.set_key_value_metadata(&value)?;
        }
        Ok(options)
    }

    /// The MIME type of the encoding these options describe.
    #[getter]
    fn mime_type(&self) -> PyMimeType {
        PyMimeType::from_core(self.inner.mime_type())
    }

    /// The root Field name: what an inferred root is called, and the name
    /// of the built `field`.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    #[setter]
    fn set_name(&mut self, name: &str) -> PyResult<()> {
        self.require_mutable()?;
        // The trait's setter names a `SmolStr`, which is the core's string type
        // and not a dependency of this crate; its builder takes anything that
        // converts into one, so the builder is the route from a Python string.
        self.inner = self.inner.clone().with_name(name);
        Ok(())
    }

    /// The declared root datatype, or `None` when the shape is inferred.
    ///
    /// The setter takes a `DataType`, a datatype expression, or anything else
    /// that names a datatype; `None` clears the declaration.
    #[getter]
    fn dtype(&self) -> Option<PyDataType> {
        self.inner.dtype().cloned().map(PyDataType::from_inner)
    }

    #[setter]
    fn set_dtype(&mut self, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.require_mutable()?;
        let dtype = value.map(core_dtype_from_value).transpose()?;
        self.inner.set_dtype(dtype);
        Ok(())
    }

    /// The root metadata, as a snapshot; empty unless declared.
    ///
    /// The setter takes a mapping or an iterable of `(key, value)` string
    /// pairs, `Field.metadata` included; `None` or nothing clears it.
    #[getter]
    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        metadata_into_dict(py, self.inner.metadata())
    }

    #[setter]
    fn set_metadata(&mut self, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_metadata(metadata_from_value(value)?);
        Ok(())
    }

    /// The declared canonical root Field, built from `name`, `dtype`, and
    /// `metadata`; `None` until a datatype is declared.
    ///
    /// The setter takes any root spelling and declares its three parts.
    #[getter]
    fn field(&self) -> Option<PyField> {
        self.inner.field().map(PyField::from_inner)
    }

    #[setter]
    fn set_field(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        let field = core_root_field_from_value(value, self.inner.name())?;
        self.inner.set_field(field);
        Ok(())
    }

    /// Whether a cast may null a value it cannot convert.
    #[getter]
    fn safe(&self) -> bool {
        self.inner.safe()
    }

    #[setter]
    fn set_safe(&mut self, safe: bool) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_safe(safe);
        Ok(())
    }

    /// The row-per-batch bound, when one is set.
    #[getter]
    fn batch_row_size(&self) -> Option<usize> {
        self.inner.batch_row_size()
    }

    #[setter]
    fn set_batch_row_size(&mut self, batch_row_size: Option<usize>) -> PyResult<()> {
        self.require_mutable()?;
        set_batch_row_size_option(&mut self.inner, batch_row_size)
    }

    /// The streamed-write publication cadence, in rows.
    ///
    /// `None` publishes once after the source ends. A positive value publishes
    /// each complete group of that many incoming rows and the final remainder.
    /// Zero is retained so a write can reject it before inspecting a one-shot
    /// Python input.
    #[getter]
    fn commit_row_size(&self) -> Option<usize> {
        self.inner.commit_row_size()
    }

    #[setter]
    fn set_commit_row_size(&mut self, commit_row_size: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.require_mutable()?;
        let commit_row_size = commit_row_size
            .map(|value| {
                if value.is_instance_of::<PyBool>() {
                    return Err(PyTypeError::new_err(
                        "commit_row_size must be an integer or None, not bool",
                    ));
                }
                value.extract::<usize>()
            })
            .transpose()?;
        self.inner.set_commit_row_size(commit_row_size);
        Ok(())
    }

    /// The bound on how many result rows flow in total, when one is set.
    ///
    /// A count of rows, applied last - after the declared schema, selection,
    /// completion cast, and partition filter - so `0` is a valid ask: the
    /// shaped schema with no batches, rather than an error.
    #[getter]
    fn max_row_size(&self) -> Option<u64> {
        self.inner.max_row_size()
    }

    #[setter]
    fn set_max_row_size(&mut self, max_row_size: Option<u64>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_max_row_size(max_row_size);
        Ok(())
    }

    /// The bound on the result rows' Arrow in-memory bytes, when one is set.
    ///
    /// Counted uncompressed, never as encoded bytes; a non-zero bound always
    /// yields at least one row, and only `0` yields nothing.
    #[getter]
    fn max_byte_size(&self) -> Option<u64> {
        self.inner.max_byte_size()
    }

    #[setter]
    fn set_max_byte_size(&mut self, max_byte_size: Option<u64>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_max_byte_size(max_byte_size);
        Ok(())
    }

    /// The compression level applied to a content coding, on the 0-to-9 scale.
    #[getter]
    fn level(&self) -> u8 {
        self.inner.level().get()
    }

    #[setter]
    fn set_level(&mut self, level: u8) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_level(Level::new(level));
        Ok(())
    }

    /// The column names a write matches rows on; empty means overwrite.
    #[getter]
    fn merge_by_names(&self) -> Vec<String> {
        self.inner.merge_by_names().to_vec()
    }

    #[setter]
    fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_merge_by_names(merge_by_names);
        Ok(())
    }

    /// The column names a read or write is narrowed to; empty selects all.
    #[getter]
    fn select_by_names(&self) -> Vec<String> {
        self.inner.select_by_names().to_vec()
    }

    #[setter]
    fn set_select_by_names(&mut self, select_by_names: Vec<String>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_select_by_names(select_by_names);
        Ok(())
    }

    /// The partition equalities a read is pruned and filtered by; empty
    /// keeps every row. Values are spelled as partition paths spell them.
    #[getter]
    fn filter_partitions(&self) -> Vec<(String, String)> {
        self.inner.filter_partitions().to_vec()
    }

    #[setter]
    fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_filter_partitions(filter_partitions);
        Ok(())
    }

    /// The Avro block codec name, or `None` for another encoding.
    #[getter]
    fn block_codec(&self) -> Option<&str> {
        self.inner.avro_block_codec()
    }

    #[setter]
    fn set_block_codec(&mut self, block_codec: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_avro_block_codec(block_codec)
            .map_err(value_error)
    }

    /// The fixed sixteen-byte Avro synchronization marker, when one is set.
    #[getter]
    fn sync_marker<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .avro_sync_marker()
            .map(|marker| PyBytes::new(py, marker))
    }

    #[setter]
    fn set_sync_marker(&mut self, marker: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        self.require_mutable()?;
        let marker = marker.map(bytes_from_value).transpose()?;
        self.inner
            .set_avro_sync_marker(marker.as_deref())
            .map_err(value_error)
    }

    /// The page compression applied inside a Parquet file, if this is one.
    ///
    /// A setting one encoding has is absent on the others rather than invented,
    /// so this is `None` for an Arrow IPC stream, whose coding belongs to the
    /// handle instead.
    #[getter]
    fn compression(&self) -> Option<String> {
        self.inner.parquet_compression_name()
    }

    #[setter]
    fn set_compression(&mut self, compression: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_parquet_compression_name(compression)
            .map_err(value_error)
    }

    /// The maximum rows per row group, for the encodings that have them.
    #[getter]
    fn max_row_group_size(&self) -> Option<usize> {
        self.inner.parquet_max_row_group_size()
    }

    #[setter]
    fn set_max_row_group_size(&mut self, rows: usize) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_parquet_max_row_group_size(rows)
            .map_err(value_error)
    }

    /// The file-level metadata written into a footer, for the encodings that
    /// have one.
    #[getter]
    fn key_value_metadata<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(metadata) = self.inner.parquet_key_value_metadata() else {
            return Ok(None);
        };
        let pairs = PyDict::new(py);
        for (key, value) in metadata {
            pairs.set_item(key, value)?;
        }
        Ok(Some(pairs))
    }

    #[setter]
    fn set_key_value_metadata(&mut self, metadata: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_parquet_key_value_metadata(string_pairs_from_value(metadata)?)
            .map_err(value_error)
    }

    /// Return the deterministic hash of the complete native configuration.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&mut self) -> isize {
        self.hash_locked = true;
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let state = self.pickle_state(py)?;
        Ok(format!(
            "RecordOptions._from_pickle({})",
            state.repr()?.to_str()?
        ))
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (self.pickle_state(py)?.into_any().unbind(),),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}
