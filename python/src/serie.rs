//! Python's native view of the shared [`Serie`]: many values, as a
//! schema-free run or as the Arrow buffers of one field.
//!
//! [`PySerie`] owns only the core value and redirects every verb to it. It is
//! mutable - a write lands in the buffers the column holds - so it has
//! equality over its rows and no hash, which is Python's contract for a
//! mutable container. Arrow crosses by sharing buffers through the C Data and
//! C Stream interfaces.
//!
//! This is also the one place a foreign columnar object becomes a native
//! value. `PyArrow`, pandas, polars, `NumPy` and anything implementing the
//! Arrow C data or stream protocol is read once, by [`columnar`], as a column
//! in hand or as a stream: a held column is a [`PySerie`], a stream is a
//! [`PySerieReader`], and nothing else. A frame is converted by its own
//! library, and the declared `Field` is applied by the core's one cast.
//!
//! A nested column is a subclass named for its leaf - `SerieSerie`,
//! `LargeSerieSerie`, `SerieViewSerie`, `LargeSerieViewSerie`,
//! `FixedSizeSerieSerie`, `MapSerie`, `StructSerie` - adding the verbs that
//! leaf lends: its offsets, the rows it cuts, its entries. Every serie handed
//! out goes through [`described`], so a record's child, a serie's items and
//! one row of them come back as their own leaf, all the way down. A write
//! never changes a column's leaf, so the class an object has stays true.

use std::borrow::Cow;

use pyo3::IntoPyObjectExt;
use pyo3::PyClassInitializer;
use pyo3::class::basic::CompareOp;
use std::sync::Mutex;

use arrow_array::RecordBatch;
use arrow_pyarrow::FromPyArrow;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{
    IntoPyDict, PyFrozenSet, PyIterator, PyList, PyMapping, PySequence, PySet, PySlice,
};
use yggdryl::arrow::BatchReader;
use yggdryl::media::RecordOptions;
use yggdryl::{
    ArrowCastOptions, Field as CoreField, FieldPath, MimeType, Scalar, Serie, SerieReader,
};

use crate::datatype::{PyDataType, arrow_array_from_pyarrow, arrow_array_to_pyarrow};
use crate::field::{PyField, core_field_from_value};
use crate::iomedia::{
    Frames, batch_reader_from_any, batch_reader_from_record_sequence, batch_reader_from_value,
    batch_reader_to_pyarrow, batch_to_pyarrow, columnar_reader, core_root_field_from_value,
    declared_by, frame_from_reader, record_batch_from_value, type_name,
};
use crate::scalar::{
    PyScalar, PyScalarIterator, as_py, as_py_with_field, from_py, pyarrow_scalar_into_array,
};
use crate::{cast_options, compare, normalize_index, value_error};

/// Many values: a schema-free run, or the Arrow buffers of one field.
#[pyclass(
    name = "Serie",
    module = "yggdryl._native",
    subclass,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PySerie {
    pub(crate) inner: Serie,
}

impl PySerie {
    pub(crate) const fn from_inner(inner: Serie) -> Self {
        Self { inner }
    }

    /// Resolve a Python index against the serie, negative from the end.
    fn index(&self, index: isize) -> PyResult<usize> {
        normalize_index(index, self.inner.len()).ok_or_else(|| PyIndexError::new_err(index))
    }
}

/// The invariant [`described`] keeps: a nested class is built only over the
/// leaf it is named for, and no write changes a column's leaf.
const LEAF: &str = "a nested Serie class holds the leaf it is named for";

/// Hand `serie` to Python as the class its leaf is named for.
pub(crate) fn described(py: Python<'_>, serie: Serie) -> PyResult<Py<PyAny>> {
    let leaf = Leaf::of(&serie);
    let base = PyClassInitializer::from(PySerie::from_inner(serie));
    Ok(match leaf {
        Leaf::Other => Py::new(py, base)?.into_any(),
        Leaf::Serie => Py::new(py, base.add_subclass(PySerieSerie))?.into_any(),
        Leaf::LargeSerie => Py::new(py, base.add_subclass(PyLargeSerieSerie))?.into_any(),
        Leaf::SerieView => Py::new(py, base.add_subclass(PySerieViewSerie))?.into_any(),
        Leaf::LargeSerieView => Py::new(py, base.add_subclass(PyLargeSerieViewSerie))?.into_any(),
        Leaf::FixedSizeSerie => Py::new(py, base.add_subclass(PyFixedSizeSerieSerie))?.into_any(),
        Leaf::Map => Py::new(py, base.add_subclass(PyMapSerie))?.into_any(),
        Leaf::Struct => Py::new(py, base.add_subclass(PyStructSerie))?.into_any(),
    })
}

/// Hand every serie of `series` to Python as its own class.
fn described_all<'a>(
    py: Python<'_>,
    series: impl IntoIterator<Item = &'a Serie>,
) -> PyResult<Vec<Py<PyAny>>> {
    series
        .into_iter()
        .map(|serie| described(py, serie.clone()))
        .collect()
}

/// What `_from_pickle` takes back: the rows, and the field they are under.
type PickleArguments = (Vec<PyScalar>, Option<PyField>);

/// Which class a serie is handed out as.
#[derive(Clone, Copy)]
enum Leaf {
    Other,
    Serie,
    LargeSerie,
    SerieView,
    LargeSerieView,
    FixedSizeSerie,
    Map,
    Struct,
}

