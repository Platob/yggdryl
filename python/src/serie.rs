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
//! in hand, as chunks in hand or as a stream: a held column is a [`PySerie`],
//! held chunks a [`PyChunkedSerie`], a stream a [`PyStreamChunkedSerie`], and nothing
//! else. A frame is converted by its own library, and the declared `Field` is
//! applied by the core's one cast.
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

use arrow_schema::ffi::FFI_ArrowSchema;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{
    IntoPyDict, PyBytes, PyCapsule, PyFrozenSet, PyIterator, PyList, PyMapping, PySequence, PySet,
    PySlice, PyString,
};
use yggdryl::arrow::BatchReader;
use yggdryl::expression::{IntoOrderings, Ordering as CoreOrdering};
use yggdryl::media::RecordOptions;
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, Field as CoreField, FieldPath, MimeType, Scalar, Serie,
    SortOptions, StreamChunkedSerie,
};

use crate::chunked_serie::{PyChunkedSerie, chunked_from_arrays};
use crate::datatype::{
    ArrayIntake, BatchIntake, PyDataType, array_capsule, arrow_array_to_pyarrow, pyarrow,
    schema_capsule,
};
use crate::expression::PySelector;
use crate::field::{PyField, core_field_from_value};
use crate::graph::ellipsis;
use crate::iomedia::{
    Frames, batch_reader_from_any, batch_reader_from_record_sequence, batch_reader_from_value,
    columnar_reader, core_root_field_from_value, declared_by, frame_from_reader,
    record_batch_intake, rooted_batch_to_pyarrow, rooted_reader_to_pyarrow, type_name,
};
use crate::join::{JoinKeywords, PyJoinOptions, join_keys_of, join_kind_of};
use crate::key_serie::{PyKeySerie, PyKeySeries, PyStreamKeySerie, key_by};
use crate::scalar::{
    PyScalar, PyScalarIterator, as_py, as_py_with_field, from_py, from_py_under,
    pyarrow_scalar_as_array,
};
use crate::spill::{PySpillOptions, spill_options_of};
use crate::stream_chunked_serie::PyStreamChunkedSerie;
use crate::stream_serie::PyStreamSerie;
use crate::window_serie::PyWindowSerie;
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
    fn index(&self, py: Python<'_>, index: isize) -> PyResult<usize> {
        normalize_index(index, py.detach(|| self.inner.len()))
            .ok_or_else(|| PyIndexError::new_err(index))
    }

    /// Answer `read` over this serie off the GIL, over a clone taken and
    /// released before it starts, so another thread writing this serie waits
    /// for the GIL rather than finding it borrowed.
    fn detached<T, F>(slf: &Bound<'_, Self>, read: F) -> PyResult<T>
    where
        T: Send,
        F: FnOnce(Serie) -> yggdryl::Result<T> + Send,
    {
        let serie = slf.try_borrow()?.inner.clone();
        slf.py().detach(move || read(serie)).map_err(value_error)
    }

    /// Held edits finish under the GIL so another reader can take its
    /// snapshot. A lazy edit detaches because landing may call Python.
    fn mutated<T, F>(&mut self, py: Python<'_>, write: F) -> yggdryl::Result<T>
    where
        T: Send,
        F: FnOnce(&mut Serie) -> yggdryl::Result<T> + Send,
    {
        if self.inner.is_held() {
            write(&mut self.inner)
        } else {
            py.detach(|| write(&mut self.inner))
        }
    }
}

impl Drop for PySerie {
    fn drop(&mut self) {
        if !self.inner.is_held() {
            let inner = std::mem::replace(&mut self.inner, Serie::new(Vec::new()));
            Python::attach(|py| py.detach(move || drop(inner)));
        }
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

/// Convert Python rows, each a value of `field` when the rows have one.
pub(crate) fn rows_from_py(
    field: Option<&CoreField>,
    rows: &Bound<'_, PyAny>,
) -> PyResult<Vec<Scalar>> {
    if let Ok(serie) = rows.extract::<PyRef<'_, PySerie>>() {
        return Ok(serie.inner.rows().into_owned());
    }
    rows.try_iter()?
        .map(|row| match field {
            Some(field) => from_py_under(field, &row?),
            None => from_py(&row?),
        })
        .collect::<PyResult<Vec<Scalar>>>()
}

/// The two facts an ordering states beside its key, as Python spells them:
/// two keyword booleans, ascending with nulls last unless stated.
pub(crate) const fn sort_options(descending: bool, nulls_first: bool) -> SortOptions {
    let direction = if descending {
        SortOptions::descending()
    } else {
        SortOptions::ascending()
    };
    direction.with_nulls_first(nulls_first)
}

/// Resolve the `order by` keys a sort verb takes, once: a `Selector` - every
/// projection ascending with nulls last - or any value `Scalar` holds, read
/// as the core reads one: the clause's text (`"venue, price desc nulls
/// first"`), a list of key texts, or a list of `{"term": ..., "descending":
/// ..., "nulls_first": ...}` records.
pub(crate) fn orderings_of(by: &Bound<'_, PyAny>) -> PyResult<Vec<CoreOrdering>> {
    if let Ok(selector) = by.extract::<PyRef<'_, PySelector>>() {
        return selector.inner.clone().into_orderings().map_err(value_error);
    }
    from_py(by)?.into_orderings().map_err(value_error)
}

/// The keys a root declares its rows keep, each as `SORT:by` spells it.
pub(crate) fn declared_texts(
    keys: yggdryl::Result<Option<Vec<CoreOrdering>>>,
) -> PyResult<Option<Vec<String>>> {
    Ok(keys
        .map_err(value_error)?
        .map(|keys| keys.iter().map(ToString::to_string).collect()))
}

/// Read the serie a verb takes beside its own - indices, a mask, keys - once.
///
/// A columnar object is the column [`columnar`] reads it as: a `Serie`
/// shared, chunks joined, a stream drained. Any other iterable is the run
/// of its values, each read through `Scalar` as `Serie(values)` reads it.
/// Text, bytes and a mapping iterate as something other than values, so
/// they are refused by name rather than read as characters or keys.
pub(crate) fn serie_argument(value: &Bound<'_, PyAny>, name: &str) -> PyResult<Serie> {
    if let Some(columnar) = columnar(value)? {
        return columnar.into_serie(value.py(), None, ArrowCastOptions::new());
    }
    if value.is_instance_of::<PyString>()
        || value.is_instance_of::<PyBytes>()
        || value.cast::<PyMapping>().is_ok()
        || !value.hasattr(intern!(value.py(), "__iter__"))?
    {
        return Err(PyTypeError::new_err(format!(
            "expected {name} as a Serie, a columnar object or an iterable of values, got {}",
            type_name(value)
        )));
    }
    rows_from_py(None, value).map(Serie::new)
}

/// Resolve an optional Python field argument once.
pub(crate) fn field_of(field: Option<&Bound<'_, PyAny>>) -> PyResult<Option<CoreField>> {
    field.map(core_field_from_value).transpose()
}

/// Resolve a cast target once: a field as it is spelled, or a bare
/// `DataType` as the required column named `value` it declares.
pub(crate) fn target_of(target: &Bound<'_, PyAny>) -> PyResult<CoreField> {
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
    /// Columns in hand under one field, held apart, their buffers shared.
    Chunked(ChunkedSerie),
    /// A native reader, taken: record columns already landed under its
    /// root, never re-landed.
    Reader(Box<StreamChunkedSerie>),
    /// A batch stream, not yet pulled.
    Stream(BatchReader),
}

impl Columnar {
    /// The name a declared root takes when its spelling carries none: the
    /// held field's own, a native reader's root, and `row` for a stream or
    /// a run.
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Held(serie) | Self::Pinned(serie) => {
                serie.field().map_or(DEFAULT_ROOT, CoreField::name)
            }
            Self::Chunked(chunked) => chunked.field().name(),
            Self::Reader(reader) => reader.field().name(),
            Self::Stream(_) => DEFAULT_ROOT,
        }
    }

    /// The column this object holds, draining a stream and joining chunks
    /// under `root` - the whole drain, join and cast off the GIL.
    fn into_serie(
        self,
        py: Python<'_>,
        root: Option<&CoreField>,
        options: ArrowCastOptions,
    ) -> PyResult<Serie> {
        if root.is_none()
            && let Self::Held(serie) | Self::Pinned(serie) = self
        {
            return Ok(serie);
        }
        py.detach(move || self.joined(root, options))
            .map_err(value_error)
    }

    /// [`Self::into_serie`]'s core work, which reads no Python object.
    fn joined(
        self,
        root: Option<&CoreField>,
        options: ArrowCastOptions,
    ) -> yggdryl::arrow::Result<Serie> {
        match self {
            Self::Held(serie) | Self::Pinned(serie) => match root {
                Some(root) => serie.cast(root, options),
                None => Ok(serie),
            },
            Self::Chunked(chunked) => Self::Held(chunked.into_serie()?).joined(root, options),
            Self::Reader(reader) => {
                Self::Chunked(ChunkedSerie::from_chunked_stream(*reader)?).joined(root, options)
            }
            Self::Stream(reader) => Serie::from_arrow_reader(root, reader, options),
        }
    }

    /// The stream this object is: a held column as its one batch, held
    /// chunks as one batch each, and a stream as it stands, none pulled.
    ///
    /// A root casts what is already held once, by one plan, and a native
    /// reader's records by one plan after its own; nothing held is exported
    /// and landed again.
    fn into_reader(
        self,
        py: Python<'_>,
        root: Option<&CoreField>,
        options: ArrowCastOptions,
    ) -> PyResult<StreamChunkedSerie> {
        py.detach(move || {
            let held = match self {
                Self::Held(serie) | Self::Pinned(serie) => StreamChunkedSerie::from_serie(serie),
                Self::Chunked(chunked) => StreamChunkedSerie::from_chunked(chunked),
                Self::Reader(reader) => Ok(*reader),
                Self::Stream(reader) => {
                    return StreamChunkedSerie::from_arrow_reader(root, reader, options);
                }
            }?;
            match root {
                Some(root) => held.cast(root, options),
                None => Ok(held),
            }
        })
        .map_err(value_error)
    }

    /// The chunks this object is: a held column as its one chunk, held
    /// chunks as they stand, a native reader drained one chunk per record
    /// column it yields, and a stream one chunk per batch, each under
    /// `root` - off the GIL.
    pub(crate) fn into_chunked(
        self,
        py: Python<'_>,
        root: Option<&CoreField>,
        options: ArrowCastOptions,
    ) -> PyResult<ChunkedSerie> {
        if root.is_none()
            && let Self::Chunked(chunked) = self
        {
            return Ok(chunked);
        }
        py.detach(move || self.chunked(root, options))
            .map_err(value_error)
    }

    /// [`Self::into_chunked`]'s core work, which reads no Python object.
    fn chunked(
        self,
        root: Option<&CoreField>,
        options: ArrowCastOptions,
    ) -> yggdryl::arrow::Result<ChunkedSerie> {
        match self {
            Self::Held(serie) | Self::Pinned(serie) => {
                ChunkedSerie::from_series(root, [serie], options)
            }
            Self::Chunked(chunked) => match root {
                Some(root) => chunked.cast(root, options),
                None => Ok(chunked),
            },
            Self::Reader(reader) => {
                ChunkedSerie::from_chunked_stream(*reader).and_then(|chunked| match root {
                    Some(root) => chunked.cast(root, options),
                    None => Ok(chunked),
                })
            }
            Self::Stream(reader) => ChunkedSerie::from_arrow_reader(root, reader, options),
        }
    }
}

