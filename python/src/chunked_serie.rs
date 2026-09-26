//! Python's native view of the shared [`ChunkedSerie`]: many columns under
//! one field, held apart - what `PyArrow` calls a `ChunkedArray`, and a
//! `Table` of one batch per chunk when the field is a record.
//!
//! [`PyChunkedSerie`] owns only the core value and redirects every verb to
//! it. Appending a chunk changes it, so it compares by its rows and has no
//! hash, as a `Serie` does. Arrow crosses one chunk at a time through the C
//! Data and C Stream interfaces, buffers shared, and nothing is joined but by
//! `into_serie`.
//!
//! Its intake is [`columnar`]'s one recognition ladder read as chunks: a
//! chunked array is its chunks, a stream one chunk per batch, a held column
//! its one chunk, and any other value what `Serie.from_` reads it as.

use pyo3::IntoPyObjectExt;
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{IntoPyDict, PyBool, PyList, PySlice};
use yggdryl::{ArrowCastOptions, ChunkedSerie, Field as CoreField, FieldPath, SerieReader};

use crate::datatype::{
    ArrayIntake, PyDataType, arrow_array_to_pyarrow, core_field_to_pyarrow, pyarrow,
};
use crate::field::{PyField, core_field_from_value};
use crate::iomedia::{
    Frames, core_root_field_from_value, frame_from_reader, rooted_reader_to_pyarrow, type_name,
};
use crate::scalar::{PyScalar, PyScalarIterator, as_py_with_field, from_py};
use crate::serie::{
    PySerie, columnar, described, field_of, reader_capsule, requested_field, serie_from_value,
    stream_of, target_of,
};
use crate::{cast_options, compare, normalize_index, value_error};

/// Many columns under one field, held apart: a chunked array, or a table.
#[pyclass(name = "ChunkedSerie", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyChunkedSerie {
    pub(crate) inner: ChunkedSerie,
}

impl PyChunkedSerie {
    pub(crate) const fn from_inner(inner: ChunkedSerie) -> Self {
        Self { inner }
    }

    /// Resolve a Python index against the rows, negative from the end.
    fn index(&self, index: isize) -> PyResult<usize> {
        normalize_index(index, self.inner.len()).ok_or_else(|| PyIndexError::new_err(index))
    }
}

/// What `from_series` takes back from a pickle: the chunks, and their field.
type PickleArguments = (Vec<Py<PyAny>>, PyField);

/// Read Arrow arrays as the chunks they are, of their own layout or cast
/// into `field`, each crossing the C Data Interface on its own with its
/// buffers shared.
///
/// A `pyarrow.ChunkedArray` is its chunks, one array - anything exporting
/// the C array protocol - its one chunk, and any other iterable is read as
/// arrays, one chunk each. A chunked array states its type even when it
/// holds no chunk, which no array can, so a chunkless one with no field is
/// the empty chunked serie of the field the core gives the empty array of
/// that type - `combine_chunks` answers it - and holds no chunk.
pub(crate) fn chunked_from_arrays(
    value: &Bound<'_, PyAny>,
    field: Option<&CoreField>,
    options: ArrowCastOptions,
) -> PyResult<ChunkedSerie> {
    let py = value.py();
    let chunked = value.is_instance(pyarrow::chunked_array(py)?)?;
    let arrays = if chunked {
        let chunks = value.getattr(intern!(py, "chunks"))?;
        chunks
            .try_iter()?
            .map(|chunk| ArrayIntake::from_value(&chunk?))
            .collect::<PyResult<Vec<_>>>()?
    } else if value.hasattr(intern!(py, "__arrow_c_array__"))? {
        vec![ArrayIntake::from_value(value)?]
    } else {
        let items = value.try_iter().map_err(|_| {
            PyTypeError::new_err(format!(
                "expected a pyarrow.ChunkedArray, an Arrow array, or an iterable of Arrow \
                 arrays, got {}",
                type_name(value)
            ))
        })?;
        items
            .map(|item| ArrayIntake::from_value(&item?))
            .collect::<PyResult<Vec<_>>>()?
    };
    if chunked && arrays.is_empty() && field.is_none() {
        let empty = ArrayIntake::from_value(&value.call_method0("combine_chunks")?)?;
        return py
            .detach(move || {
                let stated = ChunkedSerie::from_arrow_arrays(None, [empty.validated()?], options)?;
                Ok::<_, yggdryl::arrow::Error>(ChunkedSerie::empty(stated.field().clone())?)
            })
            .map_err(value_error);
    }
    // Every chunk is proven and landed in one detached section: a detach per
    // chunk would wait on the GIL once per chunk under contention.
    py.detach(move || {
        let arrays = arrays
            .into_iter()
            .map(ArrayIntake::validated)
            .collect::<Result<Vec<_>, _>>()?;
        ChunkedSerie::from_arrow_arrays(field, arrays, options)
    })
    .map_err(value_error)
}