impl Leaf {
    fn of(serie: &Serie) -> Self {
        match serie {
            Serie::Serie(_) => Self::Serie,
            Serie::LargeSerie(_) => Self::LargeSerie,
            Serie::SerieView(_) => Self::SerieView,
            Serie::LargeSerieView(_) => Self::LargeSerieView,
            Serie::FixedSizeSerie(_) => Self::FixedSizeSerie,
            Serie::Map(_) | Serie::SortedMap(_) => Self::Map,
            Serie::Struct(_) => Self::Struct,
            _ => Self::Other,
        }
    }
}

/// Convert every Python value in `rows` to a core value, once.
fn rows_from_py(rows: &Bound<'_, PyAny>) -> PyResult<Vec<Scalar>> {
    if let Ok(serie) = rows.extract::<PyRef<'_, PySerie>>() {
        return Ok(serie.inner.rows().into_owned());
    }
    rows.try_iter()?
        .map(|row| from_py(&row?))
        .collect::<PyResult<Vec<Scalar>>>()
}

/// Resolve an optional Python field argument once.
fn field_of(field: Option<&Bound<'_, PyAny>>) -> PyResult<Option<CoreField>> {
    field.map(core_field_from_value).transpose()
}

/// Resolve a cast target once: a field as it is spelled, or a bare
/// `DataType` as the required column named `value` it declares.
fn target_of(target: &Bound<'_, PyAny>) -> PyResult<CoreField> {
    if let Ok(dtype) = target.extract::<PyRef<'_, PyDataType>>() {
        return Ok(dtype.inner.clone().required_field("value"));
    }
    core_field_from_value(target)
}

/// Read anything that streams record batches: a reader, a table, a batch,
/// a frame, a dataset or scanner, or an iterable of any of those, pulled
/// one item at a time.
pub(crate) fn stream_of(value: &Bound<'_, PyAny>) -> PyResult<BatchReader> {
    batch_reader_from_any(
        value,
        &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).map_err(value_error)?,
    )
}

/// A foreign columnar object, read once at the boundary.
pub(crate) enum Columnar {
    /// A column in hand, its buffers shared.
    Held(Serie),
    /// One Arrow scalar: a column of one row, which as a value is that row.
    Pinned(Serie),
    /// A batch stream, not yet pulled.
    Stream(BatchReader),
}

impl Columnar {
    /// The column this object holds, draining a stream under `root`.
    fn into_serie(self, root: Option<&CoreField>, options: ArrowCastOptions) -> PyResult<Serie> {
        match self {
            Self::Held(serie) | Self::Pinned(serie) => match root {
                Some(root) => serie.cast(root, options).map_err(value_error),
                None => Ok(serie),
            },
            Self::Stream(reader) => {
                Serie::from_arrow_reader(root, reader, options).map_err(value_error)
            }
        }
    }

    /// The stream this object is: a held column as its one batch, and a
    /// stream as it stands, neither pulled.
    fn into_reader(
        self,
        root: Option<&CoreField>,
        options: ArrowCastOptions,
    ) -> PyResult<SerieReader> {
        match self {
            Self::Held(serie) | Self::Pinned(serie) => {
                let reader = SerieReader::from_serie(serie).map_err(value_error)?;
                match root {
                    Some(root) => SerieReader::from_arrow_reader(
                        Some(root),
                        reader.into_arrow_reader(),
                        options,
                    )
                    .map_err(value_error),
                    None => Ok(reader),
                }
            }
            Self::Stream(reader) => {
                SerieReader::from_arrow_reader(root, reader, options).map_err(value_error)
            }
        }
    }
}

/// Read a columnar Python object, or answer `None` for a value that is not
/// one.
///
/// The order is deterministic and each step is one library's own conversion:
///
/// 1. a native `Serie`, shared, or a native `SerieReader`, taken;
/// 2. a pandas or polars series, converted by that library;
/// 3. a `NumPy` array;
/// 4. a `PyArrow` container, whose exact class decides held or streamed;
/// 5. a frame, a dataset or a scanner, or any Arrow C stream exporter;
/// 6. anything exporting the Arrow C array protocol.
///
/// A frame library is recognized by the value's own type rather than by
/// importing the library, so a caller who never installed pandas never pays
/// an import for it.
///
/// # Errors
///
/// Returns whatever a library conversion or an Arrow C crossing raised, and a
/// `ValueError` for a `SerieReader` already handed over.
pub(crate) fn columnar(value: &Bound<'_, PyAny>) -> PyResult<Option<Columnar>> {
    if let Ok(serie) = value.extract::<PyRef<'_, PySerie>>() {
        return Ok(Some(Columnar::Held(serie.inner.clone())));
    }
    if let Ok(mut reader) = value.extract::<PyRefMut<'_, PySerieReader>>() {
        return Ok(Some(Columnar::Stream(reader.take()?.into_arrow_reader())));
    }
    if let Some(series) = series_to_arrow(value)? {
        return array_of(&series).map(Some);
    }
    if declared_by(value, "numpy", "ndarray") {
        return numpy_value(value).map(Some);
    }
    // A held container is recognized by its exact class before the stream
    // ladder, because a `RecordBatch` also exports a stream and reading it as
    // one would lose the length it already knows.
    if let Some(value) = pyarrow_value(value)? {
        return Ok(Some(value));
    }
    if let Some(reader) = columnar_reader(value)? {
        return Ok(Some(Columnar::Stream(reader)));
    }
    if value.hasattr("__arrow_c_array__")? {
        return array_of(value).map(Some);
    }
    Ok(None)
}

