//! The one place a foreign columnar object becomes a native Arrow value.
//!
//! `PyArrow`, pandas, polars, `NumPy`, and anything implementing the Arrow C
//! data or stream protocol all describe the same four shapes the core's
//! [`ArrowValue`] already names. This module reads each of them once, at the
//! boundary, and hands the buffers to Rust - so the conversion, the cast, and
//! every later question about the value are the core's, not a per-library
//! path in Python.
//!
//! Nothing here reimplements a conversion. A frame is converted by its own
//! library, Arrow crossings use the C data and stream interfaces, and the
//! declared `Field` is applied by the core's one recursive cast.

use std::sync::Mutex;

use arrow_array::RecordBatch;
use arrow_pyarrow::{FromPyArrow, IntoPyArrow};
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{IntoPyDict, PyList};
use yggdryl::{ArrowCastOptions, ArrowShape, ArrowValue, Field as CoreField};

use crate::iomedia::{
    Frames, batch_reader_from_value, batch_reader_to_pyarrow, core_root_field_from_value,
    declared_by, frame_from_reader, frame_to_arrow, type_name,
};
use crate::types::datatype::{arrow_array_from_pyarrow, arrow_array_to_pyarrow};
use crate::types::field::{PyField, core_field_from_value};
use crate::types::scalar::{PyScalar, arrow_scalar_into_array, as_py_with_field};
use crate::{cast_options, value_error};

/// One Arrow-backed value: a scalar, a column, a table, or a stream.
///
/// A held shape - a scalar, a column, a table - can be read as often as it is
/// asked, because Arrow buffers are shared behind a pointer. A stream is
/// one-shot: the first method that narrows or exports it reads it, and asking
/// again is a `ValueError` rather than an empty answer.
#[pyclass(name = "ArrowValue", module = "yggdryl._native")]
pub(crate) struct PyArrowValue {
    inner: Mutex<Option<ArrowValue>>,
}

impl PyArrowValue {
    pub(crate) fn from_inner(inner: ArrowValue) -> Self {
        Self {
            inner: Mutex::new(Some(inner)),
        }
    }

    /// Take the value out, leaving a held shape behind and a stream consumed.
    fn take(&self) -> PyResult<ArrowValue> {
        let mut held = self
            .inner
            .lock()
            .map_err(|_| PyValueError::new_err("this ArrowValue is poisoned"))?;
        let value = held.take().ok_or_else(|| {
            PyValueError::new_err(
                "this ArrowValue held a stream, which crosses once and has already been read",
            )
        })?;
        // Arrow buffers live behind `Arc`, so a held shape is shared back at
        // the cost of a pointer and stays readable. A stream has nothing to
        // share until it is drained, so it is what one-shot means here.
        *held = shared(&value);
        Ok(value)
    }

    /// Read one answer off the value without consuming it.
    fn peek<T>(&self, read: impl FnOnce(&ArrowValue) -> T) -> PyResult<T> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PyValueError::new_err("this ArrowValue is poisoned"))?;
        guard.as_ref().map(read).ok_or_else(|| {
            PyValueError::new_err(
                "this ArrowValue was already consumed; an Arrow value crosses once",
            )
        })
    }
}

/// Read any Arrow-convertible Python object as one native Arrow value.
///
/// The order is deterministic and each step is one library's own conversion:
///
/// 1. a native `ArrowValue`, which is taken rather than copied;
/// 2. a pandas or polars frame or series, converted by that library;
/// 3. a `NumPy` array;
/// 4. a `PyArrow` container, whose exact class decides the shape;
/// 5. a `Dataset` or `Scanner`, which hand back a reader;
/// 6. anything exporting the Arrow C stream or array protocol;
/// 7. any other value, read as one native [`yggdryl::Scalar`].
///
/// A frame library is recognized by the value's own type rather than by
/// importing the library, so a caller who never installed pandas never pays an
/// import for it.
///
/// `field` is the declared shape. When it is given, the value is cast onto it
/// by the core's one recursive cast - one compiled plan for a stream, one
/// batch at a time - which is where a foreign runtime's types are narrowed to
/// what this project's schema says they are.
pub(crate) fn arrow_value_from_py(
    value: &Bound<'_, PyAny>,
    field: Option<&Bound<'_, PyAny>>,
    options: ArrowCastOptions,
) -> PyResult<ArrowValue> {
    let read = ingest(value)?;
    let Some(field) = field else {
        return Ok(read);
    };
    let declared = if read.shape().is_tabular() {
        core_root_field_from_value(field, read.field().name())?
    } else {
        core_field_from_value(field)?
    };
    read.cast(&declared, options).map_err(value_error)
}

