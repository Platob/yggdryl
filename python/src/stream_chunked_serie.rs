//! Native chunk streams and the Arrow `RecordBatchReader` protocol.

use crate::chunked_serie::PyChunkedSerie;
use crate::field::{PyField, core_schema_to_pyarrow};
use crate::graph::ellipsis;
use crate::iomedia::{rooted_batch_to_pyarrow, rooted_reader_to_pyarrow};
use crate::join::{JoinKeywords, PyJoinOptions, join_keys_of, join_kind_of};
use crate::key_serie::{PyStreamKeySerie, key_by, partition_options};
use crate::serie::{
    PySerie, chunked_stream_from_py, described, field_of, orderings_of, reader_capsule,
    serie_source_of, sort_options, stream_of, target_of,
};
use crate::spill::{PySpillOptions, spill_options_of};
use crate::stream_serie::PyStreamSerie;
use crate::{cast_options, value_error};
use pyo3::exceptions::PyStopIteration;
use pyo3::prelude::*;
use pyo3::sync::MutexExt;
use std::sync::Mutex;
use yggdryl::{Field, StreamChunkedSerie};

#[pyclass(name = "StreamChunkedSerie", module = "yggdryl._native", frozen)]
pub(crate) struct PyStreamChunkedSerie {
    reader: Mutex<Option<StreamChunkedSerie>>,
    field: Field,
}

impl From<StreamChunkedSerie> for PyStreamChunkedSerie {
    fn from(reader: StreamChunkedSerie) -> Self {
        Self {
            field: reader.field().clone(),
            reader: Mutex::new(Some(reader)),
        }
    }
}

impl PyStreamChunkedSerie {
    pub(crate) fn take(&self, py: Python<'_>) -> PyResult<StreamChunkedSerie> {
        self.reader
            .lock_py_attached(py)
            .map_err(|_| value_error("StreamChunkedSerie was poisoned"))?
            .take()
            .ok_or_else(|| value_error("StreamChunkedSerie was already consumed"))
    }
    fn require_held(&self, py: Python<'_>) -> PyResult<()> {
        if self
            .reader
            .lock_py_attached(py)
            .map_err(|_| value_error("StreamChunkedSerie was poisoned"))?
            .is_some()
        {
            Ok(())
        } else {
            Err(value_error("StreamChunkedSerie was already consumed"))
        }
    }
    fn next_chunk(&self, py: Python<'_>) -> PyResult<Option<yggdryl::Serie>> {
        py.detach(|| {
            self.reader
                .lock()
                .map_err(|_| value_error("StreamChunkedSerie was poisoned"))?
                .as_mut()
                .and_then(StreamChunkedSerie::next_chunk)
                .transpose()
                .map_err(value_error)
        })
    }
}

impl Drop for PyStreamChunkedSerie {
    fn drop(&mut self) {
        let reader = self
            .reader
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if reader.is_some() {
            Python::attach(|py| py.detach(move || drop(reader)));
        }
    }
}

