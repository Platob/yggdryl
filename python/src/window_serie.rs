//! Python's native view of the shared [`WindowSerie`]: a window over a serie
//! that reads and writes through the serie's own implementation.
//!
//! [`PyWindowSerie`] holds the `Serie` object it was taken from, an offset
//! and a length - and, for a window `window_by` lent, the core's windows of
//! that call and its place among them: every call borrows that serie when
//! it is asked - shared for a read, mutably for a write - takes the core
//! window over it again, and redirects. One class is therefore both the
//! core's `WindowSerie` and its `WindowSerieMut`: Python has no borrow to
//! tell them apart, so a write is checked when it is made rather than when
//! the window is taken. A window taken before its serie shrank is refused
//! naming the serie and both counts, as the core refuses a window past the
//! end.
//!
//! A window `window_by` lent states its static values. Every window of one
//! call shares the core's [`SerieWindows`] of it, owned
//! ([`SerieWindows::into_owned`]): the windowed serie cloned, its buffers
//! shared and no row copied, so the windows outlive the call that cut them.
//! The window's record is what the core window those windows lend at its
//! place states ([`WindowSerie::static_values`]), built when it is read and
//! answered as the struct value `Scalar` reads by name - the rows as
//! `window_by` read them when it cut the windows - and its own `window_by`
//! redirects through that core window, so the windows of it keep the
//! record: the outer key cells first, the `rownum` absolute. Every other
//! window states none, as in the core, and so does a narrower one.
//!
//! The window's rows are its identity, as a serie's are: it compares by them
//! against a window or a `Serie`, and is unhashable because its serie is
//! mutable.

use std::sync::Arc;

use pyo3::IntoPyObjectExt;
use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PySlice};
use yggdryl::{Scalar, Serie, SerieWindows, WindowSerie, WindowSerieMut};

use crate::datatype::PyDataType;
use crate::expression::selector_from_value;
use crate::field::PyField;
use crate::scalar::{PyScalar, PyScalarIterator, from_py, from_py_under};
use crate::serie::{
    PySerie, described, groups_to_py, rows_from_py, serie_argument, sort_options, static_struct,
};
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
    /// Where `window_by` lent this window: the windows of that call, owned
    /// and shared by every window of it, and this window's place among them
    /// - its `windownum`. `None` for every other window.
    lent: Option<(Arc<SerieWindows<'static>>, usize)>,
}

/// The invariant a lent window keeps: its place is one of the windows that
/// lent it, which never change.
const LENT: &str = "a lent window's place is among the windows that lent it";

/// The windows one `window_by` call cut, read off the GIL into what
/// outlives the core's borrow: the windows themselves, owned, whether
/// `sorted` gathered the rows into key order - every window then views the
/// gathered copy they hold - and per window its key, its offset and its
/// length. No record is built here: a window builds its own when it is read.
pub(crate) struct LentWindows {
    windows: Arc<SerieWindows<'static>>,
    gathered: bool,
    cuts: Vec<(Scalar, usize, usize)>,
}

impl LentWindows {
    /// Read every window `windows` lends over `windowed` - its key, its
    /// offset and its length - and keep the windows, owned.
    pub(crate) fn of(windowed: &Serie, windows: SerieWindows<'_>) -> Self {
        let gathered = !std::ptr::eq(windows.serie(), windowed);
        let cuts = windows
            .iter()
            .map(|(key, window)| (key, window.offset(), window.len()))
            .collect();
        Self {
            windows: Arc::new(windows.into_owned()),
            gathered,
            cuts,
        }
    }

    /// Hand the windows to Python as `(key, window)` pairs: each window over
    /// `serie` - the windowed object itself - or, where `sorted` gathered the
    /// rows, over one new `Serie` of the gathered copy every window shares;
    /// each holding the windows and its place among them.
    pub(crate) fn into_py(
        self,
        py: Python<'_>,
        serie: Py<PySerie>,
    ) -> PyResult<Vec<(PyScalar, PyWindowSerie)>> {
        let serie = if self.gathered {
            described(py, self.windows.serie().clone())?
                .into_bound(py)
                .cast_into::<PySerie>()?
                .unbind()
        } else {
            serie
        };
        let windows = self.windows;
        Ok(self
            .cuts
            .into_iter()
            .enumerate()
            .map(|(index, (key, offset, len))| {
                let window = PyWindowSerie {
                    serie: serie.clone_ref(py),
                    offset,
                    len,
                    lent: Some((Arc::clone(&windows), index)),
                };
                (PyScalar::from_inner(key), window)
            })
            .collect())
    }
}

impl PyWindowSerie {
    /// The window `offset..offset + len` over `serie`, which the caller has
    /// proven against the serie's length; it states no record.
    pub(crate) const fn new(serie: Py<PySerie>, offset: usize, len: usize) -> Self {
        Self {
            serie,
            offset,
            len,
            lent: None,
        }
    }