fn ingest(value: &Bound<'_, PyAny>) -> PyResult<ArrowValue> {
    if let Ok(native) = value.extract::<PyRef<'_, PyArrowValue>>() {
        return native.take();
    }
    if let Some(series) = series_to_arrow(value)? {
        return array_of(&series);
    }
    if declared_by(value, "numpy", "ndarray") {
        return numpy_value(value);
    }
    // A held container is recognized by its exact class before the stream
    // ladder, because a `RecordBatch` also exports a stream and reading it as
    // one would lose the length it already knows.
    if let Some(value) = pyarrow_value(value)? {
        return Ok(value);
    }
    if let Some(reader) = columnar_reader(value)? {
        return ArrowValue::from_reader(reader).map_err(value_error);
    }
    if value.hasattr("__arrow_c_array__")? {
        return array_of(value);
    }
    native_value(value)
}

/// Read a batch stream out of a value that is already a source of rows.
///
/// This is the one recognition ladder for foreign row sources, shared by the
/// record write surface and by [`ingest`]: a pandas or polars frame, anything
/// exporting the Arrow C stream, and a dataset or scanner that hands one back.
/// `None` means the value names rows some other way, which is what leaves the
/// iterable paths to the caller. Nothing here consumes an iterator.
///
/// # Errors
///
/// Returns whatever an attribute lookup or a library conversion raised.
pub(crate) fn columnar_reader(
    value: &Bound<'_, PyAny>,
) -> PyResult<Option<yggdryl::arrow::BatchReader>> {
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

/// Convert one pandas or polars series to a `PyArrow` array, if it is one.
fn series_to_arrow<'py>(value: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    if declared_by(value, "polars", "Series") {
        return value.call_method0("to_arrow").map(Some);
    }
    if declared_by(value, "pandas", "Series") {
        // pandas does not export Arrow itself, so `PyArrow` - a dependency -
        // converts it, which behaves the same on every pandas release.
        return value
            .py()
            .import("pyarrow")?
            .getattr("Array")?
            .call_method1("from_pandas", (value,))
            .map(Some);
    }
    Ok(None)
}

/// Read one `NumPy` array as a column, or as rows when its dtype is a record.
fn numpy_value(value: &Bound<'_, PyAny>) -> PyResult<ArrowValue> {
    let py = value.py();
    let dimensions = value.getattr("ndim")?.extract::<usize>()?;
    if dimensions != 1 {
        return Err(PyTypeError::new_err(format!(
            "expected a one-dimensional numpy array; Arrow has no {dimensions}-dimensional \
             column, so reshape it or build a fixed_size_list column",
        )));
    }
    let pyarrow = py.import("pyarrow")?;
    let names = value.getattr("dtype")?.getattr("names")?;
    if names.is_none() {
        return array_of(&pyarrow.getattr("array")?.call1((value,))?);
    }
    // A record dtype names its members, and each member is a column: NumPy
    // interleaves them in one buffer, so `PyArrow` converts them one at a
    // time rather than reading the record as a struct column.
    let names = names.try_iter()?.collect::<PyResult<Vec<_>>>()?;
    let mut columns = Vec::with_capacity(names.len());
    for name in &names {
        columns.push(
            pyarrow
                .getattr("array")?
                .call1((value.get_item(name)?,))?,
        );
    }
    let batch = pyarrow.getattr("RecordBatch")?.call_method(
        "from_arrays",
        (PyList::new(py, columns)?,),
        Some(&[("names", PyList::new(py, names)?)].into_py_dict(py)?),
    )?;
    batch_of(&batch)
}

/// Read one `PyArrow` container by its exact class, or report it is not one.
///
/// The class decides the shape, which is what keeps a held table from being
/// reported as one row and a stream from claiming a length it does not know.
fn pyarrow_value(value: &Bound<'_, PyAny>) -> PyResult<Option<ArrowValue>> {
    let Ok(pyarrow) = value.py().import("pyarrow") else {
        return Ok(None);
    };
    if value.is_instance(&pyarrow.getattr("RecordBatch")?)? {
        return batch_of(value).map(Some);
    }
    // A table may hold many chunks, and the C stream hands them over without
    // combining them, so it crosses as a stream rather than as a copy.
    if value.is_instance(&pyarrow.getattr("Table")?)?
        || value.is_instance(&pyarrow.getattr("RecordBatchReader")?)?
    {
        return stream_of(value).map(Some);
    }
    if value.is_instance(&pyarrow.getattr("ChunkedArray")?)? {
        // A column is one buffer set, so the chunks are combined here rather
        // than silently reporting only the first.
        return array_of(&value.call_method0("combine_chunks")?).map(Some);
    }
    if value.is_instance(&pyarrow.getattr("Array")?)? {
        return array_of(value).map(Some);
    }
    if value.is_instance(&pyarrow.getattr("Scalar")?)? {
        let array = arrow_scalar_into_array(value)?;
        let field = inferred_field(&array, "value")?;
        return ArrowValue::from_scalar_array(field, array)
            .map(Some)
            .map_err(value_error);
    }
    Ok(None)
}