/// Read any Python value as chunks under one field, cast into `field` when
/// one is given.
///
/// A columnar object is [`columnar`]'s, read as chunks: a chunked array or
/// a `ChunkedSerie` as its chunks, a stream one chunk per batch, and a held
/// column its one chunk. Any other value is read as `Serie.from_` reads it,
/// and is one chunk.
fn chunked_from_py(
    value: &Bound<'_, PyAny>,
    field: Option<&Bound<'_, PyAny>>,
    options: ArrowCastOptions,
) -> PyResult<ChunkedSerie> {
    if let Some(columnar) = columnar(value)? {
        let root = field
            .map(|field| core_root_field_from_value(field, columnar.name()))
            .transpose()?;
        return columnar.into_chunked(value.py(), root.as_ref(), options);
    }
    ChunkedSerie::from_serie(serie_from_value(value, field, options)?).map_err(value_error)
}

/// Read the chunked inputs a plan is applied to chunk by chunk - a
/// `ChunkedSerie`, shared, and a `pyarrow.ChunkedArray` or `pyarrow.Table`,
/// each landed with no field exactly as `ChunkedSerie.from_` lands it - or
/// answer `None` for any other value.
pub(crate) fn chunked_of(value: &Bound<'_, PyAny>) -> PyResult<Option<ChunkedSerie>> {
    let py = value.py();
    let chunked = value.extract::<PyRef<'_, PyChunkedSerie>>().is_ok()
        || value.is_instance(pyarrow::chunked_array(py)?)?
        || value.is_instance(pyarrow::table(py)?)?;
    if !chunked {
        return Ok(None);
    }
    columnar(value)?
        .map(|columnar| columnar.into_chunked(py, None, ArrowCastOptions::new()))
        .transpose()
}

#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
#[pymethods]
impl PyChunkedSerie {
    // Appending a chunk changes it, so it cannot promise a stable hash.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The chunked serie of no chunks under `field`.
    #[staticmethod]
    fn empty(field: &Bound<'_, PyAny>) -> PyResult<Self> {
        ChunkedSerie::empty(core_field_from_value(field)?)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// One held column as the one chunk it is, sharing its buffers; a run
    /// names no field and is refused.
    #[staticmethod]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn from_serie(serie: PyRef<'_, PySerie>) -> PyResult<Self> {
        ChunkedSerie::from_serie(serie.inner.clone())
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Hold `chunks`, each a `Serie` column, under one field: the first
    /// chunk's own, nullable where any chunk's is, or `field`.
    ///
    /// With no field the chunks are one datatype in pieces, and a chunk of
    /// another datatype is refused naming it. A chunk already under the
    /// field is held as it stands; with `field`, any other is cast into it,
    /// one plan per run of chunks under one source field. No chunk and no
    /// field is refused, because nothing names the field.
    #[staticmethod]
    #[pyo3(signature = (chunks, field = None, *, safe = true, representation = "value"))]
    fn from_series(
        chunks: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, representation)?;
        let field = field_of(field)?;
        let chunks = chunks
            .try_iter()?
            .map(|chunk| {
                let chunk = chunk?;
                chunk
                    .extract::<PyRef<'_, PySerie>>()
                    .map(|serie| serie.inner.clone())
                    .map_err(|_| {
                        PyTypeError::new_err(format!(
                            "expected every chunk to be a Serie, got {}",
                            type_name(&chunk)
                        ))
                    })
            })
            .collect::<PyResult<Vec<_>>>()?;
        ChunkedSerie::from_series(field.as_ref(), chunks, options)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Take Arrow arrays as chunks: of their own layout, or cast into
    /// `field`, one plan per run of arrays of one layout.
    ///
    /// A `pyarrow.ChunkedArray` is its chunks, each crossing the C Data
    /// Interface with its buffers shared; an iterable of arrays is read the
    /// same way. With no field the chunks are the column of the first one's
    /// layout, named `item`, and a chunk of another datatype is refused.
    #[staticmethod]
    #[pyo3(signature = (chunked, field = None, *, safe = true, representation = "value"))]
    fn from_arrow_chunked_array(
        chunked: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, representation)?;
        let field = field_of(field)?;
        chunked_from_arrays(chunked, field.as_ref(), options).map(Self::from_inner)
    }

