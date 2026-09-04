//! Python views over the core Avro schema and `Scalar` codecs.
//!
//! This boundary owns no Avro model: schemas, containers, resolution, and
//! single-object framing all remain native Rust values. Python values enter
//! and leave through the same shared `Scalar` conversion used by the structured
//! codecs and record adapters.

use std::sync::Arc;

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyByteArray, PyBytes, PyDict, PyList};
use yggdryl::holder::Buffer;
use yggdryl::media::avro::{
    Block as CoreAvroBlock, Blocks as CoreAvroBlocks, Container, Resolution, Schema,
};
use yggdryl::text::Limits;

use crate::record::string_pairs_from_value;
use crate::scalar::{PyScalar, as_py, from_py};
use crate::value_error;

/// A parsed native Apache Avro schema.
#[derive(Clone)]
#[pyclass(
    name = "AvroSchema",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyAvroSchema {
    pub(crate) inner: Schema,
}

impl PyAvroSchema {
    const fn from_inner(inner: Schema) -> Self {
        Self { inner }
    }
}

#[pymethods]
#[allow(clippy::wrong_self_convention)] // Python `into_*` methods do not consume wrappers.
impl PyAvroSchema {
    /// Parse an Avro schema from a native Scalar, a Python natural value, JSON
    /// UTF-8, or JSON bytes.
    #[new]
    #[pyo3(signature = (value, *, max_depth = None, max_input_bytes = None, max_nodes = None))]
    fn new(
        value: &Bound<'_, PyAny>,
        max_depth: Option<usize>,
        max_input_bytes: Option<usize>,
        max_nodes: Option<usize>,
    ) -> PyResult<Self> {
        schema_from_value(value, decode_limits(max_depth, max_input_bytes, max_nodes))
            .map(Self::from_inner)
    }

    /// Parse an Avro schema from any accepted schema representation.
    #[classmethod]
    #[pyo3(signature = (value, *, max_depth = None, max_input_bytes = None, max_nodes = None))]
    fn from_value(
        _class: &Bound<'_, pyo3::types::PyType>,
        value: &Bound<'_, PyAny>,
        max_depth: Option<usize>,
        max_input_bytes: Option<usize>,
        max_nodes: Option<usize>,
    ) -> PyResult<Self> {
        Self::new(value, max_depth, max_input_bytes, max_nodes)
    }

    /// Return the native schema's JSON representation as natural Python data.
    fn into_json(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        as_py(py, &self.inner.clone().into_json())
    }

    /// Return the Avro Parsing Canonical Form.
    fn into_canonical_form(&self) -> String {
        self.inner.clone().into_canonical_form()
    }

    /// Return the CRC-64-AVRO fingerprint of the canonical form.
    fn fingerprint(&self) -> u64 {
        self.inner.fingerprint()
    }

    /// Return the schema root kind.
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    fn __str__(&self) -> String {
        self.inner.clone().into_canonical_form()
    }

    /// Return a deterministic hash of the complete retained schema document.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __repr__(&self) -> PyResult<String> {
        let document =
            yggdryl::json::into_utf8(&self.inner.clone().into_json()).map_err(value_error)?;
        Ok(format!("AvroSchema({document:?})"))
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let document =
            yggdryl::json::into_utf8(&self.inner.clone().into_json()).map_err(value_error)?;
        Ok((py.get_type::<Self>().into_any().unbind(), (document,)))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One decoded native Avro object container.
#[pyclass(
    name = "AvroContainer",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyAvroContainer {
    inner: Container,
}

impl PyAvroContainer {
    fn pickle_state<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let state = PyDict::new(py);
        state.set_item("schema", as_py(py, &self.inner.schema.clone().into_json())?)?;
        let metadata = self
            .inner
            .metadata
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<Vec<_>>();
        state.set_item("metadata", metadata)?;
        let rows = self
            .inner
            .rows
            .iter()
            .map(|row| crate::scalar::scalar_pickle_state(py, row))
            .collect::<PyResult<Vec<_>>>()?;
        state.set_item("rows", PyList::new(py, rows)?)?;
        Ok(state)
    }
}