/// Read a columnar object as the one column it holds, a stream drained.
///
/// This is what `Scalar.from_` holds a columnar argument as: a serie sharing
/// the column's buffers, its rows unread. One Arrow scalar is its row.
///
/// # Errors
///
/// [`columnar`]'s, and a stream's own.
pub(crate) fn columnar_value(value: &Bound<'_, PyAny>) -> PyResult<Option<Scalar>> {
    Ok(match columnar(value)? {
        None => None,
        Some(Columnar::Pinned(serie)) => Some(serie.scalar(0).map_err(value_error)?),
        Some(columnar) => Some(Scalar::from(
            columnar.into_serie(None, ArrowCastOptions::new())?,
        )),
    })
}

/// Read a Python value already proven not to be columnar as one column.
///
/// The value crosses through the native [`Scalar`] boundary exactly once: a
/// sequence is its rows and anything else one row, under `field` or the field
/// the value infers.
fn serie_from_value(
    value: &Bound<'_, PyAny>,
    field: Option<&Bound<'_, PyAny>>,
    options: ArrowCastOptions,
) -> PyResult<Serie> {
    let scalar = from_py(value).map_err(|error| {
        PyTypeError::new_err(format!(
            "expected a pyarrow Scalar, Array, ChunkedArray, RecordBatch, Table, \
             RecordBatchReader, Dataset or Scanner, a pandas or polars frame or series, a numpy \
             array, an Arrow C data or stream exporter, or a value a Scalar can hold, got {}: \
             {error}",
            type_name(value)
        ))
    })?;
    let declared = field
        .map(|field| core_root_field_from_value(field, DEFAULT_ROOT))
        .transpose()?;
    match scalar.as_serie() {
        // A column already names its layout, so a declared field casts it.
        Some(rows) if rows.is_column() => match declared {
            Some(field) => rows.cast(&field, options).map_err(value_error),
            None => Ok(rows.clone()),
        },
        Some(rows) => {
            let field = match declared {
                Some(field) => field,
                None => scalar.inferred_array_field().map_err(value_error)?,
            };
            Serie::from_scalars(field, rows.rows().into_owned()).map_err(value_error)
        }
        None => {
            let field = match declared {
                Some(field) => field,
                None => scalar.inferred_scalar_field().map_err(value_error)?,
            };
            Serie::from_scalars(field, [scalar]).map_err(value_error)
        }
    }
}

/// Read any Python value as one column, cast into `field` when one is given.
///
/// A columnar object is [`columnar`]'s; any other value is
/// [`serie_from_value`]'s.
fn serie_from_py(
    value: &Bound<'_, PyAny>,
    field: Option<&Bound<'_, PyAny>>,
    options: ArrowCastOptions,
) -> PyResult<Serie> {
    if let Some(columnar) = columnar(value)? {
        let name = match &columnar {
            Columnar::Held(serie) | Columnar::Pinned(serie) => serie
                .field()
                .map_or(DEFAULT_ROOT, CoreField::name)
                .to_owned(),
            Columnar::Stream(_) => DEFAULT_ROOT.to_owned(),
        };
        let root = field
            .map(|field| core_root_field_from_value(field, &name))
            .transpose()?;
        return columnar.into_serie(root.as_ref(), options);
    }
    serie_from_value(value, field, options)
}

/// Read any Python value as a stream of record columns, cast into `root`.
///
/// A columnar object is [`columnar`]'s. Iterators and generic reusable
/// iterables remain row streams, pulled one batch at a time. Sequences of
/// mappings or batch sources keep the record reader's interpretation. Other
/// concrete containers, a native [`PyScalar`], and a non-iterable cross through
/// [`serie_from_value`], then become the one held item of their stream.
pub(crate) fn serie_reader_from_py(
    value: &Bound<'_, PyAny>,
    root: Option<&Bound<'_, PyAny>>,
    options: ArrowCastOptions,
) -> PyResult<SerieReader> {
    let root = root
        .map(|root| core_root_field_from_value(root, DEFAULT_ROOT))
        .transpose()?;
    if let Some(columnar) = columnar(value)? {
        return columnar.into_reader(root.as_ref(), options);
    }
    if value.cast::<PySequence>().is_ok()
        && let Some(reader) = batch_reader_from_record_sequence(
            value,
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).map_err(value_error)?,
        )?
    {
        return SerieReader::from_arrow_reader(root.as_ref(), reader, options).map_err(value_error);
    }
    let streams = value.cast::<PyIterator>().is_ok()
        || (value.cast::<PySequence>().is_err()
            && value.cast::<PyMapping>().is_err()
            && value.cast::<PySet>().is_err()
            && value.cast::<PyFrozenSet>().is_err()
            && value.extract::<PyRef<'_, PyScalar>>().is_err()
            && value.hasattr("__iter__")?);
    if streams {
        return SerieReader::from_arrow_reader(root.as_ref(), stream_of(value)?, options)
            .map_err(value_error);
    }
    let serie = serie_from_value(value, None, ArrowCastOptions::new())?;
    Columnar::Held(serie).into_reader(root.as_ref(), options)
}

