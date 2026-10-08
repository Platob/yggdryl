//! Native keyed items and their held or streamed collections.

use crate::expression::{PySelector, selector_from_value};
use crate::field::PyField;
use crate::scalar::PyScalar;
use crate::serie::{columnar, described};
use crate::stream_chunked_serie::PyStreamChunkedSerie;
use crate::stream_serie::PyStreamSerie;
use crate::text::line::PyFieldPath;
use crate::{normalize_index, value_error};
use pyo3::exceptions::{PyIndexError, PyTypeError};
use pyo3::prelude::*;
use pyo3::sync::MutexExt;
use pyo3::types::PyString;
use std::sync::Mutex;
use yggdryl::{
    Field, FieldPath, IntoKeyBy, KeyBy, KeySerie, KeySeries, PartitionOptions, StreamKeySerie,
};

/// Resolve selectors, native paths, and typed columnar external keys once.
pub(crate) fn key_by(value: &Bound<'_, PyAny>) -> PyResult<KeyBy> {
    if value.is_instance_of::<PyString>() || value.is_instance_of::<PySelector>() {
        return selector_from_value(value)?
            .into_key_by()
            .map_err(value_error);
    }
    if let Ok(path) = value.extract::<PyRef<'_, PyFieldPath>>() {
        return vec![path.inner.clone()].into_key_by().map_err(value_error);
    }
    if let Ok(paths) = value.extract::<Vec<PyRef<'_, PyFieldPath>>>() {
        return paths
            .into_iter()
            .map(|path| path.inner.clone())
            .collect::<Vec<_>>()
            .into_key_by()
            .map_err(value_error);
    }
    if let Some(columnar) = columnar(value)? {
        return crate::serie::native_columnar(columnar).map(KeyBy::External);
    }
    Err(PyTypeError::new_err(
        "expected selector text, a Selector, FieldPath values, or typed external keys; construct value keys with Serie.from_",
    ))
}

pub(crate) fn partition_options(
    max_open: Option<usize>,
    threads: Option<usize>,
    clustered: bool,
) -> PartitionOptions {
    let mut options = PartitionOptions::new().with_clustered(clustered);
    if let Some(value) = max_open {
        options = options.with_max_open(value);
    }
    if let Some(value) = threads {
        options = options.with_threads(value);
    }
    options
}

#[pyclass(
    name = "KeySerie",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyKeySerie {
    inner: Option<KeySerie>,
}

impl PyKeySerie {
    pub(crate) const fn from_inner(inner: KeySerie) -> Self {
        Self { inner: Some(inner) }
    }
    pub(crate) fn core(&self) -> &KeySerie {
        self.inner
            .as_ref()
            .expect("key owner remains present until drop")
    }
}

#[pyclass(
    name = "KeySeries",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyKeySeries {
    inner: Option<KeySeries>,
}

impl PyKeySeries {
    pub(crate) const fn from_inner(inner: KeySeries) -> Self {
        Self { inner: Some(inner) }
    }
    pub(crate) fn core(&self) -> &KeySeries {
        self.inner
            .as_ref()
            .expect("key owner remains present until drop")
    }
}

#[pyclass(name = "KeySeriesIterator", module = "yggdryl._native", frozen)]
struct PyKeySeriesIterator {
    items: Mutex<std::vec::IntoIter<KeySerie>>,
}