#[pymethods]
impl PyAvroContainer {
    /// Rebuild the complete decoded container value for pickle.
    #[staticmethod]
    fn _from_pickle(state: &Bound<'_, PyDict>) -> PyResult<Self> {
        let schema = state
            .get_item("schema")?
            .ok_or_else(|| PyTypeError::new_err("AvroContainer pickle state needs schema"))?;
        let metadata = state
            .get_item("metadata")?
            .ok_or_else(|| PyTypeError::new_err("AvroContainer pickle state needs metadata"))?
            .extract::<Vec<(String, String)>>()?
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        let rows = state
            .get_item("rows")?
            .ok_or_else(|| PyTypeError::new_err("AvroContainer pickle state needs rows"))?
            .try_iter()?
            .map(|row| crate::scalar::scalar_from_pickle_state(&row?, 0))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self {
            inner: Container {
                schema: schema_from_value(&schema, Limits::default())?,
                metadata,
                rows,
            },
        })
    }

    /// The writer schema recorded in the object-container header.
    #[getter]
    fn schema(&self) -> PyAvroSchema {
        PyAvroSchema::from_inner(self.inner.schema.clone())
    }

    /// User metadata from the header, excluding Avro's reserved entries.
    #[getter]
    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = PyDict::new(py);
        for (key, value) in &self.inner.metadata {
            result.set_item(key.as_str(), value.as_str())?;
        }
        Ok(result)
    }

    /// Return one header metadata value without copying the whole mapping.
    fn get(&self, key: &str) -> Option<&str> {
        self.inner.get(key)
    }

    /// Every row decoded through the shared native Scalar conversion.
    #[getter]
    fn rows<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let rows = self
            .inner
            .rows
            .iter()
            .map(|row| as_py(py, row))
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, rows)
    }

    fn __len__(&self) -> usize {
        self.inner.rows.len()
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let state = self.pickle_state(py)?;
        Ok(format!(
            "AvroContainer._from_pickle({})",
            state.repr()?.to_str()?
        ))
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (self.pickle_state(py)?.into_any().unbind(),),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// One compressed Avro container block, decoded only when rows are requested.
#[pyclass(
    name = "AvroBlock",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyAvroBlock {
    inner: CoreAvroBlock,
    resolution: Option<Arc<Resolution>>,
}

#[pymethods]
impl PyAvroBlock {
    // A block carries lazy resolution context and is not a canonical value.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Rows declared by the block header.
    #[getter]
    fn count(&self) -> u64 {
        self.inner.count()
    }

    /// Compressed payload bytes held by this block.
    #[getter]
    fn size(&self) -> usize {
        self.inner.size()
    }

    /// Decode this block, applying the iterator's reader schema when supplied.
    fn rows<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let values = py
            .detach(|| match self.resolution.as_deref() {
                Some(resolution) => self.inner.rows_resolved(resolution),
                None => self.inner.rows(),
            })
            .map_err(value_error)?;
        let rows = values
            .iter()
            .map(|value| as_py(py, value))
            .collect::<PyResult<Vec<_>>>()?;
        PyList::new(py, rows)
    }

    fn __repr__(&self) -> String {
        format!(
            "AvroBlock(count={}, size={})",
            self.inner.count(),
            self.inner.size()
        )
    }
}

/// A fused, lazy Python iterator over compressed Avro container blocks.
#[pyclass(
    name = "AvroBlockIterator",
    module = "yggdryl._native",
    skip_from_py_object
)]
pub(crate) struct PyAvroBlockIterator {
    inner: CoreAvroBlocks<'static, Buffer>,
    resolution: Option<Arc<Resolution>>,
    finished: bool,
}

#[pymethods]
impl PyAvroBlockIterator {
    // Iterator identity changes as it is consumed.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The writer schema read from the container header.
    #[getter]
    fn schema(&self) -> PyAvroSchema {
        PyAvroSchema::from_inner(self.inner.schema().clone())
    }