/// Read any other Python value as one native scalar under its inferred Field.
fn native_value(value: &Bound<'_, PyAny>) -> PyResult<ArrowValue> {
    let scalar = crate::types::scalar::from_py(value).map_err(|error| {
        PyTypeError::new_err(format!(
            "expected a pyarrow Scalar, Array, ChunkedArray, RecordBatch, Table, \
             RecordBatchReader, Dataset or Scanner, a pandas or polars frame or series, a numpy \
             array, an Arrow C data or stream exporter, or a value a Scalar can hold, got {}: \
             {error}",
            type_name(value)
        ))
    })?;
    let field = scalar.inferred_scalar_field().map_err(value_error)?;
    ArrowValue::from_value(&field, &scalar).map_err(value_error)
}

/// Share a held value's buffers, or report that a stream has none to share.
fn shared(value: &ArrowValue) -> Option<ArrowValue> {
    let field = value.field().clone();
    match value.shape() {
        ArrowShape::Scalar => ArrowValue::from_scalar_array(field, value.as_array()?.clone()).ok(),
        ArrowShape::Array => ArrowValue::from_array(field, value.as_array()?.clone()).ok(),
        ArrowShape::Batch => ArrowValue::from_batch_as(field, value.as_batch()?.clone()).ok(),
        ArrowShape::Stream => None,
    }
}

fn stream_of(value: &Bound<'_, PyAny>) -> PyResult<ArrowValue> {
    ArrowValue::from_reader(batch_reader_from_value(value)?).map_err(value_error)
}

fn batch_of(value: &Bound<'_, PyAny>) -> PyResult<ArrowValue> {
    ArrowValue::from_batch(RecordBatch::from_pyarrow_bound(value)?).map_err(value_error)
}

fn array_of(value: &Bound<'_, PyAny>) -> PyResult<ArrowValue> {
    let array = arrow_array_from_pyarrow(value)?;
    let field = inferred_field(&array, "item")?;
    ArrowValue::from_array(field, array).map_err(value_error)
}

/// Name the Field one foreign column proves about itself.
fn inferred_field(array: &arrow_array::ArrayRef, name: &str) -> PyResult<CoreField> {
    let dtype = yggdryl::DataType::try_from(array.data_type().clone()).map_err(value_error)?;
    Ok(CoreField::new(name, dtype, array.null_count() != 0))
}