#[pymethods]
impl PyKeySeriesIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> Option<PyKeySerie> {
        self.items
            .lock_py_attached(py)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next()
            .map(PyKeySerie::from_inner)
    }
}
#[allow(clippy::wrong_self_convention)]
#[pymethods]
impl PyKeySerie {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.core().field().clone())
    }
    #[getter]
    fn key_field(&self) -> PyField {
        PyField::from_inner(self.core().key_field().clone())
    }
    #[getter]
    fn serie_field(&self) -> PyField {
        PyField::from_inner(self.core().serie_field().clone())
    }
    #[getter]
    fn key_paths(&self) -> Vec<Option<PyFieldPath>> {
        self.core()
            .key_paths()
            .iter()
            .cloned()
            .map(|path| path.map(PyFieldPath::from_core))
            .collect()
    }

    #[getter]
    fn key(&self) -> PyScalar {
        PyScalar::from_inner(self.core().key().clone())
    }
    #[getter]
    fn rownum(&self) -> Option<u64> {
        self.core().rownum()
    }
    #[getter]
    fn rows(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        described(py, self.core().rows().clone())
    }
    fn into_parts(&self, py: Python<'_>) -> PyResult<(PyScalar, Py<PyAny>)> {
        let (key, rows) = self.core().clone().into_parts();
        Ok((PyScalar::from_inner(key), described(py, rows)?))
    }

    fn memory_size(&self) -> usize {
        self.core().memory_size()
    }
    fn partition_by(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<PyKeySeries> {
        let by = key_by(by)?;
        let inner = self.core().clone();
        py.detach(move || inner.partition_by(by))
            .map(PyKeySeries::from_inner)
            .map_err(value_error)
    }
    #[pyo3(signature = (by, sorted = Some(false)))]
    fn window_by(
        &self,
        py: Python<'_>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<PyKeySeries> {
        let by = key_by(by)?;
        let inner = self.core().clone();
        py.detach(move || inner.window_by(by, sorted.unwrap_or(false)))
            .map(PyKeySeries::from_inner)
            .map_err(value_error)
    }
    fn into_stream(&self, py: Python<'_>) -> PyResult<PyStreamSerie> {
        let inner = self.core().clone();
        py.detach(move || inner.into_stream())
            .map(PyStreamSerie::from_core)
            .map_err(value_error)
    }
    #[pyo3(signature = (row_size = None, byte_size = None))]
    fn into_chunked_stream(
        &self,
        py: Python<'_>,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> PyResult<PyStreamChunkedSerie> {
        let inner = self.core().clone();
        py.detach(move || inner.into_chunked_stream(row_size, byte_size))
            .map(PyStreamChunkedSerie::from)
            .map_err(value_error)
    }
    fn __repr__(&self) -> String {
        format!("{:?}", self.core())
    }
}
#[allow(clippy::wrong_self_convention)]
#[pymethods]
impl PyKeySeries {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.core().field().clone())
    }
    #[getter]
    fn key_field(&self) -> PyField {
        PyField::from_inner(self.core().key_field().clone())
    }
    #[getter]
    fn serie_field(&self) -> PyField {
        PyField::from_inner(self.core().serie_field().clone())
    }
    #[getter]
    fn key_paths(&self) -> Vec<Option<PyFieldPath>> {
        self.core()
            .key_paths()
            .iter()
            .cloned()
            .map(|path| path.map(PyFieldPath::from_core))
            .collect()
    }

    fn __len__(&self) -> usize {
        self.core().len()
    }
    fn __getitem__(&self, index: isize) -> PyResult<PyKeySerie> {
        let index = normalize_index(index, self.core().len())
            .ok_or_else(|| PyIndexError::new_err(index))?;
        self.core()
            .get(index)
            .cloned()
            .map(PyKeySerie::from_inner)
            .ok_or_else(|| PyIndexError::new_err(index))
    }
    fn __iter__(&self) -> PyKeySeriesIterator {
        PyKeySeriesIterator {
            items: Mutex::new(self.core().clone().into_iter()),
        }
    }

    fn memory_size(&self) -> usize {
        self.core().memory_size()
    }
    fn partition_by(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<PyKeySeries> {
        let by = key_by(by)?;
        let inner = self.core().clone();
        py.detach(move || inner.partition_by(by))
            .map(PyKeySeries::from_inner)
            .map_err(value_error)
    }
    #[pyo3(signature = (by, sorted = Some(false)))]
    fn window_by(
        &self,
        py: Python<'_>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<PyKeySeries> {
        let by = key_by(by)?;
        let inner = self.core().clone();
        py.detach(move || inner.window_by(by, sorted.unwrap_or(false)))
            .map(PyKeySeries::from_inner)
            .map_err(value_error)
    }
    fn into_stream(&self, py: Python<'_>) -> PyResult<PyStreamSerie> {
        let inner = self.core().clone();
        py.detach(move || inner.into_stream())
            .map(PyStreamSerie::from_core)
            .map_err(value_error)
    }
    #[pyo3(signature = (row_size = None, byte_size = None))]
    fn into_chunked_stream(
        &self,
        py: Python<'_>,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> PyResult<PyStreamChunkedSerie> {
        let inner = self.core().clone();
        py.detach(move || inner.into_chunked_stream(row_size, byte_size))
            .map(PyStreamChunkedSerie::from)
            .map_err(value_error)
    }
    fn __repr__(&self) -> String {
        format!("{:?}", self.core())
    }
}