/// The name a record root takes when its spelling carries none, because
/// Arrow names columns and never the record.
const DEFAULT_ROOT: &str = "row";

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
fn numpy_value(value: &Bound<'_, PyAny>) -> PyResult<Columnar> {
    let py = value.py();
    let dimensions = value.getattr("ndim")?.extract::<usize>()?;
    if dimensions != 1 {
        return Err(PyTypeError::new_err(format!(
            "expected a one-dimensional numpy array; Arrow has no {dimensions}-dimensional \
             column, so reshape it or build a fixed_size_serie column",
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
        columns.push(pyarrow.getattr("array")?.call1((value.get_item(name)?,))?);
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
fn pyarrow_value(value: &Bound<'_, PyAny>) -> PyResult<Option<Columnar>> {
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
        return Ok(Some(Columnar::Stream(batch_reader_from_value(value)?)));
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
        let array = pyarrow_scalar_into_array(value)?;
        let serie =
            Serie::from_arrow_array(None, array, ArrowCastOptions::new()).map_err(value_error)?;
        // A pinned row is a value, so its column is named as one.
        let field = serie.require_field().map_err(value_error)?.clone();
        let serie = serie
            .cast(&field.with_name("value"), ArrowCastOptions::new())
            .map_err(value_error)?;
        return Ok(Some(Columnar::Pinned(serie)));
    }
    Ok(None)
}

fn batch_of(value: &Bound<'_, PyAny>) -> PyResult<Columnar> {
    Serie::from_arrow_batch(
        None,
        &RecordBatch::from_pyarrow_bound(value)?,
        ArrowCastOptions::new(),
    )
    .map(Columnar::Held)
    .map_err(value_error)
}

/// One foreign column as the column of its own layout: the field it proves
/// about itself, named `item`, and its buffers, shared.
fn array_of(value: &Bound<'_, PyAny>) -> PyResult<Columnar> {
    Serie::from_arrow_array(
        None,
        arrow_array_from_pyarrow(value)?,
        ArrowCastOptions::new(),
    )
    .map(Columnar::Held)
    .map_err(value_error)
}

#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
#[pymethods]
impl PySerie {
    /// A schema-free run of `values`, each converted once; a column is
    /// built by [`Self::from_scalars`] and the Arrow doors.
    #[new]
    #[pyo3(signature = (values = None))]
    fn new(values: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let rows = values.map(rows_from_py).transpose()?.unwrap_or_default();
        Ok(Self::from_inner(Serie::new(rows)))
    }

    /// The column `field` types `rows` into, each through its contract once.
    #[staticmethod]
    fn from_scalars(
        py: Python<'_>,
        field: &Bound<'_, PyAny>,
        rows: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let serie = Serie::from_scalars(core_field_from_value(field)?, rows_from_py(rows)?)
            .map_err(value_error)?;
        described(py, serie)
    }

    /// The empty column of `field`.
    #[staticmethod]
    fn empty(py: Python<'_>, field: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        described(
            py,
            Serie::empty(core_field_from_value(field)?).map_err(value_error)?,
        )
    }

    /// The empty column of `field`, with room for `rows` rows.
    #[staticmethod]
    fn with_capacity(py: Python<'_>, field: &Bound<'_, PyAny>, rows: usize) -> PyResult<Py<PyAny>> {
        described(
            py,
            Serie::with_capacity(core_field_from_value(field)?, rows).map_err(value_error)?,
        )
    }

    /// Take one Arrow array as a column: of its own field, or cast into
    /// `field`.
    ///
    /// With no field the column is named `item` and typed by what the array
    /// proves about itself; with one, an exact layout shares the buffers and
    /// any other is cast under the three options.
    #[staticmethod]
    #[pyo3(signature = (array, field = None, *, safe = true, nullability = "default", representation = "value"))]
    fn from_arrow_array(
        py: Python<'_>,
        array: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, nullability, representation)?;
        let array = arrow_array_from_pyarrow(array)?;
        let field = field_of(field)?;
        described(
            py,
            Serie::from_arrow_array(field.as_ref(), array, options).map_err(value_error)?,
        )
    }

    /// Take one record batch as a record column: of its own schema, under
    /// the root `row`, or cast into `root`.
    #[staticmethod]
    #[pyo3(signature = (batch, root = None, *, safe = true, nullability = "default", representation = "value"))]
    fn from_arrow_batch(
        py: Python<'_>,
        batch: &Bound<'_, PyAny>,
        root: Option<&Bound<'_, PyAny>>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, nullability, representation)?;
        let root = field_of(root)?;
        let batch = record_batch_from_value(batch)?;
        described(
            py,
            Serie::from_arrow_batch(root.as_ref(), &batch, options).map_err(value_error)?,
        )
    }

    /// Drain a record batch stream into one record column: of its own
    /// schema, or cast into `root` by one plan.
    #[staticmethod]
    #[pyo3(signature = (reader, root = None, *, safe = true, nullability = "default", representation = "value"))]
    fn from_arrow_reader(
        py: Python<'_>,
        reader: &Bound<'_, PyAny>,
        root: Option<&Bound<'_, PyAny>>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, nullability, representation)?;
        let root = field_of(root)?;
        let reader = stream_of(reader)?;
        described(
            py,
            Serie::from_arrow_reader(root.as_ref(), reader, options).map_err(value_error)?,
        )
    }

    /// `rows` copies of `field`'s canonical default, laid out once.
    #[staticmethod]
    #[pyo3(signature = (field, rows = 1))]
    fn from_default(py: Python<'_>, field: &Bound<'_, PyAny>, rows: usize) -> PyResult<Py<PyAny>> {
        described(
            py,
            Serie::from_default(core_field_from_value(field)?, rows).map_err(value_error)?,
        )
    }

    /// Read any columnar object, or any value a `Scalar` holds, as one
    /// column: of its own field, or cast into `field`.
    ///
    /// A `pyarrow` array, chunked array, batch or table, a pandas or polars
    /// frame or series, a `NumPy` array and any Arrow C exporter each cross
    /// by their own conversion, buffers shared; a stream is drained. Any
    /// other value is read as a `Scalar`: a sequence is its rows, and
    /// anything else one row.
    #[staticmethod]
    #[pyo3(name = "from_")]
    #[pyo3(signature = (value, field = None, *, safe = true, nullability = "default", representation = "value"))]
    fn from_(
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, nullability, representation)?;
        described(py, serie_from_py(value, field, options)?)
    }

    /// Rebuild a pickled serie: its rows, under its field when it had one.
    #[staticmethod]
    #[pyo3(signature = (rows, field = None))]
    fn _from_pickle(
        py: Python<'_>,
        rows: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let rows = rows_from_py(rows)?;
        let serie = match field_of(field)? {
            Some(field) => Serie::from_scalars(field, rows).map_err(value_error)?,
            None => Serie::new(rows),
        };
        described(py, serie)
    }

    /// The field a column carries, or `None` for a run.
    #[getter]
    fn field(&self) -> Option<PyField> {
        self.inner.field().cloned().map(PyField::from_inner)
    }

    /// `serie(<the field named item>)` for a column; agreed out of a run's rows.
    #[getter]
    fn dtype(&self) -> PyResult<PyDataType> {
        self.inner
            .dtype()
            .map(PyDataType::from_inner)
            .map_err(value_error)
    }

    /// Whether this is a column rather than a schema-free run.
    #[getter]
    fn is_column(&self) -> bool {
        self.inner.is_column()
    }

    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn is_null(&self, index: usize) -> PyResult<bool> {
        self.inner.is_null(index).map_err(value_error)
    }

    /// Row `index`, built as one value.
    fn scalar(&self, index: usize) -> PyResult<PyScalar> {
        self.inner
            .scalar(index)
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Row `index`, or `None` past the end.
    fn get(&self, index: usize) -> Option<PyScalar> {
        self.inner
            .get(index)
            .map(|row| PyScalar::from_inner(row.into_owned()))
    }

    /// Every row, each built once and kept by the list alone.
    fn rows(&self) -> Vec<PyScalar> {
        self.inner
            .rows()
            .into_owned()
            .into_iter()
            .map(PyScalar::from_inner)
            .collect()
    }

    /// Every row as the Python value it is, a record as a `dict`.
    fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        match self.inner.field() {
            Some(field) => {
                for row in &self.inner {
                    list.append(as_py_with_field(py, &row, field)?)?;
                }
            }
            None => {
                for row in &self.inner {
                    list.append(as_py(py, &row)?)?;
                }
            }
        }
        Ok(list.into_any().unbind())
    }

    /// This serie as the one value a `Scalar` sequence holds.
    fn into_scalar(&self) -> PyScalar {
        PyScalar::from_inner(Scalar::from(self.inner.clone()))
    }

    /// The run of this serie's rows, dropping the field.
    fn into_run(&self) -> Self {
        Self::from_inner(Serie::from(self.inner.clone().into_run()))
    }

    /// `length` rows from `offset`, sharing a column's buffers.
    fn slice(&self, py: Python<'_>, offset: usize, length: usize) -> PyResult<Py<PyAny>> {
        described(py, self.inner.slice(offset, length).map_err(value_error)?)
    }

    /// A record column's child named `name`, or `None`.
    fn child(&self, py: Python<'_>, name: &str) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .child(name)
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// A record column's child, or a union's member, at `index`.
    fn child_at(&self, py: Python<'_>, index: usize) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .child_at(index)
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// Every child of a record column, or member of a union.
    fn children(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        described_all(py, self.inner.children())
    }

    /// A sequence column's items, a mapping's entries, an encoding's values.
    fn items(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .items()
            .map(|items| described(py, items.clone()))
            .transpose()
    }

    /// The column `path` reaches, spelled as a field path.
    fn get_child_by_path(&self, py: Python<'_>, path: &str) -> PyResult<Option<Py<PyAny>>> {
        let path = FieldPath::from_str(path).map_err(value_error)?;
        self.inner
            .get_child_by_path(&path)
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// Replace rows `start..end` by `rows`: the one mutation.
    fn splice(&mut self, start: usize, end: usize, rows: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .splice(start..end, rows_from_py(rows)?)
            .map_err(value_error)
    }

    fn set(&mut self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.set(index, from_py(value)?).map_err(value_error)
    }

    fn push(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.push(from_py(value)?).map_err(value_error)
    }

    fn insert(&mut self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .insert(index, from_py(value)?)
            .map_err(value_error)
    }

    /// Remove row `index` and answer it.
    fn remove(&mut self, index: usize) -> PyResult<PyScalar> {
        self.inner
            .remove(index)
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Remove the last row and answer it, or `None` when empty.
    fn pop(&mut self) -> PyResult<Option<PyScalar>> {
        self.inner
            .pop()
            .map(|row| row.map(PyScalar::from_inner))
            .map_err(value_error)
    }

    fn truncate(&mut self, len: usize) -> PyResult<()> {
        self.inner.truncate(len).map_err(value_error)
    }

    fn clear(&mut self) -> PyResult<()> {
        self.inner.clear().map_err(value_error)
    }

    fn extend(&mut self, rows: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.extend(rows_from_py(rows)?).map_err(value_error)
    }

    /// Append every row of `other`, buffer to buffer where the fields agree.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn extend_from_serie(&mut self, other: PyRef<'_, Self>) -> PyResult<()> {
        self.inner
            .extend_from_serie(&other.inner)
            .map_err(value_error)
    }

    fn resize(&mut self, len: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.resize(len, from_py(value)?).map_err(value_error)
    }

    /// Replace a record column's child of `child`'s name, or add it.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn set_child(&mut self, child: PyRef<'_, Self>) -> PyResult<()> {
        self.inner
            .set_child(child.inner.clone())
            .map_err(value_error)
    }

    /// Write one cell of row `index`, `path` deep, in place.
    fn set_cell(&mut self, path: &str, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let path = FieldPath::from_str(path).map_err(value_error)?;
        self.inner
            .set_cell(&path, index, from_py(value)?)
            .map_err(value_error)
    }

    /// This column under `field`, cast once; a `DataType` is the required
    /// column named `value` it declares. A run has no layout to cast.
    #[pyo3(signature = (field, *, safe = true, nullability = "default", representation = "value"))]
    fn cast(
        &self,
        py: Python<'_>,
        field: &Bound<'_, PyAny>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, nullability, representation)?;
        let target = target_of(field)?;
        described(py, self.inner.cast(&target, options).map_err(value_error)?)
    }

    /// This column's one row as a `pyarrow.Scalar`, sharing its buffers.
    fn into_arrow_scalar<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let scalar = self.inner.into_arrow_scalar().map_err(value_error)?;
        arrow_array_to_pyarrow(py, &scalar.into_inner(), self.inner.field())?.get_item(0)
    }

    /// The column's buffers as a `pyarrow.Array`, shared; a run has none.
    fn into_arrow_array<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let array = self.inner.require_arrow_array().map_err(value_error)?;
        arrow_array_to_pyarrow(py, &array, self.inner.field())
    }

    /// A record column as one `pyarrow.RecordBatch` of its children.
    fn into_arrow_batch<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        batch_to_pyarrow(py, self.inner.into_arrow_batch().map_err(value_error)?)
    }

    /// A record column as a `pyarrow.RecordBatchReader` of one batch.
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        batch_reader_to_pyarrow(py, self.inner.into_arrow_reader().map_err(value_error)?)
    }

    /// This column's rows as a `pyarrow.Table` of one batch.
    fn into_arrow_table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.into_arrow_reader(py)?.call_method0("read_all")
    }

    /// This column's rows as one pandas frame.
    fn into_pandas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.inner.into_arrow_reader().map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Pandas)
    }

    /// This column's rows as one polars frame.
    fn into_polars<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.inner.into_arrow_reader().map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Polars)
    }

    /// This column as one `NumPy` array.
    ///
    /// `NumPy` has no counterpart for Arrow's null mask or its nested
    /// layouts, so the crossing is allowed to copy and `PyArrow`'s own
    /// conversion decides what each value becomes - a null becomes `nan`,
    /// and a record row a mapping in an object array.
    fn into_numpy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.into_arrow_array(py)?.call_method(
            "to_numpy",
            (),
            Some(&[("zero_copy_only", false)].into_py_dict(py)?),
        )
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Row `key`, negative from the end, or the window a step-1 slice names.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(slice) = key.cast::<PySlice>() {
            let window = slice.indices(isize::try_from(self.inner.len()).unwrap_or(isize::MAX))?;
            if window.step != 1 {
                return Err(PyValueError::new_err(
                    "a Serie slices with a step of 1 only",
                ));
            }
            let start = usize::try_from(window.start).unwrap_or(0);
            let length = window.slicelength;
            return self.slice(py, start, length);
        }
        if key.is_instance_of::<pyo3::types::PyBool>() {
            return Err(PyTypeError::new_err("Serie indexes must be int"));
        }
        let index = key
            .extract::<isize>()
            .map_err(|_| PyTypeError::new_err("Serie indexes must be int or slice"))?;
        self.scalar(self.index(index)?)?.into_py_any(py)
    }

    fn __setitem__(&mut self, index: isize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let index = self.index(index)?;
        self.set(index, value)
    }

    fn __delitem__(&mut self, index: isize) -> PyResult<()> {
        let index = self.index(index)?;
        self.remove(index).map(|_| ())
    }

    fn __iter__(&self) -> PyScalarIterator {
        PyScalarIterator::new(self.inner.iter().map(Cow::into_owned))
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let value = from_py(value)?;
        Ok(self.inner.iter().any(|row| *row == value))
    }

    /// The call that rebuilds this serie, its rows spelled as Python values.
    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let serie = slf.borrow();
        let rows = serie.as_py(slf.py())?.bind(slf.py()).repr()?.to_string();
        Ok(match serie.field() {
            Some(field) => {
                let field = Bound::new(slf.py(), field)?.repr()?.to_string();
                format!("Serie.from_scalars({field}, {rows})")
            }
            None => format!("Serie({rows})"),
        })
    }

    /// Equality and order over the rows alone, as the core defines them.
    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        compare(self.inner.cmp(&other.inner), operation).into_py_any(other.py())
    }

    // A serie is mutable, so it cannot promise a stable hash.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Rebuild through `_from_pickle` from the rows and the field.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, PickleArguments)> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (self.rows(), self.field()),
        ))
    }

    /// A copy sharing the buffers, of the same class.
    fn __copy__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(py, self.inner.clone())
    }

    fn __deepcopy__(&self, py: Python<'_>, _memo: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        described(py, self.inner.clone())
    }
}