    /// Header metadata, excluding Avro's reserved entries.
    #[getter]
    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = PyDict::new(py);
        for (key, value) in self.inner.metadata() {
            result.set_item(key.as_str(), value.as_str())?;
        }
        Ok(result)
    }

    /// Return one header metadata value without copying the whole mapping.
    fn get(&self, key: &str) -> Option<&str> {
        self.inner.get(key)
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<Option<PyAvroBlock>> {
        if self.finished {
            return Ok(None);
        }
        match self.inner.next_block() {
            Ok(Some(inner)) => Ok(Some(PyAvroBlock {
                inner,
                resolution: self.resolution.clone(),
            })),
            Ok(None) => {
                self.finished = true;
                Ok(None)
            }
            Err(error) => {
                self.finished = true;
                Err(value_error(error))
            }
        }
    }
}

/// Decode a self-describing Avro object container from bytes.
#[pyfunction]
#[pyo3(
    name = "avro_loads",
    signature = (
        data,
        *,
        reader_schema = None,
        max_depth = None,
        max_input_bytes = None,
        max_nodes = None
    )
)]
pub(crate) fn avro_loads(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    reader_schema: Option<&PyAvroSchema>,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
) -> PyResult<PyAvroContainer> {
    let bytes = bytes_from_value(data)?;
    let reader_schema = reader_schema.map(|schema| schema.inner.clone());
    let limits = decode_limits(max_depth, max_input_bytes, max_nodes);
    let container = py
        .detach(|| {
            let source = Buffer::from_bytes(bytes);
            match reader_schema.as_ref() {
                Some(reader) => yggdryl::media::avro::read_container_resolved_with_limits(
                    &source, reader, limits,
                ),
                None => yggdryl::media::avro::read_container_with_limits(&source, limits),
            }
        })
        .map_err(value_error)?;
    Ok(PyAvroContainer { inner: container })
}

/// Open a lazy iterator over the compressed blocks of an object container.
#[pyfunction]
#[pyo3(
    name = "avro_blocks",
    signature = (
        data,
        *,
        reader_schema = None,
        max_depth = None,
        max_input_bytes = None,
        max_nodes = None
    )
)]
pub(crate) fn avro_blocks(
    data: &Bound<'_, PyAny>,
    reader_schema: Option<&PyAvroSchema>,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
) -> PyResult<PyAvroBlockIterator> {
    let source = Buffer::from_bytes(bytes_from_value(data)?);
    let limits = decode_limits(max_depth, max_input_bytes, max_nodes);
    let inner =
        yggdryl::media::avro::read_blocks_owned_with_limits(source, limits).map_err(value_error)?;
    let resolution = reader_schema
        .map(|reader| Resolution::from_schemas(inner.schema(), &reader.inner))
        .transpose()
        .map_err(value_error)?
        .map(Arc::new);
    Ok(PyAvroBlockIterator {
        inner,
        resolution,
        finished: false,
    })
}

/// Encode rows as a self-describing Avro object container.
#[pyfunction]
#[pyo3(name = "avro_dumps", signature = (rows, schema, *, metadata = None))]
pub(crate) fn avro_dumps<'py>(
    py: Python<'py>,
    rows: &Bound<'_, PyAny>,
    schema: &Bound<'_, PyAny>,
    metadata: Option<&Bound<'_, PyAny>>,
) -> PyResult<Bound<'py, PyBytes>> {
    let schema = schema_from_value(schema, Limits::default())?;
    let schema_json = schema.into_json();
    let rows = values_from_iterable(rows, "Avro rows must be an iterable of values")?;
    let metadata = metadata
        .map(string_pairs_from_value)
        .transpose()?
        .unwrap_or_default();
    let borrowed = metadata
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let encoded = py
        .detach(|| {
            let mut target = Buffer::new();
            yggdryl::media::avro::write_container(&mut target, &schema_json, &borrowed, &rows)?;
            Ok::<_, yggdryl::Error>(target.into_bytes())
        })
        .map_err(value_error)?;
    Ok(PyBytes::new(py, &encoded))
}