/// The layout remains readable after the core walk is consumed.
#[pyclass(name = "StreamKeySerie", module = "yggdryl._native", frozen)]
pub(crate) struct PyStreamKeySerie {
    inner: Mutex<Option<StreamKeySerie>>,
    field: Field,
    key_field: Field,
    serie_field: Field,
    key_paths: Vec<Option<FieldPath>>,
}

impl PyStreamKeySerie {
    pub(crate) fn from_core(inner: StreamKeySerie) -> Self {
        Self {
            field: inner.field().clone(),
            key_field: inner.key_field().clone(),
            serie_field: inner.serie_field().clone(),
            key_paths: inner.key_paths().to_vec(),
            inner: Mutex::new(Some(inner)),
        }
    }
    pub(crate) fn take(&self, py: Python<'_>) -> PyResult<StreamKeySerie> {
        self.inner
            .lock_py_attached(py)
            .map_err(|_| value_error("StreamKeySerie was poisoned"))?
            .take()
            .ok_or_else(|| value_error("StreamKeySerie was already consumed"))
    }
}

impl Drop for PyStreamKeySerie {
    fn drop(&mut self) {
        let inner = self
            .inner
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if inner.is_some() {
            Python::attach(|py| py.detach(move || drop(inner)));
        }
    }
}

#[allow(clippy::wrong_self_convention)]
#[pymethods]
impl PyStreamKeySerie {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.field.clone())
    }
    #[getter]
    fn key_field(&self) -> PyField {
        PyField::from_inner(self.key_field.clone())
    }
    #[getter]
    fn serie_field(&self) -> PyField {
        PyField::from_inner(self.serie_field.clone())
    }
    #[getter]
    fn key_paths(&self) -> Vec<Option<PyFieldPath>> {
        self.key_paths
            .iter()
            .cloned()
            .map(|path| path.map(PyFieldPath::from_core))
            .collect()
    }
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<PyKeySerie>> {
        let next = py.detach(|| {
            self.inner
                .lock()
                .map_err(|_| value_error("StreamKeySerie was poisoned"))?
                .as_mut()
                .and_then(Iterator::next)
                .transpose()
                .map_err(value_error)
        });
        next.map(|item| item.map(PyKeySerie::from_inner))
    }
    #[pyo3(signature = (by, sorted = Some(false)))]
    fn window_by(
        &self,
        py: Python<'_>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<Self> {
        let by = key_by(by)?;
        let inner = self.take(py)?;
        py.detach(move || inner.window_by(by, sorted.unwrap_or(false)))
            .map(Self::from_core)
            .map_err(value_error)
    }
    #[pyo3(signature = (by, max_open = None, threads = None, clustered = false))]
    fn partition_by(
        &self,
        py: Python<'_>,
        by: &Bound<'_, PyAny>,
        max_open: Option<usize>,
        threads: Option<usize>,
        clustered: bool,
    ) -> PyResult<Self> {
        let by = key_by(by)?;
        let options = partition_options(max_open, threads, clustered);
        let inner = self.take(py)?;
        py.detach(move || inner.partition_by(by, options))
            .map(Self::from_core)
            .map_err(value_error)
    }
    fn into_stream(&self, py: Python<'_>) -> PyResult<PyStreamSerie> {
        let inner = self.take(py)?;
        py.detach(move || inner.into_stream())
            .map(PyStreamSerie::from_core)
            .map_err(value_error)
    }
    #[pyo3(signature = (row_size = None, byte_size = None))]
    fn into_chunked_stream(
        &self,
        py: Python<'_>,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> PyResult<PyStreamChunkedSerie> {
        let inner = self.take(py)?;
        py.detach(move || inner.into_chunked_stream(row_size, byte_size))
            .map(PyStreamChunkedSerie::from)
            .map_err(value_error)
    }
    fn __repr__(&self) -> String {
        format!("StreamKeySerie(field={})", self.field)
    }
}

impl Drop for PyKeySerie {
    fn drop(&mut self) {
        let inner = self.inner.take();
        Python::attach(|py| py.detach(move || drop(inner)));
    }
}

impl Drop for PyKeySeries {
    fn drop(&mut self) {
        let inner = self.inner.take();
        Python::attach(|py| py.detach(move || drop(inner)));
    }
}
