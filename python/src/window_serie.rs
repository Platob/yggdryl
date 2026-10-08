//! Live borrowed windows over a native Serie.

use pyo3::IntoPyObjectExt;
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;

// A free function accepts every window loan lifetime; the method pointer
// fixes the impl lifetime and cannot satisfy `detached`'s higher-ranked bound.
fn into_window_stream(window: yggdryl::WindowSerie<'_>) -> yggdryl::Result<yggdryl::StreamSerie> {
    window.into_stream()
}
use pyo3::types::{PyBool, PySlice};
use yggdryl::{Scalar, Serie, WindowSerie, WindowSerieMut};

use crate::datatype::PyDataType;
use crate::field::PyField;
use crate::key_serie::{PyKeySeries, key_by};
use crate::scalar::{PyScalar, PyScalarIterator, from_py, from_py_under};
use crate::serie::{PySerie, described, orderings_of, rows_from_py, serie_argument, sort_options};
use crate::stream_chunked_serie::PyStreamChunkedSerie;
use crate::stream_serie::PyStreamSerie;
use crate::{compare, normalize_index, value_error};

/// A window over a `Serie`, read and written through it, window-relative.
#[pyclass(
    name = "WindowSerie",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyWindowSerie {
    serie: Py<PySerie>,
    offset: usize,
    len: usize,
}

impl PyWindowSerie {
    /// The window `offset..offset + len` over `serie`, which the caller has
    /// proven against the serie's length; it states no record.
    pub(crate) const fn new(serie: Py<PySerie>, offset: usize, len: usize) -> Self {
        Self { serie, offset, len }
    }