/// Whether `value` is a native `Serie`, `ChunkedSerie` or `StreamChunkedSerie`.
///
/// Each exports the Arrow `PyCapsule` Interface for foreign consumers, which
/// no door inside the binding reads them through.
pub(crate) fn is_native_columnar(value: &Bound<'_, PyAny>) -> bool {
    value.is_instance_of::<PySerie>()
        || value.is_instance_of::<PyChunkedSerie>()
        || value.is_instance_of::<PyStreamChunkedSerie>()
        || value.is_instance_of::<PyStreamSerie>()
        || value.is_instance_of::<PyKeySerie>()
        || value.is_instance_of::<PyKeySeries>()
        || value.is_instance_of::<PyStreamKeySerie>()
}

/// Read a columnar Python object, or answer `None` for a value that is not
/// one.
///
/// The order is deterministic and each step is one library's own conversion:
///
/// 1. a native `Serie` or `ChunkedSerie`, shared, or a native
///    `StreamChunkedSerie`, taken;
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
/// `ValueError` for a `StreamChunkedSerie` already handed over.
pub(crate) fn columnar(value: &Bound<'_, PyAny>) -> PyResult<Option<Columnar>> {
    if let Ok(serie) = value.extract::<PyRef<'_, PySerie>>() {
        return Ok(Some(Columnar::Held(serie.inner.clone())));
    }
    if let Ok(chunked) = value.extract::<PyRef<'_, PyChunkedSerie>>() {
        return Ok(Some(Columnar::Chunked(chunked.inner.clone())));
    }
    if let Ok(reader) = value.extract::<PyRef<'_, PyStreamChunkedSerie>>() {
        return Ok(Some(Columnar::Reader(Box::new(reader.take(value.py())?))));
    }
    if let Ok(rows) = value.extract::<PyRef<'_, PyStreamSerie>>() {
        return Ok(Some(Columnar::Held(Serie::from(rows.take(value.py())?))));
    }
    if let Ok(key) = value.extract::<PyRef<'_, PyKeySerie>>() {
        return Ok(Some(Columnar::Held(Serie::from(key.core().clone()))));
    }
    if let Ok(keys) = value.extract::<PyRef<'_, PyKeySeries>>() {
        return Ok(Some(Columnar::Held(Serie::from(keys.core().clone()))));
    }
    if let Ok(keys) = value.extract::<PyRef<'_, PyStreamKeySerie>>() {
        return Ok(Some(Columnar::Held(Serie::from(keys.take(value.py())?))));
    }
    // A held container is recognized by its exact class before the stream
    // ladder, because a `RecordBatch` also exports a stream and reading it as
    // one would lose the length it already knows. `PyArrow`'s classes are
    // none of the pandas, polars or `NumPy` types below, so asking first
    // changes no answer and saves the common input their probes.
    if let Some(value) = pyarrow_value(value)? {
        return Ok(Some(value));
    }
    if let Some(series) = series_to_arrow(value)? {
        return array_of(&series).map(Some);
    }
    if declared_by(value, "numpy", "ndarray") {
        return numpy_value(value).map(Some);
    }
    if let Some(reader) = columnar_reader(value)? {
        return Ok(Some(Columnar::Stream(reader)));
    }
    if value.hasattr(intern!(value.py(), "__arrow_c_array__"))? {
        return array_of(value).map(Some);
    }
    Ok(None)
}