/// One record [`Serie`] per batch of an Arrow stream, each cast by one plan.
///
/// The core [`SerieReader`] compiles its plan before a batch is pulled, so
/// a planning failure is raised by the constructor. The reader is `Send` but
/// not `Sync`, and Python may pull from any thread, so it sits behind a lock;
/// `into_arrow_reader` takes it, after which nothing is left to pull.
#[pyclass(name = "SerieReader", module = "yggdryl._native")]
pub(crate) struct PySerieReader {
    reader: Mutex<Option<SerieReader>>,
    field: CoreField,
}

impl PySerieReader {
    fn from_inner(reader: SerieReader) -> Self {
        Self {
            field: reader.field().clone(),
            reader: Mutex::new(Some(reader)),
        }
    }

    /// The reader, or `None` once `into_arrow_reader` took it.
    fn held(&mut self) -> &mut Option<SerieReader> {
        self.reader
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Take the reader, which is what handing a stream over means.
    fn take(&mut self) -> PyResult<SerieReader> {
        self.held().take().ok_or_else(|| {
            PyValueError::new_err("SerieReader was already handed over by into_arrow_reader")
        })
    }
}

impl From<SerieReader> for PySerieReader {
    fn from(reader: SerieReader) -> Self {
        Self::from_inner(reader)
    }
}

#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
#[pymethods]
impl PySerieReader {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Read `reader`'s batches as record columns: of its own schema, under
    /// the root `row`, or cast into `root` by one plan compiled here.
    #[staticmethod]
    #[pyo3(signature = (reader, root = None, *, safe = true, nullability = "default", representation = "value"))]
    fn from_arrow_reader(
        reader: &Bound<'_, PyAny>,
        root: Option<&Bound<'_, PyAny>>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, nullability, representation)?;
        let root = field_of(root)?;
        let reader = SerieReader::from_arrow_reader(root.as_ref(), stream_of(reader)?, options)
            .map_err(value_error)?;
        Ok(Self::from_inner(reader))
    }

