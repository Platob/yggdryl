//! Native scalar row streams, consumed exactly once.

use crate::field::{PyField, core_field_from_value};
use crate::iomedia::{batch_reader_from_arrow_reader, rooted_reader_to_pyarrow};
use crate::key_serie::{PyStreamKeySerie, key_by, partition_options};
use crate::scalar::{PyScalar, from_py_under};
use crate::serie::{reader_capsule, serie_source_of};
use crate::stream_chunked_serie::PyStreamChunkedSerie;
use crate::value_error;
use pyo3::prelude::*;
use pyo3::sync::MutexExt;
use std::sync::Mutex;
use yggdryl::{Field, StreamSerie};

#[pyclass(name = "StreamSerie", module = "yggdryl._native", frozen)]
pub(crate) struct PyStreamSerie {
    field: Field,
    rows: Mutex<Option<StreamSerie>>,
}

impl PyStreamSerie {
    pub(crate) fn from_core(rows: StreamSerie) -> Self {
        Self {
            field: rows.field().clone(),
            rows: Mutex::new(Some(rows)),
        }
    }
    pub(crate) fn take(&self, py: Python<'_>) -> PyResult<StreamSerie> {
        self.rows
            .lock_py_attached(py)
            .map_err(|_| value_error("StreamSerie was poisoned"))?
            .take()
            .ok_or_else(|| value_error("StreamSerie was already consumed"))
    }
}

impl Drop for PyStreamSerie {
    fn drop(&mut self) {
        let rows = self
            .rows
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if rows.is_some() {
            Python::attach(|py| py.detach(move || drop(rows)));
        }
    }
}

#[allow(clippy::wrong_self_convention)]
#[pymethods]
impl PyStreamSerie {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.field.clone())
    }
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<PyScalar>> {
        py.detach(|| {
            self.rows
                .lock()
                .map_err(|_| value_error("StreamSerie was poisoned"))?
                .as_mut()
                .and_then(Iterator::next)
                .transpose()
                .map_err(value_error)
        })
        .map(|row| row.map(PyScalar::from_inner))
    }
    fn collect(&self, py: Python<'_>) -> PyResult<Vec<PyScalar>> {
        let rows = self.take(py)?;
        py.detach(move || rows.collect_rows())
            .map(|rows| rows.into_iter().map(PyScalar::from_inner).collect())
            .map_err(value_error)
    }
    #[staticmethod]
    fn from_arrow_reader(reader: &Bound<'_, PyAny>) -> PyResult<Self> {
        let reader = batch_reader_from_arrow_reader(reader)?;
        StreamSerie::from_arrow_reader(reader)
            .map(Self::from_core)
            .map_err(value_error)
    }
    #[staticmethod]
    #[pyo3(name = "from_")]
    fn from_(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let source = serie_source_of(value)?;
        value
            .py()
            .detach(move || source.into_stream())
            .map(Self::from_core)
            .map_err(value_error)
    }
    #[staticmethod]
    fn from_rows(field: &Bound<'_, PyAny>, rows: &Bound<'_, PyAny>) -> PyResult<Self> {
        let field = core_field_from_value(field)?;
        let iterator = rows.try_iter()?.unbind();
        let root = field.clone();
        let rows = std::iter::from_fn(move || {
            Python::attach(|py| {
                iterator.bind(py).clone().next().map(|row| {
                    row.and_then(|row| from_py_under(&root, &row))
                        .map_err(|error| yggdryl::Error::InvalidRecord {
                            path: root.name().into(),
                            reason: error.to_string().into(),
                        })
                })
            })
        });
        Ok(Self::from_core(StreamSerie::from_rows(field, rows)))
    }
    fn into_stream(&self, py: Python<'_>) -> PyResult<Self> {
        self.take(py).map(Self::from_core)
    }
    #[pyo3(signature = (row_size = None, byte_size = None))]
    fn into_chunked_stream(
        &self,
        py: Python<'_>,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> PyResult<PyStreamChunkedSerie> {
        let rows = self.take(py)?;
        py.detach(move || rows.into_chunked_stream(row_size, byte_size))
            .map(PyStreamChunkedSerie::from)
            .map_err(value_error)
    }
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let rows = self.take(py)?;
        let reader = py
            .detach(move || rows.into_arrow_reader())
            .map_err(value_error)?;
        rooted_reader_to_pyarrow(py, &self.field, reader)
    }
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_stream__<'py>(
        &self,
        py: Python<'py>,
        requested_schema: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let rows = self.take(py)?;
        let reader = py
            .detach(move || rows.into_chunked_stream(None, None))
            .map_err(value_error)?;
        reader_capsule(py, reader, requested_schema)
    }
    #[pyo3(signature = (by, sorted = Some(false)))]
    fn window_by(
        &self,
        py: Python<'_>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<PyStreamKeySerie> {
        let by = key_by(by)?;
        let rows = self.take(py)?;
        py.detach(move || rows.window_by(by, sorted.unwrap_or(false)))
            .map(PyStreamKeySerie::from_core)
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
    ) -> PyResult<PyStreamKeySerie> {
        let by = key_by(by)?;
        let options = partition_options(max_open, threads, clustered);
        let rows = self.take(py)?;
        py.detach(move || rows.partition_by(by, options))
            .map(PyStreamKeySerie::from_core)
            .map_err(value_error)
    }
    fn __repr__(&self) -> String {
        format!("StreamSerie(field={})", self.field)
    }
}