/// Read a columnar object as the one column it holds, a stream drained.
///
/// This is what `Scalar.from_` holds a columnar argument as: a serie sharing
/// the column's buffers, its rows unread, chunks joined into one. One Arrow
/// scalar is its row.
///
/// # Errors
///
/// [`columnar`]'s, and a stream's own.
pub(crate) fn columnar_value(value: &Bound<'_, PyAny>) -> PyResult<Option<Scalar>> {
    Ok(match columnar(value)? {
        None => None,
        Some(Columnar::Pinned(serie)) => Some(serie.scalar(0).map_err(value_error)?),
        Some(columnar) => Some(Scalar::from(columnar.into_serie(
            value.py(),
            None,
            ArrowCastOptions::new(),
        )?)),
    })
}

/// Read a Python value already proven not to be columnar as one column.
///
/// The value crosses through the native [`Scalar`] boundary exactly once: a
/// sequence is its rows and anything else one row, under `field` or the field
/// the value infers.
pub(crate) fn serie_from_value(
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
    value
        .py()
        .detach(move || match scalar.as_serie() {
            // A column already names its layout, so a declared field casts it.
            Some(rows) if rows.is_column() => match declared {
                Some(field) => rows.cast(&field, options),
                None => Ok(rows.clone()),
            },
            Some(rows) => {
                let field = match declared {
                    Some(field) => field,
                    None => scalar.inferred_array_field()?,
                };
                Ok(Serie::from_scalars(field, rows.rows().into_owned())?)
            }
            None => {
                let field = match declared {
                    Some(field) => field,
                    None => scalar.inferred_scalar_field()?,
                };
                Ok(Serie::from_scalars(field, [scalar])?)
            }
        })
        .map_err(value_error)
}

/// Read any Python value as one column, cast into `field` when one is given.
///
/// A columnar object is [`columnar`]'s; any other value is
/// [`serie_from_value`]'s.
pub(crate) fn serie_from_py(
    value: &Bound<'_, PyAny>,
    field: Option<&Bound<'_, PyAny>>,
    options: ArrowCastOptions,
) -> PyResult<Serie> {
    if let Some(columnar) = columnar(value)? {
        let root = field
            .map(|field| core_root_field_from_value(field, columnar.name()))
            .transpose()?;
        return columnar.into_serie(value.py(), root.as_ref(), options);
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
pub(crate) fn chunked_stream_from_py(
    value: &Bound<'_, PyAny>,
    root: Option<&Bound<'_, PyAny>>,
    options: ArrowCastOptions,
) -> PyResult<StreamChunkedSerie> {
    let root = root
        .map(|root| core_root_field_from_value(root, DEFAULT_ROOT))
        .transpose()?;
    if let Some(columnar) = columnar(value)? {
        return columnar.into_reader(value.py(), root.as_ref(), options);
    }
    if value.cast::<PySequence>().is_ok()
        && let Some(reader) = batch_reader_from_record_sequence(
            value,
            &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM).map_err(value_error)?,
        )?
    {
        return StreamChunkedSerie::from_arrow_reader(root.as_ref(), reader, options)
            .map_err(value_error);
    }
    let streams = value.cast::<PyIterator>().is_ok()
        || (value.cast::<PySequence>().is_err()
            && value.cast::<PyMapping>().is_err()
            && value.cast::<PySet>().is_err()
            && value.cast::<PyFrozenSet>().is_err()
            && value.extract::<PyRef<'_, PyScalar>>().is_err()
            && value.hasattr("__iter__")?);
    if streams {
        return StreamChunkedSerie::from_arrow_reader(root.as_ref(), stream_of(value)?, options)
            .map_err(value_error);
    }
    let serie = serie_from_value(value, None, ArrowCastOptions::new())?;
    Columnar::Held(serie).into_reader(value.py(), root.as_ref(), options)
}

/// The name a record root takes when its spelling carries none, because
/// Arrow names columns and never the record.
const DEFAULT_ROOT: &str = "row";

/// The record root a column crosses into a batch or a stream under - the
/// one rule the core names, [`StreamChunkedSerie::root_of`].
fn batch_root(serie: &Serie) -> PyResult<CoreField> {
    StreamChunkedSerie::root_of(serie.require_field().map_err(value_error)?).map_err(value_error)
}

/// The C schema a column crosses the Arrow `PyCapsule` Interface under: a
/// record column's is the exchange projection of the root it is a batch of,
/// which keeps the root's metadata and its dictionary identities; any other
/// column's is its own field's.
fn capsule_schema(serie: &Serie) -> PyResult<FFI_ArrowSchema> {
    let field = serie.require_field().map_err(value_error)?;
    if field.dtype().as_fields().is_some() {
        batch_root(serie)?
            .into_arrow_exchange_ffi()
            .map_err(value_error)
    } else {
        field.clone().into_arrow_field_ffi().map_err(value_error)
    }
}

/// Resolve a `requested_schema` capsule a consumer handed a column to the
/// field it asks for, named as the column is: the capsule is imported by
/// `pyarrow`, never read here, and a type carries no name worth taking.
pub(crate) fn requested_field(requested: &Bound<'_, PyAny>, name: &str) -> PyResult<CoreField> {
    let py = requested.py();
    let field =
        pyarrow::field(py)?.call_method1(intern!(py, "_import_from_c_capsule"), (requested,))?;
    Ok(core_field_from_value(&field)?.with_name(name))
}

/// Hand `reader` over as the Arrow `PyCapsule` Interface's stream capsule,
/// cast into `requested_schema` - a record schema - by the one cast first
/// when a consumer asks for one.
///
/// The stream crosses to `pyarrow` under its root's exact schema, and the
/// capsule is the one that reader exports, so a foreign consumer reads every
/// nested flag the root states.
pub(crate) fn reader_capsule<'py>(
    py: Python<'py>,
    reader: StreamChunkedSerie,
    requested_schema: Option<&Bound<'py, PyAny>>,
) -> PyResult<Bound<'py, PyAny>> {
    let reader = match requested_schema {
        Some(requested) => {
            let schema = pyarrow::schema(py)?
                .call_method1(intern!(py, "_import_from_c_capsule"), (requested,))?;
            let root = core_root_field_from_value(&schema, reader.field().name())?;
            py.detach(move || reader.cast(&root, ArrowCastOptions::new()))
                .map_err(value_error)?
        }
        None => reader,
    };
    let root = reader.field().clone();
    rooted_reader_to_pyarrow(py, &root, reader.into_arrow_reader())?
        .call_method0(intern!(py, "__arrow_c_stream__"))
}