    /// Read any columnar object, or any rows, as a stream of record
    /// columns: of its own schema, or cast into `root` by one plan.
    ///
    /// A stream - a reader, a table, a frame, a dataset, an iterator or
    /// generic iterable - stays a stream. A held column - a `Serie`, an array,
    /// a batch - is the one item of its stream. Sequences of mappings or batch
    /// sources remain record streams. Other concrete containers and scalar
    /// values cross as `Serie.from_` reads them, then become one held item.
    #[staticmethod]
    #[pyo3(name = "from_")]
    #[pyo3(signature = (value, root = None, *, safe = true, nullability = "default", representation = "value"))]
    fn from_(
        value: &Bound<'_, PyAny>,
        root: Option<&Bound<'_, PyAny>>,
        safe: bool,
        nullability: &str,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, nullability, representation)?;
        serie_reader_from_py(value, root, options).map(Self::from_inner)
    }

    /// Read one held column as a stream of one record column: a record
    /// column as the batch it is, any other as the one child of a `row`.
    #[staticmethod]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn from_serie(serie: PyRef<'_, PySerie>) -> PyResult<Self> {
        SerieReader::from_serie(serie.inner.clone())
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// The record every yielded column is typed by.
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.field.clone())
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// The next batch as one record column, or the end of the stream.
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let reader = self.held();
        let next = py.detach(|| reader.as_mut().and_then(Iterator::next));
        match next {
            Some(Ok(serie)) => described(py, serie).map(Some),
            Some(Err(error)) => Err(value_error(error)),
            None => Ok(None),
        }
    }

    /// The batches not yet pulled, cast to the root, as a
    /// `pyarrow.RecordBatchReader`; this reader is spent afterwards.
    fn into_arrow_reader<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.take()?;
        batch_reader_to_pyarrow(py, reader.into_arrow_reader())
    }

    fn __repr__(&self) -> String {
        format!("SerieReader(field={})", self.field)
    }
}

