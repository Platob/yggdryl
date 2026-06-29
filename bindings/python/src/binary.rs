//! Python wrapper for the in-memory binary buffer [`yggdryl_core::Binary`].

use std::collections::BTreeMap;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use yggdryl_core::{
    Binary as CoreBinary, BinaryBased, BinaryType as CoreBinaryType, Io, Jsonable, Scalar,
};

use crate::{
    anyscalar_to_py, anytype_to_py, hash_of, py_bool, py_to_anytype, value_err, BinaryType, Whence,
};

/// A growable, in-memory binary buffer that also implements the IO surface
/// (`read`/`write`/`seek`/`pread`/`pwrite`/`resize`).
///
/// Cloning shares the allocation; `read`/`pread` hand back zero-copy `Binary`
/// views and writes copy-on-write, so views stay valid. Equality and hashing are
/// content-based (the cursor is not part of identity).
#[pyclass(module = "yggdryl", name = "Binary")]
#[derive(Clone)]
pub struct Binary {
    pub(crate) inner: CoreBinary,
}

#[pymethods]
impl Binary {
    #[new]
    #[pyo3(signature = (data = None, large = false))]
    fn new(data: Option<&[u8]>, large: bool) -> Self {
        let mut inner = match data {
            Some(bytes) => CoreBinary::from_bytes(bytes),
            None => CoreBinary::new(),
        };
        if large {
            inner = inner.with_data_type(CoreBinaryType::large());
        }
        Binary { inner }
    }

    /// The scalar's data type (a `BinaryType` object).
    #[getter]
    fn data_type(&self, py: Python<'_>) -> PyResult<PyObject> {
        anytype_to_py(py, &self.inner.data_type())
    }

    /// Returns a copy carrying the given `binary` type variant.
    fn with_data_type(&self, data_type: BinaryType) -> Self {
        Binary {
            inner: self.inner.with_data_type(data_type.inner),
        }
    }

    /// The buffer's raw bytes.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_bytes())
    }

    /// A `binary` buffer holding a copy of `data`.
    #[staticmethod]
    fn from_bytes(data: &[u8]) -> Self {
        Binary {
            inner: CoreBinary::from_bytes(data),
        }
    }

    /// The component map (`type`, plus `value` as hex).
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.inner.to_mapping()
    }

    /// Reconstructs a buffer from its component map.
    #[staticmethod]
    fn from_mapping(mapping: BTreeMap<String, String>) -> PyResult<Self> {
        CoreBinary::from_mapping(&mapping)
            .map(|inner| Binary { inner })
            .map_err(value_err)
    }

    /// The JSON form.
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs a buffer from its JSON form.
    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreBinary::from_json(value)
            .map(|inner| Binary { inner })
            .map_err(value_err)
    }

    /// The JSON bytes (JSON text encoded with the global charset).
    fn to_bson<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.to_bson())
    }

    /// Reconstructs a buffer from its JSON bytes.
    #[staticmethod]
    fn from_bson(data: &[u8]) -> PyResult<Self> {
        CoreBinary::from_bson(data)
            .map(|inner| Binary { inner })
            .map_err(value_err)
    }

    /// Casts the value to `data_type` (a `BinaryType` or `Utf8Type`), returning a
    /// new `Binary` or `Utf8` (binary → string fails on non-UTF-8 bytes).
    fn cast(&self, py: Python<'_>, data_type: &Bound<'_, PyAny>) -> PyResult<PyObject> {
        let data_type = py_to_anytype(data_type)?;
        let scalar = self.inner.cast(&data_type).map_err(value_err)?;
        anyscalar_to_py(py, scalar)
    }

    /// Sets the data type in place (same-family only); use `cast` to convert.
    fn set_data_type(&mut self, data_type: &Bound<'_, PyAny>) -> PyResult<()> {
        let data_type = py_to_anytype(data_type)?;
        self.inner.set_data_type(&data_type).map_err(value_err)
    }

    // --- IO surface ---

    /// The number of valid bytes.
    #[getter]
    fn size(&self) -> u64 {
        self.inner.size()
    }

    /// The allocated capacity in bytes.
    #[getter]
    fn capacity(&self) -> u64 {
        self.inner.capacity()
    }

    /// The current cursor position.
    fn tell(&self) -> u64 {
        self.inner.tell()
    }

    /// Moves the cursor; returns the new position.
    #[pyo3(signature = (offset, whence = Whence::Start))]
    fn seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
        self.inner.seek(offset, whence.into()).map_err(value_err)
    }

    /// Positional read of up to `length` bytes at `offset` (a zero-copy view).
    fn pread(&self, offset: u64, length: usize) -> PyResult<Self> {
        self.inner
            .pread(offset, length)
            .map(|inner| Binary { inner })
            .map_err(value_err)
    }

    /// Cursor read of up to `length` bytes; advances the cursor.
    fn read(&mut self, length: usize) -> PyResult<Self> {
        self.inner
            .read(length)
            .map(|inner| Binary { inner })
            .map_err(value_err)
    }

    /// Positional write at `offset`, growing the buffer if needed.
    fn pwrite(&mut self, offset: u64, data: &[u8]) -> PyResult<usize> {
        self.inner.pwrite(offset, data).map_err(value_err)
    }

    /// Cursor write; advances the cursor.
    fn write(&mut self, data: &[u8]) -> PyResult<usize> {
        self.inner.write(data).map_err(value_err)
    }

    /// Sets the allocated capacity.
    fn set_capacity(&mut self, capacity: u64) -> PyResult<()> {
        self.inner.set_capacity(capacity).map_err(value_err)
    }

    /// Resizes the logical length, filling new bytes with `fill`.
    #[pyo3(signature = (new_size, fill = 0))]
    fn resize(&mut self, new_size: u64, fill: u8) -> PyResult<()> {
        self.inner.resize(new_size, fill).map_err(value_err)
    }

    // --- dunders ---

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice())
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<Binary>()
            .is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "Binary({:?}, large={})",
            self.inner.as_slice(),
            py_bool(self.inner.binary_type().is_large()),
        )
    }

    fn __getnewargs__<'py>(&self, py: Python<'py>) -> (Bound<'py, PyBytes>, bool) {
        (
            PyBytes::new_bound(py, self.inner.as_slice()),
            self.inner.binary_type().is_large(),
        )
    }
}