/// Convert one pandas or polars series to a `PyArrow` array, if it is one.
fn series_to_arrow<'py>(value: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    if declared_by(value, "polars", "Series") {
        return value.call_method0("to_arrow").map(Some);
    }
    if declared_by(value, "pandas", "Series") {
        // pandas does not export Arrow itself, so `PyArrow` - a dependency -
        // converts it, which behaves the same on every pandas release.
        return pyarrow::array(value.py())?
            .call_method1(intern!(value.py(), "from_pandas"), (value,))
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
    let batch = pyarrow::record_batch(py)?.call_method(
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
    let py = value.py();
    if value.is_instance(pyarrow::record_batch(py)?)? {
        return batch_of(value).map(Some);
    }
    // A table may hold many chunks, and the C stream hands them over without
    // combining them, so it crosses as a stream rather than as a copy.
    if value.is_instance(pyarrow::table(py)?)?
        || value.is_instance(pyarrow::record_batch_reader(py)?)?
    {
        return Ok(Some(Columnar::Stream(batch_reader_from_value(value)?)));
    }
    if value.is_instance(pyarrow::chunked_array(py)?)? {
        // A chunked array is its chunks, each crossing on its own with its
        // buffers shared; a column of it is their one join.
        return chunked_from_arrays(value, None, ArrowCastOptions::new())
            .map(|chunked| Some(Columnar::Chunked(chunked)));
    }
    if value.is_instance(pyarrow::array(py)?)? {
        return array_of(value).map(Some);
    }
    if value.is_instance(pyarrow::scalar(py)?)? {
        let serie = held_array(&pyarrow_scalar_as_array(value)?)?;
        // A pinned row is a value, so its column is named as one.
        let field = serie.require_field().map_err(value_error)?.clone();
        let serie = serie
            .cast(&field.with_name("value"), ArrowCastOptions::new())
            .map_err(value_error)?;
        return Ok(Some(Columnar::Pinned(serie)));
    }
    Ok(None)
}

/// One record batch as the record column of its own schema, its columns
/// proven and landed off the GIL.
fn batch_of(value: &Bound<'_, PyAny>) -> PyResult<Columnar> {
    let intake = BatchIntake::from_value(value)?;
    value
        .py()
        .detach(move || {
            let batch = intake.validated()?;
            Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())
        })
        .map(Columnar::Held)
        .map_err(value_error)
}

/// One foreign column as the column of its own layout: the field it proves
/// about itself, named `item`, and its buffers, shared - proven and landed
/// off the GIL.
fn array_of(value: &Bound<'_, PyAny>) -> PyResult<Columnar> {
    held_array(value).map(Columnar::Held)
}

