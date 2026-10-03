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
use yggdryl::{ArrowCastOptions, ChunkedSerie, Field as CoreField, FieldPath, Scalar, SerieReader};

use crate::datatype::{
    ArrayIntake, PyDataType, arrow_array_to_pyarrow, core_field_to_pyarrow, pyarrow,
};
use crate::expression::selector_from_value;
use crate::field::{PyField, core_field_from_value};
use crate::graph::ellipsis;
use crate::iomedia::{
    Frames, core_root_field_from_value, frame_from_reader, rooted_reader_to_pyarrow, type_name,
};
use crate::join::{JoinKeywords, PyJoinOptions, join_keys_of, join_kind_of};
use crate::scalar::{PyScalar, PyScalarIterator, as_py_with_field, from_py};
use crate::serie::{
    PySerie, columnar, declared_texts, described, field_of, orderings_of, reader_capsule,
    requested_field, serie_argument, serie_from_value, sort_options, stream_of, target_of,
};
use crate::spill::{PySpillOptions, spill_options_of};
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

    /// Hand groups to Python as `(key, rows)` pairs.
    fn groups(groups: Vec<(Scalar, ChunkedSerie)>) -> Vec<(PyScalar, Self)> {
        groups
            .into_iter()
            .map(|(key, rows)| (PyScalar::from_inner(key), Self::from_inner(rows)))
            .collect()
    }

    /// Answer `read` over these chunks off the GIL, over a clone taken and
    /// released before it starts, as `cast` and `into_serie` do.
    fn detached<T, F>(slf: &Bound<'_, Self>, read: F) -> PyResult<T>
    where
        T: Send,
        F: FnOnce(ChunkedSerie) -> yggdryl::Result<T> + Send,
    {
        let chunked = slf.borrow().inner.clone();
        slf.py().detach(move || read(chunked)).map_err(value_error)
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
        // A chunkless chunked array of an extension type is a column of its
        // datatype, as its chunks would be.
        let declared = empty.stated_field().cloned();
        return py
            .detach(move || {
                let stated = ChunkedSerie::from_arrow_arrays(
                    declared.as_ref(),
                    [empty.validated()?],
                    options,
                )?;
                Ok::<_, yggdryl::arrow::Error>(ChunkedSerie::empty(stated.field().clone())?)
            })
            .map_err(value_error);
    }
    // A chunked array of an extension type states it on every chunk's type,
    // which is the field a caller who named none reads under; chunks stating
    // two of them are two columns, never one read as either.
    let stated = arrays.first().and_then(ArrayIntake::stated_field).cloned();
    if field.is_none() {
        let first = stated.as_ref().map(CoreField::dtype);
        if let Some((at, other)) = arrays
            .iter()
            .enumerate()
            .find(|(_, chunk)| chunk.stated_field().map(CoreField::dtype) != first)
        {
            return Err(PyValueError::new_err(format!(
                "expected every chunk of one datatype, got {} at chunk 0 and {} at chunk {at}",
                first.map_or_else(|| "its storage".to_owned(), ToString::to_string),
                other.stated_field().map_or_else(
                    || "its storage".to_owned(),
                    |field| field.dtype().to_string()
                ),
            )));
        }
    }
    let field = field.or(stated.as_ref());
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

    // ------------------------------------------------------------------
    // Ordering, uniqueness and grouping across the chunks: the reads off
    // the GIL over a clone, as `Serie`'s are; the `as_*` writes in place,
    // answering this same object so calls chain.
    // ------------------------------------------------------------------

    /// The row positions in sorted order across every chunk, as a `uint32`
    /// column named `index`: the one join, then the sort.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn sort_indices(
        slf: &Bound<'_, Self>,
        descending: bool,
        nulls_first: bool,
    ) -> PyResult<Py<PyAny>> {
        let options = sort_options(descending, nulls_first);
        let order = Self::detached(slf, move |chunked| chunked.sort_indices(options))?;
        described(slf.py(), order)
    }

    /// Whether the rows are in sorted order across the chunks: every chunk
    /// and every chunk edge read, none joined.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn is_sorted(slf: &Bound<'_, Self>, descending: bool, nulls_first: bool) -> PyResult<bool> {
        let options = sort_options(descending, nulls_first);
        Self::detached(slf, move |chunked| Ok(chunked.is_sorted(options)))
    }

    /// Whether no two rows across the chunks hold one value.
    fn is_unique(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Self::detached(slf, |chunked| Ok(chunked.is_unique()))
    }

    /// How many distinct values the rows hold across the chunks.
    fn unique_count(slf: &Bound<'_, Self>) -> PyResult<usize> {
        Self::detached(slf, |chunked| Ok(chunked.unique_count()))
    }

    /// The rows in sorted order, with no join: each chunk sorted on its own,
    /// then the sorted chunks merged into output chunks of at most
    /// `DEFAULT_RECORD_BATCH_ROW_SIZE` rows.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn into_sorted(slf: &Bound<'_, Self>, descending: bool, nulls_first: bool) -> PyResult<Self> {
        let options = sort_options(descending, nulls_first);
        Self::detached(slf, move |chunked| chunked.into_sorted(options)).map(Self::from_inner)
    }

    /// The row positions across every chunk in the order the `order by`
    /// keys of `by` state, as a `uint32` column named `index`: the keys
    /// resolved, then the one join, then the sort.
    fn sort_indices_by(slf: &Bound<'_, Self>, by: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let by = orderings_of(by)?;
        let order = Self::detached(slf, move |chunked| chunked.sort_indices_by(by))?;
        described(slf.py(), order)
    }

    /// The rows in the order the `order by` keys of `by` state, with no
    /// join: each chunk sorted on its own, then the sorted chunks merged, as
    /// `into_sorted` merges them; the field declares that order.
    fn into_sort_by(slf: &Bound<'_, Self>, by: &Bound<'_, PyAny>) -> PyResult<Self> {
        let by = orderings_of(by)?;
        Self::detached(slf, move |chunked| chunked.into_sort_by(by)).map(Self::from_inner)
    }

    /// The first occurrence of every value, in order of first occurrence,
    /// with no join: each chunk keeps its own first occurrences, kept apart,
    /// and a chunk left with no row is dropped.
    fn into_unique(slf: &Bound<'_, Self>) -> PyResult<Self> {
        Self::detached(slf, |chunked| chunked.into_unique()).map(Self::from_inner)
    }

    /// The rows in reverse order: the chunks reversed, each reversed.
    fn into_reversed(slf: &Bound<'_, Self>) -> PyResult<Self> {
        Self::detached(slf, |chunked| Ok(chunked.into_reversed())).map(Self::from_inner)
    }

    /// The rows `indices` names across the chunks, as one chunk.
    fn into_taken(slf: &Bound<'_, Self>, indices: &Bound<'_, PyAny>) -> PyResult<Self> {
        let indices = serie_argument(indices, "indices")?;
        Self::detached(slf, move |chunked| chunked.into_taken(&indices)).map(Self::from_inner)
    }

    /// The rows `mask` keeps, chunk by chunk and kept apart.
    fn into_filtered(slf: &Bound<'_, Self>, mask: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mask = serie_argument(mask, "mask")?;
        Self::detached(slf, move |chunked| chunked.into_filtered(&mask)).map(Self::from_inner)
    }

    /// The rows grouped by `keys`, as long as the whole: one `(key, rows)`
    /// per distinct key in order of first occurrence, each group's rows
    /// what each chunk contributed, kept apart. Keys held in chunks - a
    /// `ChunkedSerie`, a `pyarrow.ChunkedArray` or a table - are grouped
    /// chunk beside chunk where both are cut at the same rows, with no
    /// join, and the keys joined once and cut to the rows' chunks otherwise.
    fn partition_by(
        slf: &Bound<'_, Self>,
        keys: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<(PyScalar, Self)>> {
        let groups = if let Some(keys) = chunked_of(keys)? {
            Self::detached(slf, move |chunked| chunked.partition_by_chunked(&keys))?
        } else {
            let keys = serie_argument(keys, "keys")?;
            Self::detached(slf, move |chunked| chunked.partition_by(&keys))?
        };
        Ok(Self::groups(groups))
    }

    /// The windows of equal adjacent keys across the chunks, each
    /// `(key, rows)`: a run crossing a chunk edge is one window, its pieces
    /// kept apart, and `sorted` regroups the runs into key order with no row
    /// copied. A window states no record: its key is the first half of the
    /// pair and its place among the windows its place in the list, so a key
    /// cell named `windownum` or `rownum` is taken. `sorted=None` is `False`.
    #[pyo3(
        signature = (by, sorted = Some(false)),
        text_signature = "($self, by, sorted=False)"
    )]
    fn window_by(
        slf: &Bound<'_, Self>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<Vec<(PyScalar, Self)>> {
        let selector = selector_from_value(by)?;
        let sorted = sorted.unwrap_or(false);
        Self::detached(slf, move |chunked| chunked.window_by(selector, sorted)).map(Self::groups)
    }

    /// The bytes the rows occupy: every chunk's.
    fn memory_size(&self) -> usize {
        self.inner.memory_size()
    }

    /// The bytes the rows occupy in memory: every chunk's `resident_size`.
    fn resident_size(&self) -> usize {
        self.inner.resident_size()
    }

    /// Whether every chunk's rows lie in a spill file: no byte resident,
    /// and some bytes. No chunk is never spilled.
    fn is_spilled(&self) -> bool {
        self.inner.is_spilled()
    }

    /// Move chunks to disk until the resident bytes are under the bound, in
    /// place: the heaviest chunks spill whole first, so the lightest stay
    /// resident. `options` - the process default for `None` - takes
    /// `byte_size` and `folder` on a copy where given.
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
        slf.borrow_mut().inner.spill(&options).map_err(value_error)
    }

    /// The `order by` keys the field declares the rows keep across every
    /// chunk (`SORT:by`), each as that declaration spells it, or `None`.
    fn declared_order(&self) -> PyResult<Option<Vec<String>>> {
        declared_texts(self.inner.declared_order())
    }

    /// These chunks joined with `other` on `by`, under `how`: the output
    /// batches kept apart as chunks. `other` is a `ChunkedSerie` or
    /// anything `ChunkedSerie.from_` reads; `Serie.join_with` states the
    /// keys, the kinds and the options.
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
    ) -> PyResult<Self> {
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
        let other = chunked_from_py(other, None, ArrowCastOptions::new())?;
        Self::detached(slf, move |chunked| {
            chunked.join_with(&other, keys, how, &options)
        })
        .map(Self::from_inner)
    }

    /// Sort the rows in place, the merged chunks replacing the chunks,
    /// answering this chunked serie.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn as_sorted<'py>(
        slf: &Bound<'py, Self>,
        descending: bool,
        nulls_first: bool,
    ) -> PyResult<Bound<'py, Self>> {
        let options = sort_options(descending, nulls_first);
        slf.borrow_mut()
            .inner
            .as_sorted(options)
            .map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Sort the rows in place in the order the `order by` keys of `by`
    /// state, the merged chunks replacing the chunks, answering this
    /// chunked serie; a refusal leaves it as it was.
    fn as_sort_by<'py>(
        slf: &Bound<'py, Self>,
        by: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let by = orderings_of(by)?;
        slf.borrow_mut().inner.as_sort_by(by).map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Keep the first occurrence of every value, in place, each chunk
    /// filtered where it stands.
    fn as_unique<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        slf.borrow_mut().inner.as_unique().map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Reverse the rows in place: the chunks reversed, each where it stands.
    fn as_reversed<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        slf.borrow_mut().inner.as_reversed().map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Keep the rows `indices` names, in place, as one chunk.
    fn as_taken<'py>(
        slf: &Bound<'py, Self>,
        indices: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let indices = serie_argument(indices, "indices")?;
        slf.borrow_mut()
            .inner
            .as_taken(&indices)
            .map_err(value_error)?;
        Ok(slf.clone())
    }

    /// Keep the rows `mask` keeps, in place and chunk by chunk.
    fn as_filtered<'py>(
        slf: &Bound<'py, Self>,
        mask: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let mask = serie_argument(mask, "mask")?;
        slf.borrow_mut()
            .inner
            .as_filtered(&mask)
            .map_err(value_error)?;
        Ok(slf.clone())
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
