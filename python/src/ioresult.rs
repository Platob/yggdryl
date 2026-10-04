//! Python's view of the core [`IOResult`]: immutable, compared, hashed and
//! pickled by its three counts; `+` is the core's `Add`, `str` its `Display`.

use pyo3::class::basic::CompareOp;
use pyo3::prelude::*;
use yggdryl::IOResult;

use crate::{compare, python_hash};

/// The rows one record write read, wrote and skipped - what a `where` kept
/// out, or a bound cut off. A write cut into several commits answers their
/// sum. Immutable.
#[pyclass(
    name = "IOResult",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
// `Default` is the core's: the result of a write that read nothing.
#[derive(Clone, Copy, Default)]
pub(crate) struct PyIOResult {
    inner: IOResult,
}

impl PyIOResult {
    /// Wrap a value the core answered.
    pub(crate) const fn from_core(inner: IOResult) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyIOResult {
    /// `skipped_rows` omitted is the read rows the write did not write,
    /// never below zero; stated, the three counts are taken as they are - a
    /// sum's own, which `repr` and pickle rebuild.
    #[new]
    #[pyo3(signature = (read_rows = 0, written_rows = 0, skipped_rows = None))]
    const fn new(read_rows: u64, written_rows: u64, skipped_rows: Option<u64>) -> Self {
        Self::from_core(match skipped_rows {
            Some(skipped_rows) => IOResult {
                read_rows,
                written_rows,
                skipped_rows,
            },
            None => IOResult::new(read_rows, written_rows),
        })
    }

    /// The rows the write pulled from its source.
    #[getter]
    const fn read_rows(&self) -> u64 {
        self.inner.read_rows
    }

    /// The rows that reached the destination.
    #[getter]
    const fn written_rows(&self) -> u64 {
        self.inner.written_rows
    }

    /// The rows read and not written.
    #[getter]
    const fn skipped_rows(&self) -> u64 {
        self.inner.skipped_rows
    }

    /// Whether the write read no row at all: its source was empty.
    const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// A deterministic hash of the three counts.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// The two results summed count by count.
    fn __add__(&self, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        Ok(Py::new(py, Self::from_core(self.inner + other.inner))?.into_any())
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "IOResult(read_rows={}, written_rows={}, skipped_rows={})",
            self.inner.read_rows, self.inner.written_rows, self.inner.skipped_rows,
        )
    }

    fn __hash__(&self) -> isize {
        python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (u64, u64, u64)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (
                self.inner.read_rows,
                self.inner.written_rows,
                self.inner.skipped_rows,
            ),
        )
    }

    const fn __copy__(&self) -> Self {
        *self
    }

    const fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        *self
    }
}