    /// Answer `read` through the core window over the serie as it is now,
    /// borrowed for the call.
    fn read<T>(
        &self,
        py: Python<'_>,
        read: impl FnOnce(WindowSerie<'_>) -> PyResult<T>,
    ) -> PyResult<T> {
        let serie = self.serie.bind(py).try_borrow()?;
        read(
            serie
                .inner
                .window(self.offset, self.len)
                .map_err(value_error)?,
        )
    }

    /// Answer `read` off the GIL, over a clone of the serie taken and
    /// released before it starts, as `Serie`'s own ordering reads do.
    fn detached<T, F>(&self, py: Python<'_>, read: F) -> PyResult<T>
    where
        T: Send,
        F: for<'a> FnOnce(WindowSerie<'a>) -> yggdryl::Result<T> + Send,
    {
        let serie = self.serie.bind(py).try_borrow()?.inner.clone();
        let (offset, len) = (self.offset, self.len);
        py.detach(move || read(serie.window(offset, len)?))
            .map_err(value_error)
    }

    /// Answer `write` through the core's mutable window over the serie,
    /// borrowed mutably for the call.
    fn write<T>(
        &self,
        py: Python<'_>,
        write: impl FnOnce(WindowSerieMut<'_>) -> yggdryl::Result<T>,
    ) -> PyResult<T> {
        let mut serie = self.serie.bind(py).try_borrow_mut()?;
        let window = serie
            .inner
            .window_mut(self.offset, self.len)
            .map_err(value_error)?;
        write(window).map_err(value_error)
    }

    /// A value for row writes: through the serie's field when it has one.
    fn value_of(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
        match self.serie.bind(py).try_borrow()?.inner.field() {
            Some(field) => from_py_under(field, value),
            None => from_py(value),
        }
    }

    /// Resolve a Python index against the window, negative from the end.
    fn index(&self, index: isize) -> PyResult<usize> {
        normalize_index(index, self.len).ok_or_else(|| PyIndexError::new_err(index))
    }

    /// The serie object and the range a `WindowSerie` or `Serie` argument
    /// views: a `Serie` as its whole window.
    fn viewed(other: &Bound<'_, PyAny>) -> PyResult<(Py<PySerie>, usize, usize)> {
        if let Ok(window) = other.cast::<Self>() {
            let window = window.get();
            return Ok((
                window.serie.clone_ref(other.py()),
                window.offset,
                window.len,
            ));
        }
        if let Ok(serie) = other.cast::<PySerie>() {
            let len = serie.try_borrow()?.inner.len();
            return Ok((serie.clone().unbind(), 0, len));
        }
        Err(PyTypeError::new_err(format!(
            "expected a WindowSerie or a Serie, got {}",
            crate::iomedia::type_name(other)
        )))
    }

    /// The serie and range a `copy_from` source views, the serie as a
    /// clone sharing its buffers: the mutable borrow the write takes then
    /// never meets a shared one, and a source over this window's own serie
    /// costs the one copy of its buffers the write makes.
    fn source_of(py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<(Serie, usize, usize)> {
        let (serie, offset, len) = Self::viewed(other)?;
        let source = serie.bind(py).try_borrow()?.inner.clone();
        Ok((source, offset, len))
    }
}

#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
#[pymethods]
impl PyWindowSerie {
    // The rows are the identity and the serie under them is mutable.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The serie row the window starts at.
    #[getter]
    fn offset(&self) -> usize {
        self.offset
    }

    /// The whole serie the window reads and writes through: the same object.
    #[getter]
    fn serie(&self, py: Python<'_>) -> Py<PySerie> {
        self.serie.clone_ref(py)
    }

    /// The field every row is typed by, or `None` over a run.
    #[getter]
    fn field(&self, py: Python<'_>) -> PyResult<Option<PyField>> {
        self.read(py, |window| {
            Ok(window.field().cloned().map(PyField::from_inner))
        })
    }

    /// The datatype the window materializes into.
    #[getter]
    fn dtype(&self, py: Python<'_>) -> PyResult<PyDataType> {
        self.read(py, |window| {
            window
                .dtype()
                .map(PyDataType::from_inner)
                .map_err(value_error)
        })
    }

    /// How many rows of the window hold no value.
    fn null_count(&self, py: Python<'_>) -> PyResult<usize> {
        self.read(py, |window| Ok(window.null_count()))
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether window row `index` holds no value.
    fn is_null(&self, py: Python<'_>, index: usize) -> PyResult<bool> {
        self.read(py, |window| window.is_null(index).map_err(value_error))
    }

    /// Window row `index`, built as one value.
    fn scalar(&self, py: Python<'_>, index: usize) -> PyResult<PyScalar> {
        self.read(py, |window| {
            window
                .scalar(index)
                .map(PyScalar::from_inner)
                .map_err(value_error)
        })
    }

    /// Window row `index`, or `None` past the window.
    fn get(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyScalar>> {
        self.read(py, |window| {
            Ok(window
                .get(index)
                .map(|row| PyScalar::from_inner(row.into_owned())))
        })
    }

    /// Every row of the window, each built once and kept by the list alone.
    fn rows(&self, py: Python<'_>) -> PyResult<Vec<PyScalar>> {
        self.read(py, |window| {
            Ok(window
                .rows()
                .into_owned()
                .into_iter()
                .map(PyScalar::from_inner)
                .collect())
        })
    }

    /// Every row as the Python value it is, as `Serie.as_py` reads it.
    fn as_py(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let serie = self.read(py, |window| Ok(window.into_serie()))?;
        PySerie::from_inner(serie).as_py(py)
    }

    /// The bytes the window's rows occupy.
    fn memory_size(&self, py: Python<'_>) -> PyResult<usize> {
        self.read(py, |window| Ok(window.memory_size()))
    }

    /// The bytes the window's rows occupy in memory, read through the serie
    /// it views: a window is never spilled on its own - spill the serie.
    fn resident_size(&self, py: Python<'_>) -> PyResult<usize> {
        self.read(py, |window| Ok(window.resident_size()))
    }

    /// Whether the window's rows lie in a spill file: the serie's own answer
    /// over the rows it views.
    fn is_spilled(&self, py: Python<'_>) -> PyResult<bool> {
        self.read(py, |window| Ok(window.is_spilled()))
    }

    /// A narrower window, `offset..offset + length` of this one, over the
    /// same serie object.
    fn window(&self, py: Python<'_>, offset: usize, length: usize) -> PyResult<Self> {
        let (offset, len) = self.read(py, |window| {
            let narrower = window.window(offset, length).map_err(value_error)?;
            Ok((narrower.offset(), narrower.len()))
        })?;
        Ok(Self::new(self.serie.clone_ref(py), offset, len))
    }

    /// The windows of equal adjacent keys over this window's rows, each
    /// `(key, window)`: over the same serie object at its offsets, unless
    /// `sorted` gathered the rows into key order - then over one new `Serie`
    /// of them, which every window shares. A window `window_by` lent is cut
    /// through the core window that lent it, over the rows as they were when
    /// it was cut, and its windows keep its record: its cells but
    /// `windownum` and `rownum` ahead of the key cells, and an absolute
    /// `rownum`. `sorted=None` is `False`.
    #[pyo3(signature = (by, sorted = Some(false)))]
    fn window_by(
        &self,
        py: Python<'_>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<PyKeySeries> {
        let by = key_by(by)?;
        self.detached(py, move |window| {
            window.window_by(by, sorted.unwrap_or(false))
        })
        .map(PyKeySeries::from_inner)
    }
    fn into_stream(&self, py: Python<'_>) -> PyResult<PyStreamSerie> {
        self.detached(py, into_window_stream)
            .map(PyStreamSerie::from_core)
    }
    #[pyo3(signature = (row_size = None, byte_size = None))]
    fn into_chunked_stream(
        &self,
        py: Python<'_>,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> PyResult<PyStreamChunkedSerie> {
        let serie = self.serie.bind(py).try_borrow()?.inner.clone();
        let (offset, len) = (self.offset, self.len);
        py.detach(move || {
            serie
                .window(offset, len)?
                .into_chunked_stream(row_size, byte_size)
        })
        .map(PyStreamChunkedSerie::from)
        .map_err(value_error)
    }

    /// The window as a serie of its own: zero copy for a column; its
    /// record stays with the window.
    fn into_serie(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let serie = self.read(py, |window| Ok(window.into_serie()))?;
        described(py, serie)
    }

    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn is_sorted(&self, py: Python<'_>, descending: bool, nulls_first: bool) -> PyResult<bool> {
        let options = sort_options(descending, nulls_first);
        self.detached(py, move |window| Ok(window.is_sorted(options)))
    }

    fn is_unique(&self, py: Python<'_>) -> PyResult<bool> {
        self.detached(py, |window| Ok(window.is_unique()))
    }

    fn unique_count(&self, py: Python<'_>) -> PyResult<usize> {
        self.detached(py, |window| Ok(window.unique_count()))
    }

    /// The window-relative row positions in sorted order.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn sort_indices(
        &self,
        py: Python<'_>,
        descending: bool,
        nulls_first: bool,
    ) -> PyResult<Py<PyAny>> {
        let options = sort_options(descending, nulls_first);
        described(
            py,
            self.detached(py, move |window| window.sort_indices(options))?,
        )
    }

    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn into_sorted(
        &self,
        py: Python<'_>,
        descending: bool,
        nulls_first: bool,
    ) -> PyResult<Py<PyAny>> {
        let options = sort_options(descending, nulls_first);
        described(
            py,
            self.detached(py, move |window| window.into_sorted(options))?,
        )
    }

    /// The window-relative row positions in the order the `order by` keys
    /// of `by` state, computed over the window's rows alone.
    fn sort_indices_by(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let by = orderings_of(by)?;
        described(
            py,
            self.detached(py, move |window| window.sort_indices_by(by))?,
        )
    }

    /// The window's rows in the order the `order by` keys of `by` state, as
    /// a serie of their own.
    fn into_sort_by(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let by = orderings_of(by)?;
        described(
            py,
            self.detached(py, move |window| window.into_sort_by(by))?,
        )
    }

    fn into_unique(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(py, self.detached(py, |window| window.into_unique())?)
    }

    fn into_reversed(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(py, self.detached(py, |window| Ok(window.into_reversed()))?)
    }

    /// The window's rows `indices` names, window-relative.
    fn into_taken(&self, py: Python<'_>, indices: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let indices = serie_argument(indices, "indices")?;
        described(
            py,
            self.detached(py, move |window| window.into_taken(&indices))?,
        )
    }

    /// The window's rows `mask` keeps.
    fn into_filtered(&self, py: Python<'_>, mask: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let mask = serie_argument(mask, "mask")?;
        described(
            py,
            self.detached(py, move |window| window.into_filtered(&mask))?,
        )
    }

    fn partition_by(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<PyKeySeries> {
        let by = key_by(by)?;
        self.detached(py, move |window| window.partition_by(by))
            .map(PyKeySeries::from_inner)
    }

    // ------------------------------------------------------------------
    // Writes, each through the serie's own `set` or `splice` on the
    // rebased range: the serie borrowed mutably for the call.
    // ------------------------------------------------------------------

    /// Overwrite window row `index`, through the field's contract.
    fn set(&self, py: Python<'_>, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = self.value_of(py, value)?;
        self.write(py, |mut window| window.set(index, value))
    }

    /// Overwrite every window row with `value`, proved once.
    fn fill(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = self.value_of(py, value)?;
        self.write(py, |mut window| window.fill(value))
    }

    /// Swap window rows `left` and `right`.
    fn swap(&self, py: Python<'_>, left: usize, right: usize) -> PyResult<()> {
        self.write(py, |mut window| window.swap(left, right))
    }

    /// Overwrite the window with `other`'s rows, row for row: a window or a
    /// `Serie` exactly as long as this one.
    fn copy_from(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<()> {
        let (source, offset, len) = Self::source_of(py, other)?;
        let source = source.window(offset, len).map_err(value_error)?;
        self.write(py, |mut window| window.copy_from(&source))
    }

    /// Replace window rows `start..end` by exactly as many `rows`.
    fn splice(
        &self,
        py: Python<'_>,
        start: usize,
        end: usize,
        rows: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let field = self.serie.bind(py).try_borrow()?.inner.field().cloned();
        let rows = rows_from_py(field.as_ref(), rows)?;
        self.write(py, |mut window| window.splice(start..end, rows))
    }

    /// Sort the window's rows in place, answering this window.
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn as_sorted<'py>(
        slf: &Bound<'py, Self>,
        descending: bool,
        nulls_first: bool,
    ) -> PyResult<Bound<'py, Self>> {
        let options = sort_options(descending, nulls_first);
        slf.get()
            .write(slf.py(), |mut window| window.as_sorted(options).map(|_| ()))?;
        Ok(slf.clone())
    }

    /// Sort the window's rows in place in the order the `order by` keys of
    /// `by` state, answering this window: every row outside it untouched,
    /// and a refusal leaving the serie as it was.
    fn as_sort_by<'py>(
        slf: &Bound<'py, Self>,
        by: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let by = orderings_of(by)?;
        slf.get()
            .write(slf.py(), |mut window| window.as_sort_by(by).map(|_| ()))?;
        Ok(slf.clone())
    }

    /// Reverse the window's rows in place, answering this window.
    fn as_reversed<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, Self>> {
        slf.get()
            .write(slf.py(), |mut window| window.as_reversed().map(|_| ()))?;
        Ok(slf.clone())
    }

    /// Rearrange the window's rows as `indices` names them, exactly as
    /// many as the window holds, answering this window.
    fn as_taken<'py>(
        slf: &Bound<'py, Self>,
        indices: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        let indices = serie_argument(indices, "indices")?;
        slf.get()
            .write(slf.py(), |mut window| window.as_taken(&indices).map(|_| ()))?;
        Ok(slf.clone())
    }

    fn __len__(&self) -> usize {
        self.len
    }

    /// Window row `key`, negative from the end, or the narrower window a
    /// step-1 slice names.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        if let Ok(slice) = key.cast::<PySlice>() {
            let window = slice.indices(isize::try_from(self.len).unwrap_or(isize::MAX))?;
            if window.step != 1 {
                return Err(PyValueError::new_err(
                    "a WindowSerie slices with a step of 1 only",
                ));
            }
            let start = usize::try_from(window.start).unwrap_or(0);
            return self.window(py, start, window.slicelength)?.into_py_any(py);
        }
        if key.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err("WindowSerie indexes must be int"));
        }
        let index = key
            .extract::<isize>()
            .map_err(|_| PyTypeError::new_err("WindowSerie indexes must be int or slice"))?;
        self.scalar(py, self.index(index)?)?.into_py_any(py)
    }

    fn __setitem__(&self, py: Python<'_>, index: isize, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let index = self.index(index)?;
        self.set(py, index, value)
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<PyScalarIterator> {
        self.read(py, |window| {
            Ok(PyScalarIterator::new(window.rows().into_owned()))
        })
    }

    fn __contains__(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<bool> {
        let value = from_py(value)?;
        self.read(py, |window| Ok(window.iter().any(|row| *row == value)))
    }

    /// The window as the call that takes it, over its serie's repr.
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let serie = self.serie.bind(py).repr()?;
        Ok(format!("{serie}.window({}, {})", self.offset, self.len))
    }

    /// Equality and order over the rows alone, against a window or a
    /// `Serie`, as the core defines them.
    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok((serie, offset, len)) = Self::viewed(other) else {
            return Ok(py.NotImplemented());
        };
        let other = serie.bind(py).try_borrow()?;
        let other = other.inner.window(offset, len).map_err(value_error)?;
        let ordering = self.read(py, |window| Ok(window.cmp(&other)))?;
        compare(ordering, operation).into_py_any(py)
    }
}