/// Decode one datum in Avro single-object framing.
#[pyfunction]
#[pyo3(
    name = "avro_loads_single",
    signature = (
        data,
        schema,
        *,
        max_depth = None,
        max_input_bytes = None,
        max_nodes = None
    )
)]
pub(crate) fn avro_loads_single(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    schema: &Bound<'_, PyAny>,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let bytes = bytes_from_value(data)?;
    let limits = decode_limits(max_depth, max_input_bytes, max_nodes);
    let schema = schema_from_value(schema, limits)?;
    let value = py
        .detach(|| {
            yggdryl::media::avro::from_single_object_slice_with_limits(&bytes, &schema, limits)
        })
        .map_err(value_error)?;
    as_py(py, &value)
}

/// Encode one value in Avro single-object framing.
#[pyfunction]
#[pyo3(name = "avro_dumps_single")]
pub(crate) fn avro_dumps_single<'py>(
    py: Python<'py>,
    value: &Bound<'_, PyAny>,
    schema: &Bound<'_, PyAny>,
) -> PyResult<Bound<'py, PyBytes>> {
    let value = from_py(value)?;
    let schema = schema_from_value(schema, Limits::default())?;
    let encoded = py
        .detach(|| yggdryl::media::avro::into_single_object_vec(&schema, &value))
        .map_err(value_error)?;
    Ok(PyBytes::new(py, &encoded))
}

/// Read any accepted Python schema representation into the one core model.
fn schema_from_value(value: &Bound<'_, PyAny>, limits: Limits) -> PyResult<Schema> {
    if let Ok(schema) = value.extract::<PyRef<'_, PyAvroSchema>>() {
        return Ok(schema.inner.clone());
    }
    if let Ok(native) = value.extract::<PyRef<'_, PyScalar>>() {
        return Schema::from_json_with_limits(&native.inner, limits).map_err(value_error);
    }
    if let Ok(text) = value.extract::<&str>() {
        let document = yggdryl::json::from_utf8_with_limits(text, limits).map_err(value_error)?;
        return Schema::from_json_with_limits(&document, limits).map_err(value_error);
    }
    if value.is_instance_of::<PyBytes>()
        || value.is_instance_of::<PyByteArray>()
        || value.hasattr("tobytes")?
    {
        let document = yggdryl::json::from_bytes_with_limits(&bytes_from_value(value)?, limits)
            .map_err(value_error)?;
        return Schema::from_json_with_limits(&document, limits).map_err(value_error);
    }
    Schema::from_json_with_limits(&from_py(value)?, limits).map_err(value_error)
}

/// Resolve optional binding values onto the one core decoding budget.
fn decode_limits(
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
) -> Limits {
    let defaults = Limits::default();
    Limits::new(
        max_depth.unwrap_or_else(|| defaults.max_depth()),
        max_input_bytes.unwrap_or_else(|| defaults.max_input_bytes()),
        max_nodes.unwrap_or_else(|| defaults.max_nodes()),
        defaults.max_documents(),
    )
}

/// Copy one Python bytes-like value at the boundary.
fn bytes_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(bytes.as_bytes().to_vec());
    }
    if let Ok(bytes) = value.cast::<PyByteArray>() {
        return Ok(bytes.to_vec());
    }
    if value.hasattr("tobytes")? {
        let owned = value.call_method0("tobytes")?;
        if let Ok(bytes) = owned.cast::<PyBytes>() {
            return Ok(bytes.as_bytes().to_vec());
        }
    }
    Err(PyTypeError::new_err(
        "Avro input must be bytes, bytearray, or memoryview",
    ))
}

/// Convert a Python iterable to native Scalars exactly once.
fn values_from_iterable(
    value: &Bound<'_, PyAny>,
    message: &'static str,
) -> PyResult<Vec<yggdryl::Scalar>> {
    let iterator = value
        .try_iter()
        .map_err(|_| PyTypeError::new_err(message))?;
    iterator
        .map(|item| item.and_then(|item| from_py(&item)))
        .collect()
}