#[allow(clippy::wrong_self_convention)]
#[pymethods]
impl PyStreamChunkedSerie {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
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
        StreamChunkedSerie::from_arrow_reader(root.as_ref(), stream_of(reader)?, options)
            .map(Self::from)
            .map_err(value_error)
    }
    #[staticmethod]
    #[pyo3(name = "from_", signature = (value, root = None, *, safe = true, representation = "value"))]
    fn from_(
        value: &Bound<'_, PyAny>,
        root: Option<&Bound<'_, PyAny>>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Self> {
        chunked_stream_from_py(value, root, cast_options(safe, representation)?).map(Self::from)
    }
    #[staticmethod]
    fn from_serie(serie: &Bound<'_, PySerie>) -> PyResult<Self> {
        StreamChunkedSerie::from_serie(serie.try_borrow()?.inner.clone())
            .map(Self::from)
            .map_err(value_error)
    }
    #[staticmethod]
    fn from_chunked(chunked: &Bound<'_, PyChunkedSerie>) -> PyResult<Self> {
        StreamChunkedSerie::from_chunked(chunked.try_borrow()?.inner.clone())
            .map(Self::from)
            .map_err(value_error)
    }
    #[pyo3(signature = (field, *, safe = true, representation = "value"))]
    fn cast(
        &self,
        py: Python<'_>,
        field: &Bound<'_, PyAny>,
        safe: bool,
        representation: &str,
    ) -> PyResult<Self> {
        let options = cast_options(safe, representation)?;
        let field = target_of(field)?;
        let reader = self.take(py)?;
        py.detach(move || reader.cast(&field, options))
            .map(Self::from)
            .map_err(value_error)
    }
    fn resident_size(&self, py: Python<'_>) -> usize {
        self.reader
            .lock_py_attached(py)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map_or(0, StreamChunkedSerie::resident_size)
    }
    fn is_spilled(&self, py: Python<'_>) -> bool {
        self.reader
            .lock_py_attached(py)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(StreamChunkedSerie::is_spilled)
    }
    #[pyo3(signature = (options = None, *, byte_size = ellipsis(), folder = ellipsis()))]
    fn spill(
        &self,
        py: Python<'_>,
        options: Option<&PySpillOptions>,
        byte_size: Py<PyAny>,
        folder: Py<PyAny>,
    ) -> PyResult<()> {
        let options = spill_options_of(options, &byte_size.into_bound(py), &folder.into_bound(py))?;
        py.detach(|| {
            self.reader
                .lock()
                .map_err(|_| value_error("StreamChunkedSerie was poisoned"))?
                .as_mut()
                .ok_or_else(|| value_error("StreamChunkedSerie was already consumed"))?
                .spill(&options)
                .map_err(value_error)
        })
    }
    #[pyo3(signature = (options = None, *, byte_size = ellipsis(), folder = ellipsis()))]
    fn as_spilled<'py>(
        slf: &Bound<'py, Self>,
        options: Option<&PySpillOptions>,
        byte_size: Py<PyAny>,
        folder: Py<PyAny>,
    ) -> PyResult<Bound<'py, Self>> {
        slf.get().spill(slf.py(), options, byte_size, folder)?;
        Ok(slf.clone())
    }
    #[pyo3(signature = (options = None, *, byte_size = ellipsis(), folder = ellipsis()))]
    fn into_spilled(
        &self,
        py: Python<'_>,
        options: Option<&PySpillOptions>,
        byte_size: Py<PyAny>,
        folder: Py<PyAny>,
    ) -> PyResult<Self> {
        let options = spill_options_of(options, &byte_size.into_bound(py), &folder.into_bound(py))?;
        let reader = self.take(py)?;
        py.detach(move || reader.into_spilled(&options))
            .map(Self::from)
            .map_err(value_error)
    }
    #[pyo3(signature = (*, descending = false, nulls_first = false))]
    fn into_sorted(&self, py: Python<'_>, descending: bool, nulls_first: bool) -> PyResult<Self> {
        let options = sort_options(descending, nulls_first);
        let reader = self.take(py)?;
        py.detach(move || reader.into_sorted(options))
            .map(Self::from)
            .map_err(value_error)
    }
    fn into_sort_by(&self, py: Python<'_>, by: &Bound<'_, PyAny>) -> PyResult<Self> {
        let by = orderings_of(by)?;
        let reader = self.take(py)?;
        py.detach(move || reader.into_sort_by(by))
            .map(Self::from)
            .map_err(value_error)
    }
    #[pyo3(signature = (other, by, how = "inner", options = None, *, coalesce = ellipsis(), suffix = ellipsis(), build = ellipsis(), prune = ellipsis(), spill = ellipsis(), pushdown_keys = ellipsis()))]
    #[allow(clippy::too_many_arguments)]
    fn join_with(
        &self,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
        by: &Bound<'_, PyAny>,
        how: &str,
        options: Option<&PyJoinOptions>,
        coalesce: Py<PyAny>,
        suffix: Py<PyAny>,
        build: Py<PyAny>,
        prune: Py<PyAny>,
        spill: Py<PyAny>,
        pushdown_keys: Py<PyAny>,
    ) -> PyResult<Self> {
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
        .resolve(py, options)?;
        self.require_held(py)?;
        let other = serie_source_of(other)?;
        let reader = self.take(py)?;
        py.detach(move || reader.join_with(other, keys, how, &options))
            .map(Self::from)
            .map_err(value_error)
    }
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.field.clone())
    }
    #[getter]
    fn schema<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        core_schema_to_pyarrow(py, &self.field)
    }
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        self.next_chunk(py)?
            .map(|chunk| described(py, chunk))
            .transpose()
    }
    fn read_next_batch<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let chunk = self
            .next_chunk(py)?
            .ok_or_else(|| PyStopIteration::new_err(()))?;
        let batch = py
            .detach(move || chunk.into_arrow_batch())
            .map_err(value_error)?;
        rooted_batch_to_pyarrow(py, &self.field, batch)
    }
    fn read_all<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.into_arrow_reader(py)?.call_method0("read_all")
    }
    fn into_arrow_reader<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.take(py)?;
        rooted_reader_to_pyarrow(py, &self.field, reader.into_arrow_reader())
    }
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_stream__<'py>(
        &self,
        py: Python<'py>,
        requested_schema: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        reader_capsule(py, self.take(py)?, requested_schema)
    }
    fn into_stream(&self, py: Python<'_>) -> PyResult<PyStreamSerie> {
        let reader = self.take(py)?;
        py.detach(move || reader.into_stream())
            .map(PyStreamSerie::from_core)
            .map_err(value_error)
    }
    #[pyo3(signature = (row_size = None, byte_size = None))]
    fn into_chunked_stream(
        &self,
        py: Python<'_>,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> PyResult<Self> {
        let reader = self.take(py)?;
        py.detach(move || reader.into_chunked_stream(row_size, byte_size))
            .map(Self::from)
            .map_err(value_error)
    }
    #[pyo3(signature = (by, sorted = Some(false)))]
    fn window_by(
        &self,
        py: Python<'_>,
        by: &Bound<'_, PyAny>,
        sorted: Option<bool>,
    ) -> PyResult<PyStreamKeySerie> {
        let by = key_by(by)?;
        let reader = self.take(py)?;
        py.detach(move || reader.window_by(by, sorted.unwrap_or(false)))
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
        let reader = self.take(py)?;
        py.detach(move || reader.partition_by(by, options))
            .map(PyStreamKeySerie::from_core)
            .map_err(value_error)
    }
    fn __repr__(&self) -> String {
        format!("StreamChunkedSerie(field={})", self.field)
    }
}