#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
#[pymethods]
impl PyArrowValue {
    /// Read any Arrow-convertible object, optionally cast onto `field`.
    #[new]
    #[pyo3(signature = (value, field=None, *, safe=true, nullability="default", representation="value"))]
    fn new(
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Self> {
        Ok(Self::from_inner(arrow_value_from_py(
            value,
            field,
            cast_options(safe, nullability, representation)?,
        )?))
    }

    /// Read any Arrow-convertible object, optionally cast onto `field`.
    ///
    /// This is the one entry point every foreign columnar object crosses:
    /// `PyArrow` containers, pandas and polars frames and series, `NumPy`
    /// arrays, Arrow C data and stream exporters, datasets and scanners, and
    /// any value a `Scalar` can hold.
    #[staticmethod]
    #[pyo3(signature = (value, field=None, *, safe=true, nullability="default", representation="value"))]
    fn from_py(
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Self> {
        Self::new(value, field, safe, nullability, representation)
    }

    /// Which of Arrow's four payload shapes this value holds.
    #[getter]
    fn shape(&self) -> PyResult<&'static str> {
        self.peek(|value| value.shape().as_str())
    }

    /// The exact Field this value is typed by.
    #[getter]
    fn field(&self) -> PyResult<PyField> {
        self.peek(|value| PyField::from_inner(value.field().clone()))
    }

    /// The number of rows, or `None` for a stream that has not been drained.
    #[getter]
    fn row_size(&self) -> PyResult<Option<usize>> {
        self.peek(ArrowValue::row_size)
    }

    /// The number of columns one row carries.
    #[getter]
    fn column_size(&self) -> PyResult<usize> {
        self.peek(ArrowValue::column_size)
    }

    /// Report whether reading this value consumes an undrained stream.
    #[getter]
    fn is_streamed(&self) -> PyResult<bool> {
        self.peek(|value| value.shape().is_streamed())
    }

    /// Report whether this value's stream has already been read.
    ///
    /// Only a stream is ever consumed: a scalar, a column, and a table share
    /// their buffers back and stay readable.
    #[getter]
    fn is_consumed(&self) -> PyResult<bool> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| PyValueError::new_err("this ArrowValue is poisoned"))?
            .is_none())
    }

    /// Reshape this value onto another Field, keeping its shape.
    #[pyo3(signature = (field, *, safe=true, nullability="default", representation="value"))]
    fn cast(
        &self,
        field: &Bound<'_, PyAny>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Self> {
        let value = self.take()?;
        let declared = if value.shape().is_tabular() {
            core_root_field_from_value(field, value.field().name())?
        } else {
            core_field_from_value(field)?
        };
        value
            .cast(&declared, cast_options(safe, nullability, representation)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Hand this value to `PyArrow` as one `Scalar`.
    fn into_arrow_scalar<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let value = self.take()?;
        let field = value.field().clone();
        let array = value.into_array().map_err(value_error)?;
        if array.len() != 1 {
            return Err(PyValueError::new_err(format!(
                "an Arrow scalar holds exactly one row, got {}",
                array.len()
            )));
        }
        arrow_array_to_pyarrow(py, &array, Some(&field))?.get_item(0)
    }

    /// Hand this value to `PyArrow` as one `Array`.
    fn into_arrow_array<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let value = self.take()?;
        let field = value.field().clone();
        let array = value.into_array().map_err(value_error)?;
        arrow_array_to_pyarrow(py, &array, Some(&field))
    }

    /// Hand this value to `PyArrow` as one `RecordBatch`.
    fn into_arrow_batch<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.take()?
            .into_batch()
            .map_err(value_error)?
            .into_pyarrow(py)
    }

    /// Hand this value to `PyArrow` as one `Table`, over its own stream.
    fn into_arrow_table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.into_arrow_reader(py)?.call_method0("read_all")
    }

    /// Hand this value to `PyArrow` as one `RecordBatchReader`.
    ///
    /// This is the shape every record write takes, so it is also the cheapest
    /// crossing: nothing is collected and the C stream carries the buffers.
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.take()?.into_reader().map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Hand this value to pandas as one frame.
    fn into_pandas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.take()?.into_reader().map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Pandas)
    }

    /// Hand this value to polars as one frame.
    fn into_polars<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.take()?.into_reader().map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Polars)
    }

    /// Hand this value to `NumPy` as one array.
    ///
    /// A column becomes a one-dimensional array. `NumPy` has no counterpart
    /// for Arrow's null mask or its nested layouts, so the crossing is
    /// allowed to copy and `PyArrow`'s own conversion decides what each value
    /// becomes - a null becomes `nan`, and rows become an object array of
    /// mappings rather than a record array.
    fn into_numpy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let value = self.take()?;
        let field = value.field().clone();
        let array = value.into_array().map_err(value_error)?;
        arrow_array_to_pyarrow(py, &array, Some(&field))?.call_method(
            "to_numpy",
            (),
            Some(&[("zero_copy_only", false)].into_py_dict(py)?),
        )
    }

    /// Cross into the native value model.
    fn into_scalar(&self) -> PyResult<PyScalar> {
        self.take()?
            .into_scalar()
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Convert to Python's own scalar and collection types.
    ///
    /// The Field names what it types, so a struct row reads as a `dict` and a
    /// table as a list of them - never as a positional list a caller would
    /// have to zip against the schema themselves.
    fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let value = self.take()?;
        let field = value.field().clone();
        let listed = value.shape() != ArrowShape::Scalar;
        let scalar = value.into_scalar().map_err(value_error)?;
        if !listed {
            return as_py_with_field(py, &scalar, &field);
        }
        let items = PyList::empty(py);
        for item in scalar.as_sequence().unwrap_or_default() {
            items.append(as_py_with_field(py, item, &field)?)?;
        }
        Ok(items.into_any().unbind())
    }

    fn __repr__(&self) -> PyResult<String> {
        let consumed = self
            .inner
            .lock()
            .map_err(|_| PyValueError::new_err("this ArrowValue is poisoned"))?;
        Ok(match consumed.as_ref() {
            None => "ArrowValue(consumed)".to_owned(),
            Some(value) => format!(
                "ArrowValue({}, {}, rows={})",
                value.shape(),
                value.field(),
                value
                    .row_size()
                    .map_or_else(|| "?".to_owned(), |rows| rows.to_string()),
            ),
        })
    }
}

/// Name every shape an Arrow value can hold, in widening order.
#[pyfunction]
pub(crate) fn arrow_shapes(py: Python<'_>) -> PyResult<Py<PyAny>> {
    Ok(PyList::new(py, ArrowShape::ALL.map(ArrowShape::as_str))?
        .into_any()
        .unbind())
}