/// One foreign array as the column it is - of the extension type its type
/// states, else of its own layout - proven and landed off the GIL.
fn held_array(value: &Bound<'_, PyAny>) -> PyResult<Serie> {
    let intake = ArrayIntake::from_value(value)?;
    let stated = intake.stated_field().cloned();
    value
        .py()
        .detach(move || {
            Serie::from_arrow_array(
                stated.as_ref(),
                intake.validated()?,
                ArrowCastOptions::new(),
            )
        })
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
        let rows = values
            .map(|values| rows_from_py(None, values))
            .transpose()?
            .unwrap_or_default();
        Ok(Self::from_inner(Serie::new(rows)))
    }

    /// The column `field` types `rows` into, each through its contract once.
    #[staticmethod]
    fn from_scalars(
        py: Python<'_>,
        field: &Bound<'_, PyAny>,
        rows: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let field = core_field_from_value(field)?;
        let rows = rows_from_py(Some(&field), rows)?;
        let serie = py
            .detach(move || Serie::from_scalars(field, rows))
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
    /// any other is cast under the two options.
    #[staticmethod]
    #[pyo3(signature = (array, field = None, *, safe = true, representation = "value"))]
    fn from_arrow_array(
        py: Python<'_>,
        array: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, representation)?;
        let array = ArrayIntake::from_value(array)?;
        let field = field_of(field)?.or_else(|| array.stated_field().cloned());
        let serie = py
            .detach(move || Serie::from_arrow_array(field.as_ref(), array.validated()?, options))
            .map_err(value_error)?;
        described(py, serie)
    }

    /// Take one record batch as a record column: of its own schema, under
    /// the root `row`, or cast into `root`.
    #[staticmethod]
    #[pyo3(signature = (batch, root = None, *, safe = true, representation = "value"))]
    fn from_arrow_batch(
        py: Python<'_>,
        batch: &Bound<'_, PyAny>,
        root: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, representation)?;
        let root = field_of(root)?;
        let batch = record_batch_intake(batch)?;
        let serie = py
            .detach(move || {
                let batch = batch.validated()?;
                Serie::from_arrow_batch(root.as_ref(), &batch, options)
            })
            .map_err(value_error)?;
        described(py, serie)
    }

    /// Drain a record batch stream into one record column: of its own
    /// schema, or cast into `root` by one plan.
    #[staticmethod]
    #[pyo3(signature = (reader, root = None, *, safe = true, representation = "value"))]
    fn from_arrow_reader(
        py: Python<'_>,
        reader: &Bound<'_, PyAny>,
        root: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, representation)?;
        let root = field_of(root)?;
        let reader = stream_of(reader)?;
        // The whole drain and join run off the GIL; a Python-backed reader
        // attaches for exactly the pulls it makes.
        let serie = py
            .detach(move || Serie::from_arrow_reader(root.as_ref(), reader, options))
            .map_err(value_error)?;
        described(py, serie)
    }

    /// `length` copies of `value` under `field`: a constant column, the
    /// value proven by the field once and held as one row, the whole array
    /// laid out only when something exports it. Reading a cell answers the
    /// value, slicing moves the count, and writing another value lays the
    /// column out as its field's leaf first.
    #[staticmethod]
    fn lit(
        py: Python<'_>,
        field: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        length: usize,
    ) -> PyResult<Py<PyAny>> {
        let field = core_field_from_value(field)?;
        let value = from_py_under(&field, value)?;
        described(py, Serie::lit(field, value, length).map_err(value_error)?)
    }

    /// `rows` copies of `field`'s canonical default, as the constant column
    /// `lit` builds.
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
    /// by their own conversion, buffers shared; a stream is drained, and
    /// chunks - a `ChunkedSerie`, a chunked array - are joined once. Any
    /// other value is read as a `Scalar`: a sequence is its rows, and
    /// anything else one row.
    #[staticmethod]
    #[pyo3(name = "from_")]
    #[pyo3(signature = (value, field = None, *, safe = true, representation = "value"))]
    fn from_(
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = cast_options(safe, representation)?;
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
        let rows = rows_from_py(None, rows)?;
        let serie = match field_of(field)? {
            Some(field) => py
                .detach(move || Serie::from_scalars(field, rows))
                .map_err(value_error)?,
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

    /// Whether this is a constant column `lit` built: one value and a
    /// length, no row laid out until something exports it.
    #[getter]
    fn is_lit(&self) -> bool {
        self.inner.as_lit().is_some()
    }

    fn null_count(&self, py: Python<'_>) -> usize {
        py.detach(|| self.inner.null_count())
    }

    fn is_empty(&self, py: Python<'_>) -> bool {
        py.detach(|| self.inner.is_empty())
    }

    fn is_null(&self, py: Python<'_>, index: usize) -> PyResult<bool> {
        py.detach(|| self.inner.is_null(index).map_err(value_error))
    }

    /// Row `index`, built as one value.
    fn scalar(&self, py: Python<'_>, index: usize) -> PyResult<PyScalar> {
        py.detach(|| {
            self.inner
                .scalar(index)
                .map(PyScalar::from_inner)
                .map_err(value_error)
        })
    }

    /// Row `index`, or `None` past the end.
    fn get(&self, py: Python<'_>, index: usize) -> Option<PyScalar> {
        py.detach(|| {
            self.inner
                .get(index)
                .map(|row| PyScalar::from_inner(row.into_owned()))
        })
    }

    /// Every row, each built once and kept by the list alone.
    fn rows(&self, py: Python<'_>) -> Vec<PyScalar> {
        py.detach(|| {
            self.inner
                .rows()
                .into_owned()
                .into_iter()
                .map(PyScalar::from_inner)
                .collect()
        })
    }

    pub(crate) fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let field = self.inner.field().cloned();
        let rows = if self.inner.is_held() {
            self.inner.rows().into_owned()
        } else {
            py.detach(|| self.inner.rows().into_owned())
        };
        let list = PyList::empty(py);
        for row in rows {
            list.append(match &field {
                Some(field) => as_py_with_field(py, &row, field)?,
                None => as_py(py, &row)?,
            })?;
        }
        Ok(list.into_any().unbind())
    }

    /// This serie as the one value a `Scalar` sequence holds.
    fn into_scalar(&self, py: Python<'_>) -> PyScalar {
        py.detach(|| PyScalar::from_inner(Scalar::from(self.inner.clone())))
    }

    /// The run of this serie's rows, dropping the field.
    fn into_run(&self, py: Python<'_>) -> Self {
        py.detach(|| Self::from_inner(Serie::from(self.inner.clone().into_run())))
    }

    /// `length` rows from `offset`, sharing a column's buffers.
    fn slice(&self, py: Python<'_>, offset: usize, length: usize) -> PyResult<Py<PyAny>> {
        described(
            py,
            py.detach(|| self.inner.slice(offset, length))
                .map_err(value_error)?,
        )
    }

    /// A record column's child named `name`, or `None`.
    fn child(&self, py: Python<'_>, name: &str) -> PyResult<Option<Py<PyAny>>> {
        py.detach(|| self.inner.child(name).cloned())
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// A record column's child, or a union's member, at `index`.
    fn child_at(&self, py: Python<'_>, index: usize) -> PyResult<Option<Py<PyAny>>> {
        py.detach(|| self.inner.child_at(index).cloned())
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// Every child of a record column, or member of a union.
    fn children(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        described_all(py, py.detach(|| self.inner.children()))
    }

    /// A sequence column's items, a mapping's entries, an encoding's values.
    fn items(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        py.detach(|| self.inner.items().cloned())
            .map(|items| described(py, items.clone()))
            .transpose()
    }

    /// The column `path` reaches, spelled as a field path.
    fn get_child_by_path(&self, py: Python<'_>, path: &str) -> PyResult<Option<Py<PyAny>>> {
        let path = FieldPath::from_str(path).map_err(value_error)?;
        py.detach(|| self.inner.get_child_by_path(&path).cloned())
            .map(|child| described(py, child.clone()))
            .transpose()
    }

    /// Replace rows `start..end` by `rows`: the one mutation.
    fn splice(
        &mut self,
        py: Python<'_>,
        start: usize,
        end: usize,
        rows: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let rows = rows_from_py(self.inner.field(), rows)?;
        self.mutated(py, |inner| inner.splice(start..end, rows))
            .map_err(value_error)
    }

    fn set(&mut self, py: Python<'_>, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = match self.inner.field() {
            Some(field) => from_py_under(field, value)?,
            None => from_py(value)?,
        };
        self.mutated(py, |inner| inner.set(index, value))
            .map_err(value_error)
    }

    fn push(&mut self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = from_py(value)?;
        self.mutated(py, |inner| inner.push(value))
            .map_err(value_error)
    }

    fn insert(&mut self, py: Python<'_>, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = from_py(value)?;
        self.mutated(py, |inner| inner.insert(index, value))
            .map_err(value_error)
    }

    /// Remove row `index` and answer it.
    fn remove(&mut self, py: Python<'_>, index: usize) -> PyResult<PyScalar> {
        self.mutated(py, |inner| inner.remove(index))
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Remove the last row and answer it, or `None` when empty.
    fn pop(&mut self, py: Python<'_>) -> PyResult<Option<PyScalar>> {
        self.mutated(py, Serie::pop)
            .map(|row| row.map(PyScalar::from_inner))
            .map_err(value_error)
    }

    fn truncate(&mut self, py: Python<'_>, len: usize) -> PyResult<()> {
        self.mutated(py, |inner| inner.truncate(len))
            .map_err(value_error)
    }

    fn clear(&mut self, py: Python<'_>) -> PyResult<()> {
        self.mutated(py, Serie::clear).map_err(value_error)
    }

    fn extend(&mut self, py: Python<'_>, rows: &Bound<'_, PyAny>) -> PyResult<()> {
        let rows = rows_from_py(self.inner.field(), rows)?;
        self.mutated(py, |inner| inner.extend(rows))
            .map_err(value_error)
    }

    /// Append every row of `other`, buffer to buffer where the fields agree.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn extend_from_serie(&mut self, py: Python<'_>, other: PyRef<'_, Self>) -> PyResult<()> {
        let other = other.inner.clone();
        self.mutated(py, |inner| inner.extend_from_serie(&other))
            .map_err(value_error)
    }

    fn resize(&mut self, py: Python<'_>, len: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = from_py(value)?;
        self.mutated(py, |inner| inner.resize(len, value))
            .map_err(value_error)
    }

    /// Replace a record column's child of `child`'s name, or add it.
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn set_child(&mut self, py: Python<'_>, child: PyRef<'_, Self>) -> PyResult<()> {
        let child = child.inner.clone();
        self.mutated(py, |inner| inner.set_child(child))
            .map_err(value_error)
    }

    /// Write one cell of row `index`, `path` deep, in place.
    fn set_cell(
        &mut self,
        py: Python<'_>,
        path: &str,
        index: usize,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let path = FieldPath::from_str(path).map_err(value_error)?;
        let value = from_py(value)?;
        self.mutated(py, |inner| inner.set_cell(&path, index, value))
            .map_err(value_error)
    }

    /// This column under `field`, cast once; a `DataType` is the required
    /// column named `value` it declares. A run has no layout to cast.
    ///
    /// The cast runs off the GIL over a clone of the column, taken and
    /// released before it starts, so another thread writing this serie waits
    /// for the GIL rather than finding it borrowed.
    #[pyo3(signature = (field, *, safe = true, representation = "value"))]
    fn cast(
        slf: &Bound<'_, Self>,
        field: &Bound<'_, PyAny>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let options = cast_options(safe, representation)?;
        let target = target_of(field)?;
        let serie = slf.try_borrow()?.inner.clone();
        let cast = py
            .detach(move || serie.cast(&target, options))
            .map_err(value_error)?;
        described(py, cast)
    }

    // ------------------------------------------------------------------
    // Ordering, uniqueness and grouping: the reads answer a new serie off
    // the GIL over a clone taken and released first, as `cast` does; the
    // `as_*` writes bring this serie into the state in place, under the GIL,
    // so a buffer it holds alone is rewritten where it stands, and answer
    // this same object so calls chain.
    // ------------------------------------------------------------------

    /// The row positions in sorted order as a `uint32` column named
    /// `index`: stable, absent rows where `nulls_first` puts them.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn sort_indices(
        slf: &Bound<'_, Self>,
        descending: bool,
        nulls_first: bool,
    ) -> PyResult<Py<PyAny>> {
        let options = sort_options(descending, nulls_first);
        let order = Self::detached(slf, move |serie| serie.sort_indices(options))?;
        described(slf.py(), order)
    }

    /// Whether the rows are in sorted order: one pass over adjacent rows.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn is_sorted(slf: &Bound<'_, Self>, descending: bool, nulls_first: bool) -> PyResult<bool> {
        let options = sort_options(descending, nulls_first);
        Self::detached(slf, move |serie| Ok(serie.is_sorted(options)))
    }

    /// Whether no two rows hold one value; two absent rows are a repeat.
    fn is_unique(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Self::detached(slf, |serie| Ok(serie.is_unique()))
    }

    /// How many distinct values the rows hold, an absent row one of them.
    fn unique_count(slf: &Bound<'_, Self>) -> PyResult<usize> {
        Self::detached(slf, |serie| Ok(serie.unique_count()))
    }

    /// The rows in sorted order as a new serie under the same field.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn into_sorted(
        slf: &Bound<'_, Self>,
        descending: bool,
        nulls_first: bool,
    ) -> PyResult<Py<PyAny>> {
        let options = sort_options(descending, nulls_first);
        let sorted = Self::detached(slf, move |serie| serie.into_sorted(options))?;
        described(slf.py(), sorted)
    }

    /// The first occurrence of every value, in order of first occurrence.
    fn into_unique(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let unique = Self::detached(slf, |serie| serie.into_unique())?;
        described(slf.py(), unique)
    }

    /// The rows in reverse order as a new serie.
    fn into_reversed(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let reversed = Self::detached(slf, |serie| Ok(serie.into_reversed()))?;
        described(slf.py(), reversed)
    }

    /// The rows `indices` names, in that order: integers of any width, as a
    /// `Serie`, a columnar object or an iterable of values.
    fn into_taken(slf: &Bound<'_, Self>, indices: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let indices = serie_argument(indices, "indices")?;
        let taken = Self::detached(slf, move |serie| serie.into_taken(&indices))?;
        described(slf.py(), taken)
    }

    /// The rows `mask` keeps: booleans as long as this serie, an absent
    /// mask row keeping nothing.
    fn into_filtered(slf: &Bound<'_, Self>, mask: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let mask = serie_argument(mask, "mask")?;
        let kept = Self::detached(slf, move |serie| serie.into_filtered(&mask))?;
        described(slf.py(), kept)
    }

    /// Group native rows by one resolved key intake.
    fn partition_by(slf: &Bound<'_, Self>, by: &Bound<'_, PyAny>) -> PyResult<PyKeySeries> {
        let by = key_by(by)?;
        Self::detached(slf, move |serie| serie.partition_by(by)).map(PyKeySeries::from_inner)
    }

    /// The bytes the rows occupy: a column's buffers as its own slice
    /// counts them, a run's values as the row estimator charges them.
    fn memory_size(&self) -> usize {
        self.inner.memory_size()
    }

    /// The bytes the rows occupy in memory: `memory_size` less what lies in
    /// a spill file's mapping.
    fn resident_size(&self) -> usize {
        self.inner.resident_size()
    }

    /// Whether the rows lie in a spill file: some bytes, none of them
    /// resident. A run and an empty column are never spilled.
    fn is_spilled(&self) -> bool {
        self.inner.is_spilled()
    }

    /// Move the rows to disk until the resident bytes are under the bound,
    /// in place: `options` - the process default, `SpillOptions.from_env()`,
    /// for `None` - with `byte_size` and `folder` set on a copy where given.
    /// The heaviest leaves spill first, each buffer written once to a
    /// private file and mapped back read-only; a write brings the buffer it
    /// touches back to the heap. A run spills nothing, and a refused folder
    /// leaves the serie as it was.
    #[pyo3(signature = (options = None, *, byte_size = ellipsis(), folder = ellipsis()))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands the `...` default over as `Py`.
    fn spill(
        slf: &Bound<'_, Self>,
        options: Option<PyRef<'_, PySpillOptions>>,
        byte_size: Py<PyAny>,
        folder: Py<PyAny>,
    ) -> PyResult<()> {
        let py = slf.py();
        let options = spill_options_of(options.as_deref(), byte_size.bind(py), folder.bind(py))?;
        let mut held = slf.try_borrow_mut()?;
        let inner = &mut held.inner;
        slf.py()
            .detach(|| inner.spill(&options))
            .map_err(value_error)
    }

    /// `spill`, answering this serie so calls chain; a refused folder
    /// leaves it as it was.
    #[pyo3(signature = (options = None, *, byte_size = ellipsis(), folder = ellipsis()))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands the `...` default over as `Py`.
    fn as_spilled<'py>(
        slf: &Bound<'py, Self>,
        options: Option<PyRef<'_, PySpillOptions>>,
        byte_size: Py<PyAny>,
        folder: Py<PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let options = spill_options_of(options.as_deref(), byte_size.bind(py), folder.bind(py))?;
        let mut held = slf.try_borrow_mut()?;
        let inner = &mut held.inner;
        slf.py()
            .detach(|| inner.as_spilled(&options))
            .map_err(value_error)?;
        Ok(slf.clone())
    }

    /// A copy of this serie spilled under the bound, this one untouched:
    /// the buffers the bound leaves resident are shared, the rest written
    /// once and mapped. `options` and the keywords read as `spill` reads
    /// them.
    #[pyo3(signature = (options = None, *, byte_size = ellipsis(), folder = ellipsis()))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands the `...` default over as `Py`.
    fn into_spilled(
        slf: &Bound<'_, Self>,
        options: Option<PyRef<'_, PySpillOptions>>,
        byte_size: Py<PyAny>,
        folder: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let options = spill_options_of(options.as_deref(), byte_size.bind(py), folder.bind(py))?;
        let spilled = Self::detached(slf, move |serie| serie.into_spilled(&options))?;
        described(py, spilled)
    }

    /// The `order by` keys a record's root declares its rows keep
    /// (`SORT:by`), each as that declaration spells it, or `None`.
    fn declared_order(&self) -> PyResult<Option<Vec<String>>> {
        declared_texts(self.inner.declared_order())
    }

    /// The row positions in the order the `order by` keys of `by` state, as
    /// a `uint32` column named `index`: stable, a plain column keying as
    /// itself under its own name.
    fn sort_indices_by(slf: &Bound<'_, Self>, by: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let by = orderings_of(by)?;
        let order = Self::detached(slf, move |serie| serie.sort_indices_by(by))?;
        described(slf.py(), order)
    }

    /// The rows in the order the `order by` keys of `by` state, as a new
    /// serie whose root declares that order.
    fn into_sort_by(slf: &Bound<'_, Self>, by: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let by = orderings_of(by)?;
        let sorted = Self::detached(slf, move |serie| serie.into_sort_by(by))?;
        described(slf.py(), sorted)
    }

    /// This serie joined with `other` on `by`, under `how`: one record
    /// column of the left columns, then the right, a key stated as one bare
    /// column on both sides once under the left name when `coalesce`, a
    /// colliding right name suffixed. `other` is a `Serie` or anything
    /// `Serie.from_` reads; `options` - `JoinOptions()` for `None` - takes
    /// each keyword given on a copy.
    #[pyo3(signature = (
        other,
        by,
        how = "inner",
        options = None,
        *,
        coalesce = ellipsis(),
        suffix = ellipsis(),
        build = ellipsis(),
        prune = ellipsis(),
        spill = ellipsis(),
        pushdown_keys = ellipsis(),
    ))]
    #[allow(clippy::too_many_arguments)]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn join_with(
        slf: &Bound<'_, Self>,
        other: &Bound<'_, PyAny>,
        by: &Bound<'_, PyAny>,
        how: &str,
        options: Option<PyRef<'_, PyJoinOptions>>,
        coalesce: Py<PyAny>,
        suffix: Py<PyAny>,
        build: Py<PyAny>,
        prune: Py<PyAny>,
        spill: Py<PyAny>,
        pushdown_keys: Py<PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let keys = join_keys_of(by)?;
        let how = join_kind_of(how)?;
        let options = JoinKeywords {
            coalesce,
            suffix,
            build,
            prune,
            spill,
            pushdown_keys,
        }
        .resolve(py, options.as_deref())?;
        let other = serie_from_py(other, None, ArrowCastOptions::new())?;
        let joined = Self::detached(slf, move |serie| {
            serie.join_with(&other, keys, how, &options)
        })?;
        described(py, joined)
    }

    /// Sort the rows in place, answering this serie.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn as_sorted<'py>(
        slf: &Bound<'py, Self>,
        descending: bool,
        nulls_first: bool,
    ) -> PyResult<Bound<'py, Self>> {
        let options = sort_options(descending, nulls_first);
        let mut held = slf.try_borrow_mut()?;
        let inner = &mut held.inner;
        slf.py()
            .detach(|| inner.as_sorted(options))
            .map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Keep the first occurrence of every value, in place.
    fn as_unique<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut held = slf.try_borrow_mut()?;
        let inner = &mut held.inner;
        slf.py().detach(|| inner.as_unique()).map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Reverse the rows in place.
    fn as_reversed<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        let mut held = slf.try_borrow_mut()?;
        let inner = &mut held.inner;
        slf.py()
            .detach(|| inner.as_reversed())
            .map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Keep the rows `indices` names, in that order, in place.
    fn as_taken<'py>(
        slf: &Bound<'py, Self>,
        indices: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let indices = serie_argument(indices, "indices")?;
        let mut held = slf.try_borrow_mut()?;
        let inner = &mut held.inner;
        slf.py()
            .detach(|| inner.as_taken(&indices))
            .map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Keep the rows `mask` keeps, in place.
    fn as_filtered<'py>(
        slf: &Bound<'py, Self>,
        mask: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let mask = serie_argument(mask, "mask")?;
        let mut held = slf.try_borrow_mut()?;
        let inner = &mut held.inner;
        slf.py()
            .detach(|| inner.as_filtered(&mask))
            .map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Sort the rows in place in the order the `order by` keys of `by`
    /// state, answering this serie; a refusal leaves it as it was.
    fn as_sort_by<'py>(
        slf: &Bound<'py, Self>,
        by: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let by = orderings_of(by)?;
        let mut held = slf.try_borrow_mut()?;
        let inner = &mut held.inner;
        slf.py()
            .detach(|| inner.as_sort_by(by))
            .map_err(value_error)?;
        Ok(slf.clone())
    }

    /// The window `offset..offset + length` as a `WindowSerie` holding this
    /// serie object: every read and write goes through the serie when it is
    /// asked, window-relative.
    fn window(slf: &Bound<'_, Self>, offset: usize, length: usize) -> PyResult<PyWindowSerie> {
        slf.try_borrow()?
            .inner
            .window(offset, length)
            .map_err(value_error)?;
        Ok(PyWindowSerie::new(slf.clone().unbind(), offset, length))
    }

    /// Own the adjacent equal-key windows, sharing the native payload buffers.
    #[pyo3(signature = (by, sorted = Some(false)))]
    fn window_by(
        slf: &Bound<'_, Self>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<PyKeySeries> {
        let by = key_by(by)?;
        Self::detached(slf, move |serie| {
            serie.window_by(by, sorted.unwrap_or(false))
        })
        .map(PyKeySeries::from_inner)
    }

    fn into_stream(slf: &Bound<'_, Self>) -> PyResult<PyStreamSerie> {
        Self::detached(slf, yggdryl::Serie::into_stream).map(PyStreamSerie::from_core)
    }

    #[pyo3(signature = (row_size = None, byte_size = None))]
    fn into_chunked_stream(
        slf: &Bound<'_, Self>,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> PyResult<PyStreamChunkedSerie> {
        let serie = slf.try_borrow()?.inner.clone();
        slf.py()
            .detach(move || serie.into_chunked_stream(row_size, byte_size))
            .map(PyStreamChunkedSerie::from)
            .map_err(value_error)
    }

    /// This column's one row as a `pyarrow.Scalar`, sharing its buffers.
    fn into_arrow_scalar<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let scalar = py
            .detach(|| self.inner.into_arrow_scalar())
            .map_err(value_error)?;
        arrow_array_to_pyarrow(py, &scalar.into_inner(), self.inner.field())?.get_item(0)
    }

    /// The column's buffers as a `pyarrow.Array`, shared; a run has none.
    fn into_arrow_array<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let array = py
            .detach(|| self.inner.require_arrow_array())
            .map_err(value_error)?;
        arrow_array_to_pyarrow(py, &array, self.inner.field())
    }

    /// A record column as one `pyarrow.RecordBatch` of its children, under
    /// the root the core names it by, buffers shared.
    fn into_arrow_batch<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let batch = py
            .detach(|| self.inner.into_arrow_batch())
            .map_err(value_error)?;
        rooted_batch_to_pyarrow(py, &batch_root(&self.inner)?, batch)
    }

    /// A record column as a `pyarrow.RecordBatchReader` of one batch.
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = py
            .detach(|| self.inner.into_arrow_reader())
            .map_err(value_error)?;
        rooted_reader_to_pyarrow(py, &batch_root(&self.inner)?, reader)
    }

    /// This column's field as the Arrow `PyCapsule` Interface's
    /// `arrow_schema` capsule: the schema `__arrow_c_array__` hands over.
    fn __arrow_c_schema__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyCapsule>> {
        schema_capsule(py, capsule_schema(&self.inner)?)
    }

    /// This column as the Arrow `PyCapsule` Interface's schema and array
    /// capsules, sharing its buffers: a record column under the exchange
    /// schema of the root it is a batch of - its metadata and dictionary
    /// identities kept - and any other column under its own field.
    ///
    /// A `requested_schema` is read as a field and applied by the one cast,
    /// best effort as the interface asks; a run has no layout and raises.
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_array__<'py>(
        slf: &Bound<'py, Self>,
        requested_schema: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<(Bound<'py, PyCapsule>, Bound<'py, PyCapsule>)> {
        let py = slf.py();
        let serie = slf.try_borrow()?.inner.clone();
        let serie = match requested_schema {
            Some(requested) => {
                let field = serie.require_field().map_err(value_error)?;
                let target = requested_field(requested, field.name())?;
                py.detach(move || serie.cast(&target, ArrowCastOptions::new()))
                    .map_err(value_error)?
            }
            None => serie,
        };
        let array = py
            .detach(|| serie.require_arrow_array())
            .map_err(value_error)?;
        Ok((
            schema_capsule(py, capsule_schema(&serie)?)?,
            array_capsule(py, &array)?,
        ))
    }

    /// This column's rows as a `pyarrow.Table` of one batch.
    fn into_arrow_table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.into_arrow_reader(py)?.call_method0("read_all")
    }

    /// This column's rows as one pandas frame.
    fn into_pandas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = py
            .detach(|| self.inner.into_arrow_reader())
            .map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Pandas)
    }

    /// This column's rows as one polars frame.
    fn into_polars<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = py
            .detach(|| self.inner.into_arrow_reader())
            .map_err(value_error)?;
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

    fn __len__(&self, py: Python<'_>) -> usize {
        py.detach(|| self.inner.len())
    }

    /// Row `key`, negative from the end, or the window a step-1 slice names.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(slice) = key.cast::<PySlice>() {
            let window = slice
                .indices(isize::try_from(py.detach(|| self.inner.len())).unwrap_or(isize::MAX))?;
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
        self.scalar(py, self.index(py, index)?)?.into_py_any(py)
    }

    fn __setitem__(
        &mut self,
        py: Python<'_>,
        index: isize,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let index = self.index(py, index)?;
        self.set(py, index, value)
    }

    fn __delitem__(&mut self, py: Python<'_>, index: isize) -> PyResult<()> {
        let index = self.index(py, index)?;
        self.remove(py, index).map(|_| ())
    }

    fn __iter__(&self, py: Python<'_>) -> PyScalarIterator {
        py.detach(|| PyScalarIterator::new(self.inner.iter().map(Cow::into_owned)))
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let value = from_py(value)?;
        Ok(Python::attach(|py| {
            py.detach(|| self.inner.iter().any(|row| *row == value))
        }))
    }

    /// The call that rebuilds this serie, its rows spelled as Python values.
    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let serie = slf.try_borrow()?;
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
        let right = other.inner.clone();
        compare(other.py().detach(|| self.inner.cmp(&right)), operation).into_py_any(other.py())
    }

    // A serie is mutable, so it cannot promise a stable hash.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Rebuild through `_from_pickle` from the rows and the field.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, PickleArguments)> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (self.rows(py), self.field()),
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