    /// Drain a record batch stream into its chunks, one per batch and none
    /// joined: of its own schema, or cast into `root` by one plan.
    #[staticmethod]
    #[pyo3(signature = (reader, root = None, *, safe = true, representation = "value"))]
    fn from_arrow_reader(
        reader: &Bound<'_, PyAny>,
        root: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, representation)?;
        let root = field_of(root)?;
        let stream = stream_of(reader)?;
        reader
            .py()
            .detach(move || ChunkedSerie::from_arrow_reader(root.as_ref(), stream, options))
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Read any columnar object, or any value a `Scalar` holds, as chunks:
    /// of its own field, or cast into `field`.
    ///
    /// A `pyarrow.ChunkedArray` is its chunks; a table, a reader, a dataset,
    /// a scanner and a pandas or polars frame one chunk per batch; a batch,
    /// an array, an Arrow scalar, a pandas or polars series and a `NumPy`
    /// array one chunk; a `Serie` its one chunk, shared; a `SerieReader` is
    /// taken and drained, one chunk per batch; a `ChunkedSerie` is shared.
    /// Any other value is read as `Serie.from_` reads it, and is one chunk.
    #[staticmethod]
    #[pyo3(name = "from_")]
    #[pyo3(signature = (value, field = None, *, safe = true, representation = "value"))]
    fn from_(
        value: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, representation)?;
        chunked_from_py(value, field, options).map(Self::from_inner)
    }

    /// The field every chunk is typed by.
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.inner.field().clone())
    }

    /// `serie(<the field named item>)`: what a column of this field declares.
    #[getter]
    fn dtype(&self) -> PyDataType {
        PyDataType::from_inner(self.inner.dtype())
    }

    /// Every chunk, in order, each handed out as its own leaf class.
    #[getter]
    fn chunks(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        self.inner
            .chunks()
            .iter()
            .map(|chunk| described(py, chunk.clone()))
            .collect()
    }

    /// How many chunks the rows are cut into.
    #[getter]
    fn num_chunks(&self) -> usize {
        self.inner.num_chunks()
    }

    /// Chunk `index`, or `None` past the last.
    fn chunk(&self, py: Python<'_>, index: usize) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .chunk(index)
            .map(|chunk| described(py, chunk.clone()))
            .transpose()
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

    /// Row `index`, built as one value out of the chunk that holds it.
    fn scalar(&self, index: usize) -> PyResult<PyScalar> {
        self.inner
            .scalar(index)
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Row `index`, or `None` past the end.
    fn get(&self, index: usize) -> Option<PyScalar> {
        self.inner.get(index).map(PyScalar::from_inner)
    }

    /// Every row, chunk after chunk, each built once.
    fn rows(&self) -> Vec<PyScalar> {
        self.inner
            .rows()
            .into_iter()
            .map(PyScalar::from_inner)
            .collect()
    }

    /// Every row as the Python value it is, a record as a `dict`.
    fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let list = PyList::empty(py);
        let field = self.inner.field();
        for row in &self.inner {
            list.append(as_py_with_field(py, &row, field)?)?;
        }
        Ok(list.into_any().unbind())
    }

    /// `length` rows from `offset`, keeping the chunks the window reaches.
    fn slice(&self, offset: usize, length: usize) -> PyResult<Self> {
        self.inner
            .slice(offset, length)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// A record's child named `name` in every chunk - a table's column - or
    /// `None`.
    fn child(&self, name: &str) -> Option<Self> {
        self.inner.child(name).map(Self::from_inner)
    }

    /// A record's child, or a union's member, at `index` in every chunk.
    fn child_at(&self, index: usize) -> Option<Self> {
        self.inner.child_at(index).map(Self::from_inner)
    }

    /// Every child of a record, or member of a union, in every chunk.
    fn children(&self) -> Vec<Self> {
        self.inner
            .children()
            .into_iter()
            .map(Self::from_inner)
            .collect()
    }

    /// A sequence's items, a mapping's entries, an encoding's values.
    fn items(&self) -> Option<Self> {
        self.inner.items().map(Self::from_inner)
    }

    /// The chunked column `path` reaches, spelled as a field path.
    fn get_child_by_path(&self, path: &str) -> PyResult<Option<Self>> {
        let path = FieldPath::from_str(path).map_err(value_error)?;
        Ok(self.inner.get_child_by_path(&path).map(Self::from_inner))
    }

    /// Append one chunk: a column under the field as it stands, any other
    /// column cast into it. A refusal leaves this serie as it was.
    #[pyo3(signature = (chunk, *, safe = true, representation = "value"))]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn push_chunk(
        &mut self,
        chunk: PyRef<'_, PySerie>,
        safe: bool,
        representation: &str,
    ) -> PyResult<()> {
        let options = cast_options(safe, representation)?;
        self.inner
            .push_chunk(chunk.inner.clone(), options)
            .map_err(value_error)
    }

    /// Every row as one column: the one join, off the GIL over a clone of
    /// the chunks taken and released before it starts, handed out as its
    /// leaf class.
    fn into_serie(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let chunked = slf.borrow().inner.clone();
        let serie = py
            .detach(move || chunked.into_serie())
            .map_err(value_error)?;
        described(py, serie)
    }

    /// Every chunk under `field`, one plan compiled and applied to each; a
    /// `DataType` is the required column named `value` it declares. The
    /// cast runs off the GIL, as `into_serie` does.
    #[pyo3(signature = (field, *, safe = true, representation = "value"))]
    fn cast(
        slf: &Bound<'_, Self>,
        field: &Bound<'_, PyAny>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, representation)?;
        let target = target_of(field)?;
        let chunked = slf.borrow().inner.clone();
        slf.py()
            .detach(move || chunked.cast(&target, options))
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Every chunk's buffers as one `pyarrow.ChunkedArray`, shared: one
    /// array per chunk, each crossing under the field's exact C schema, and
    /// typed by the field even with no chunk.
    fn into_arrow_chunked_array<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let field = self.inner.field();
        let chunks = self
            .inner
            .into_arrow_arrays()
            .iter()
            .map(|array| arrow_array_to_pyarrow(py, array, Some(field)))
            .collect::<PyResult<Vec<_>>>()?;
        let dtype = core_field_to_pyarrow(py, field)?.getattr(intern!(py, "type"))?;
        py.import("pyarrow")?.call_method(
            "chunked_array",
            (PyList::new(py, chunks)?,),
            Some(&[("type", dtype)].into_py_dict(py)?),
        )
    }

    /// Every chunk as one batch of a `pyarrow.RecordBatchReader`: a record's
    /// chunks as their children, any other's each the one column of a `row`.
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let root = SerieReader::root_of(self.inner.field()).map_err(value_error)?;
        let reader = self.inner.into_arrow_reader().map_err(value_error)?;
        rooted_reader_to_pyarrow(py, &root, reader)
    }

    /// These chunks as the Arrow `PyCapsule` Interface's stream capsule, one
    /// array per chunk, cast into `requested_schema` by the one cast when a
    /// consumer asks.
    ///
    /// A record's chunks stream as the batches `SerieReader.from_chunked`
    /// reads; any other field's chunks as the column `pyarrow.ChunkedArray`
    /// streams, which is how a stream carries a column that is no record.
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_stream__<'py>(
        slf: &Bound<'py, Self>,
        requested_schema: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let chunked = slf.borrow().inner.clone();
        if chunked.field().dtype().as_fields().is_some() {
            let reader = SerieReader::from_chunked(chunked).map_err(value_error)?;
            return reader_capsule(py, reader, requested_schema);
        }
        let chunked = match requested_schema {
            Some(requested) => {
                let target = requested_field(requested, chunked.field().name())?;
                py.detach(move || chunked.cast(&target, ArrowCastOptions::new()))
                    .map_err(value_error)?
            }
            None => chunked,
        };
        Self::from_inner(chunked)
            .into_arrow_chunked_array(py)?
            .call_method0(intern!(py, "__arrow_c_stream__"))
    }

    /// Every chunk as one batch of a `pyarrow.Table`.
    fn into_arrow_table<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.into_arrow_reader(py)?.call_method0("read_all")
    }

    /// Every row as one pandas frame.
    fn into_pandas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.inner.into_arrow_reader().map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Pandas)
    }

    /// Every row as one polars frame.
    fn into_polars<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.inner.into_arrow_reader().map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Polars)
    }

    /// Every row as one `NumPy` array, which `PyArrow`'s own conversion of
    /// the chunked array answers - allowed to copy, since `NumPy` has no
    /// null mask, no nested layout and no chunks.
    fn into_numpy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.into_arrow_chunked_array(py)?.call_method(
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
                    "a ChunkedSerie slices with a step of 1 only",
                ));
            }
            let start = usize::try_from(window.start).unwrap_or(0);
            return self.slice(start, window.slicelength)?.into_py_any(py);
        }
        if key.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err("ChunkedSerie indexes must be int"));
        }
        let index = key
            .extract::<isize>()
            .map_err(|_| PyTypeError::new_err("ChunkedSerie indexes must be int or slice"))?;
        self.scalar(self.index(index)?)?.into_py_any(py)
    }

    fn __iter__(&self) -> PyScalarIterator {
        PyScalarIterator::new(self.inner.iter())
    }

    fn __contains__(&self, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let value = from_py(value)?;
        Ok(self.inner.iter().any(|row| row == value))
    }

    /// The `from_series` call over each chunk's own repr and the field: it
    /// rebuilds this chunked serie wherever a chunk's repr rebuilds the
    /// chunk, which a record's rows, spelled as mappings, do not.
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let chunks = self
            .chunks(py)?
            .iter()
            .map(|chunk| Ok(chunk.bind(py).repr()?.to_string()))
            .collect::<PyResult<Vec<_>>>()?
            .join(", ");
        let field = Bound::new(py, self.field())?.repr()?.to_string();
        Ok(format!("ChunkedSerie.from_series([{chunks}], {field})"))
    }

    /// Equality and order over the rows alone, against a `ChunkedSerie` or a
    /// `Serie`, however either is cut.
    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let ordering = if let Ok(other) = other.extract::<PyRef<'_, Self>>() {
            self.inner.cmp(&other.inner)
        } else if let Ok(other) = other.extract::<PyRef<'_, PySerie>>() {
            // The core orders a chunked serie against a column by the rows,
            // as it does two chunked series, so no chunk is joined here.
            match self.inner.partial_cmp(&other.inner) {
                Some(ordering) => ordering,
                None => return Ok(py.NotImplemented()),
            }
        } else {
            return Ok(py.NotImplemented());
        };
        compare(ordering, operation).into_py_any(py)
    }

    /// Rebuild through `from_series` from the chunks and the field.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, PickleArguments)> {
        Ok((
            py.get_type::<Self>().getattr("from_series")?.unbind(),
            (self.chunks(py)?, self.field()),
        ))
    }

    /// A copy sharing every chunk's buffers.
    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}