    /// Answer `read` off the GIL through the core window `windows` lend at
    /// `index` - the window stating its record - reached in constant time
    /// ([`SerieWindows::get`]).
    fn lent<T, F>(
        py: Python<'_>,
        (windows, index): (Arc<SerieWindows<'static>>, usize),
        read: F,
    ) -> PyResult<T>
    where
        T: Send,
        F: for<'a> FnOnce(WindowSerie<'a>) -> PyResult<T> + Send,
    {
        py.detach(move || {
            let (_, window) = windows.get(index).expect(LENT);
            read(window)
        })
    }

    /// Answer `read` through the core window over the serie as it is now,
    /// borrowed for the call.
    fn read<T>(
        &self,
        py: Python<'_>,
        read: impl FnOnce(WindowSerie<'_>) -> PyResult<T>,
    ) -> PyResult<T> {
        let serie = self.serie.bind(py).borrow();
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
        let serie = self.serie.bind(py).borrow().inner.clone();
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
        let mut serie = self.serie.bind(py).borrow_mut();
        let window = serie
            .inner
            .window_mut(self.offset, self.len)
            .map_err(value_error)?;
        write(window).map_err(value_error)
    }

    /// A value for row writes: through the serie's field when it has one.
    fn value_of(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
        match self.serie.bind(py).borrow().inner.field() {
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
            let len = serie.borrow().inner.len();
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
        let source = serie.bind(py).borrow().inner.clone();
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

    /// A narrower window, `offset..offset + length` of this one, over the
    /// same serie object.
    fn window(&self, py: Python<'_>, offset: usize, length: usize) -> PyResult<Self> {
        let (offset, len) = self.read(py, |window| {
            let narrower = window.window(offset, length).map_err(value_error)?;
            Ok((narrower.offset(), narrower.len()))
        })?;
        Ok(Self::new(self.serie.clone_ref(py), offset, len))
    }

    /// The values constant over this window's rows, where `window_by` lent
    /// it: one struct value, its cells the kept cells of the window it was
    /// cut from, the key cells, `windownum` and `rownum`, as `window_by`
    /// read the rows when it cut the windows, read by name through
    /// `Scalar`'s own accessors. Built when it is read. `None` for every
    /// other window, a narrower one included.
    #[getter]
    fn static_values(&self, py: Python<'_>) -> PyResult<Option<PyScalar>> {
        let Some(lent) = self.lent.clone() else {
            return Ok(None);
        };
        let record = Self::lent(py, lent, |window| {
            window
                .static_values()
                .map(|record| static_struct(record.field(), record.value()))
                .transpose()
        })?;
        Ok(record.map(PyScalar::from_inner))
    }

    /// The windows of equal adjacent keys over this window's rows, each
    /// `(key, window)`: over the same serie object at its offsets, unless
    /// `sorted` gathered the rows into key order - then over one new `Serie`
    /// of them, which every window shares. A window `window_by` lent is cut
    /// through the core window that lent it, over the rows as they were when
    /// it was cut, and its windows keep its record: its cells but
    /// `windownum` and `rownum` ahead of the key cells, and an absolute
    /// `rownum`. `sorted=None` is `False`.
    #[pyo3(
        signature = (by, sorted = Some(false)),
        text_signature = "($self, by, sorted=False)"
    )]
    fn window_by(
        &self,
        py: Python<'_>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<Vec<(PyScalar, Self)>> {
        let selector = selector_from_value(by)?;
        let sorted = sorted.unwrap_or(false);
        let cut = |window: WindowSerie<'_>| -> yggdryl::Result<LentWindows> {
            Ok(LentWindows::of(
                window.serie(),
                window.window_by(selector, sorted)?,
            ))
        };
        let lent = match self.lent.clone() {
            // Proven against the serie as it is now, as every call is; the
            // windows of it view the serie object at the offsets the windows
            // that lent this one hold.
            Some(lent) => {
                self.read(py, |_| Ok(()))?;
                Self::lent(py, lent, |window| cut(window).map_err(value_error))?
            }
            None => self.detached(py, cut)?,
        };
        lent.into_py(py, self.serie.clone_ref(py))
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

    /// The window's rows grouped by `keys`, as long as the window.
    fn partition_by(
        &self,
        py: Python<'_>,
        keys: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<(PyScalar, Py<PyAny>)>> {
        let keys = serie_argument(keys, "keys")?;
        let groups = self.detached(py, move |window| window.partition_by(&keys))?;
        groups_to_py(py, groups)
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
        let field = self.serie.bind(py).borrow().inner.field().cloned();
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
        let other = serie.bind(py).borrow();
        let other = other.inner.window(offset, len).map_err(value_error)?;
        let ordering = self.read(py, |window| Ok(window.cmp(&other)))?;
        compare(ordering, operation).into_py_any(py)
    }
}