pub(crate) fn serie_source_of(value: &Bound<'_, PyAny>) -> PyResult<Serie> {
    match columnar(value)? {
        Some(value) => native_columnar(value),
        None => chunked_stream_from_py(value, None, ArrowCastOptions::new()).map(Serie::from),
    }
}

pub(crate) fn native_columnar(value: Columnar) -> PyResult<Serie> {
    match value {
        Columnar::Held(serie) | Columnar::Pinned(serie) => Ok(serie),
        Columnar::Chunked(chunked) => Ok(Serie::from(chunked)),
        Columnar::Reader(reader) => Ok(Serie::from(*reader)),
        Columnar::Stream(reader) => {
            StreamChunkedSerie::from_arrow_reader(None, reader, ArrowCastOptions::new())
                .map(Serie::from)
                .map_err(value_error)
        }
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

    /// This record column as the Arrow `PyCapsule` Interface's stream
    /// capsule of its one batch - the stream `StreamChunkedSerie.from_serie` reads
    /// - cast into `requested_schema` by the one cast when a consumer asks.
    ///
    /// A record column is what a table's batch is, so it streams as one; a
    /// column of any other leaf is one array, as a `pyarrow.Array` is.
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_stream__<'py>(
        slf: &Bound<'py, Self>,
        requested_schema: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let serie = slf.as_super().try_borrow()?.inner.clone();
        let reader = StreamChunkedSerie::from_serie(serie).map_err(value_error)?;
        reader_capsule(slf.py(), reader, requested_schema)
    }
}