/// Declare one nested leaf class below [`PySerie`].
macro_rules! nested {
    ($ident:ident, $name:literal, $doc:expr) => {
        #[doc = $doc]
        #[pyclass(name = $name, module = "yggdryl._native", extends = PySerie, skip_from_py_object)]
        pub(crate) struct $ident;
    };
}

nested!(
    PySerieSerie,
    "SerieSerie",
    "A serie column: `int32` offsets over one item column."
);
nested!(
    PyLargeSerieSerie,
    "LargeSerieSerie",
    "A large serie column: `int64` offsets over one item column."
);
nested!(
    PySerieViewSerie,
    "SerieViewSerie",
    "A serie-view column: `int32` offsets and sizes over one item column."
);
nested!(
    PyLargeSerieViewSerie,
    "LargeSerieViewSerie",
    "A large serie-view column: `int64` offsets and sizes over one item column."
);
nested!(
    PyFixedSizeSerieSerie,
    "FixedSizeSerieSerie",
    "A fixed-size serie column: `width` items per row."
);
nested!(
    PyMapSerie,
    "MapSerie",
    "A map column: `int32` offsets over one record column of entries."
);
nested!(
    PyStructSerie,
    "StructSerie",
    "A record column: one child column per child field."
);

/// The row range `range` answers, as the Python pair it is.
fn pair(range: Option<std::ops::Range<usize>>) -> Option<(usize, usize)> {
    range.map(|range| (range.start, range.end))
}

/// Emit the verbs every offsets-cut serie leaf lends, over its core leaf.
macro_rules! offset_serie {
    ($class:ident, $narrow:ident) => {
        #[pymethods]
        impl $class {
            /// The offsets cut, one more than the rows: row `i` is items
            /// `offsets[i]..offsets[i + 1]`.
            #[getter]
            fn offsets(slf: PyRef<'_, Self>) -> Vec<i64> {
                let leaf = slf.as_super().inner.$narrow().expect(LEAF);
                leaf.offsets()
                    .iter()
                    .map(|offset| i64::from(*offset))
                    .collect()
            }

            /// The items row `index` holds, or `None` past the end.
            fn range(slf: PyRef<'_, Self>, index: usize) -> Option<(usize, usize)> {
                pair(slf.as_super().inner.$narrow().expect(LEAF).range(index))
            }

            /// The item column cut to row `index`, sharing its buffers, or
            /// `None` past the end.
            fn row(
                slf: PyRef<'_, Self>,
                py: Python<'_>,
                index: usize,
            ) -> PyResult<Option<Py<PyAny>>> {
                let row = slf.as_super().inner.$narrow().expect(LEAF).row(index);
                row.map(|row| described(py, row)).transpose()
            }
        }
    };
}

offset_serie!(PySerieSerie, as_serie);
offset_serie!(PyLargeSerieSerie, as_large_serie);

/// Emit the verbs every serie-view leaf lends, over its core leaf.
macro_rules! view_serie {
    ($class:ident, $narrow:ident) => {
        #[pymethods]
        impl $class {
            /// Where each row's items start.
            #[getter]
            fn offsets(slf: PyRef<'_, Self>) -> Vec<i64> {
                let leaf = slf.as_super().inner.$narrow().expect(LEAF);
                leaf.offsets()
                    .iter()
                    .map(|offset| i64::from(*offset))
                    .collect()
            }

            /// How many items each row holds.
            #[getter]
            fn sizes(slf: PyRef<'_, Self>) -> Vec<i64> {
                let leaf = slf.as_super().inner.$narrow().expect(LEAF);
                leaf.sizes().iter().map(|size| i64::from(*size)).collect()
            }

            /// The items row `index` holds, or `None` past the end.
            fn range(slf: PyRef<'_, Self>, index: usize) -> Option<(usize, usize)> {
                pair(slf.as_super().inner.$narrow().expect(LEAF).range(index))
            }

            /// The item column cut to row `index`, sharing its buffers, or
            /// `None` past the end.
            fn row(
                slf: PyRef<'_, Self>,
                py: Python<'_>,
                index: usize,
            ) -> PyResult<Option<Py<PyAny>>> {
                let row = slf.as_super().inner.$narrow().expect(LEAF).row(index);
                row.map(|row| described(py, row)).transpose()
            }
        }
    };
}

view_serie!(PySerieViewSerie, as_serie_view);
view_serie!(PyLargeSerieViewSerie, as_large_serie_view);

#[pymethods]
impl PyFixedSizeSerieSerie {
    /// How many items every row holds.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn width(slf: PyRef<'_, Self>) -> usize {
        slf.as_super()
            .inner
            .as_fixed_size_serie()
            .expect(LEAF)
            .width()
    }

    /// The items row `index` holds, or `None` past the end.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn range(slf: PyRef<'_, Self>, index: usize) -> Option<(usize, usize)> {
        pair(
            slf.as_super()
                .inner
                .as_fixed_size_serie()
                .expect(LEAF)
                .range(index),
        )
    }

    /// The item column cut to row `index`, sharing its buffers, or `None`
    /// past the end.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn row(slf: PyRef<'_, Self>, py: Python<'_>, index: usize) -> PyResult<Option<Py<PyAny>>> {
        let row = slf
            .as_super()
            .inner
            .as_fixed_size_serie()
            .expect(LEAF)
            .row(index);
        row.map(|row| described(py, row)).transpose()
    }
}

#[pymethods]
impl PyMapSerie {
    /// The entries: one record column of the entries field, a key and a
    /// value per entry.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn entries(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(
            py,
            slf.as_super().inner.as_map().expect(LEAF).entries().clone(),
        )
    }

    /// The entries' key column.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn keys(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(
            py,
            slf.as_super().inner.as_map().expect(LEAF).keys().clone(),
        )
    }

    /// The entries' value column.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn values(slf: PyRef<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(
            py,
            slf.as_super().inner.as_map().expect(LEAF).values().clone(),
        )
    }

    /// The offsets cut: row `i` is entries `offsets[i]..offsets[i + 1]`.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn offsets(slf: PyRef<'_, Self>) -> Vec<i64> {
        let leaf = slf.as_super().inner.as_map().expect(LEAF);
        leaf.offsets()
            .iter()
            .map(|offset| i64::from(*offset))
            .collect()
    }

    /// Whether the field declares every row's keys sorted.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn keys_sorted(slf: PyRef<'_, Self>) -> bool {
        slf.as_super().inner.as_map().expect(LEAF).keys_sorted()
    }

    /// The entries row `index` holds, or `None` past the end.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn range(slf: PyRef<'_, Self>, index: usize) -> Option<(usize, usize)> {
        pair(slf.as_super().inner.as_map().expect(LEAF).range(index))
    }

    /// The entries column cut to row `index`, sharing its buffers, or
    /// `None` past the end.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn row(slf: PyRef<'_, Self>, py: Python<'_>, index: usize) -> PyResult<Option<Py<PyAny>>> {
        let row = slf.as_super().inner.as_map().expect(LEAF).row(index);
        row.map(|row| described(py, row)).transpose()
    }
}

#[pymethods]
impl PyStructSerie {
    /// The child fields' names, in the record's order.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    #[getter]
    fn names(slf: PyRef<'_, Self>) -> Vec<String> {
        slf.as_super()
            .inner
            .children()
            .iter()
            .filter_map(|child| child.field().map(|field| field.name().to_owned()))
            .collect()
    }

    /// This record without the child named `name`, sharing every other.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn without_child(slf: PyRef<'_, Self>, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let leaf = slf.as_super().inner.as_struct().expect(LEAF);
        let without = leaf.without_child(name).map_err(value_error)?;
        described(py, yggdryl::SerieValue::into_serie(without))
    }
}
