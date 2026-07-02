//! The `yggdryl.core` submodule — thin wrappers over the `yggdryl-core` crate.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3::wrap_pyfunction;
use yggdryl_core::{RawIOBase, Seekable};

/// The `yggdryl-core` version string.
#[pyfunction]
fn version() -> &'static str {
    yggdryl_core::version()
}

/// Prints a greeting to standard output — the minimal cross-language example.
#[pyfunction]
fn hello() {
    yggdryl_core::hello()
}

/// Wraps an [`yggdryl_core::IOError`] so pyo3 raises it as a Python `ValueError`.
struct IoError(yggdryl_core::IOError);

impl From<yggdryl_core::IOError> for IoError {
    fn from(error: yggdryl_core::IOError) -> Self {
        IoError(error)
    }
}

impl From<IoError> for PyErr {
    fn from(error: IoError) -> Self {
        PyValueError::new_err(error.0.to_string())
    }
}

/// The reference point a position is measured from.
#[pyclass(eq, eq_int)]
#[derive(Clone, PartialEq)]
pub enum Whence {
    Start,
    Current,
    End,
}

impl From<Whence> for yggdryl_core::Whence {
    fn from(whence: Whence) -> Self {
        match whence {
            Whence::Start => yggdryl_core::Whence::Start,
            Whence::Current => yggdryl_core::Whence::Current,
            Whence::End => yggdryl_core::Whence::End,
        }
    }
}

/// A growable, byte-granular in-memory buffer.
#[pyclass]
#[derive(Default)]
pub struct ByteBuffer {
    inner: yggdryl_core::ByteBuffer,
}

#[pymethods]
impl ByteBuffer {
    #[new]
    fn new() -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::new(),
        }
    }

    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::from_bytes(data),
        }
    }

    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_bytes())
    }

    fn byte_size(&self) -> usize {
        self.inner.byte_size()
    }

    fn bit_size(&self) -> usize {
        self.inner.bit_size()
    }

    fn byte_capacity(&self) -> usize {
        self.inner.byte_capacity()
    }

    fn bit_capacity(&self) -> usize {
        self.inner.bit_capacity()
    }

    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IoError> {
        Ok(self.inner.resize_byte_capacity(capacity)?)
    }

    fn resize_bit_capacity(&mut self, capacity: usize) -> Result<usize, IoError> {
        Ok(self.inner.resize_bit_capacity(capacity)?)
    }

    fn resize_bytes(&mut self, size: usize) -> Result<(), IoError> {
        Ok(self.inner.resize_bytes(size)?)
    }

    fn resize_bits(&mut self, size: usize) -> Result<(), IoError> {
        Ok(self.inner.resize_bits(size)?)
    }

    fn tell(&self) -> usize {
        self.inner.tell()
    }

    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IoError> {
        Ok(self.inner.seek(position, whence.into())?)
    }

    fn pread_byte_one(&self, position: usize, whence: Whence) -> Result<u8, IoError> {
        Ok(self.inner.pread_byte_one(position, whence.into())?)
    }

    fn pwrite_byte_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: u8,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_byte_one(position, whence.into(), value)?)
    }

    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Bound<'py, PyBytes>, IoError> {
        let bytes = self.inner.pread_byte_array(position, whence.into(), size)?;
        Ok(PyBytes::new_bound(py, &bytes))
    }

    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: Vec<u8>,
    ) -> Result<(), IoError> {
        Ok(self
            .inner
            .pwrite_byte_array(position, whence.into(), &values)?)
    }

    fn pread_bit_one(&self, position: usize, whence: Whence) -> Result<bool, IoError> {
        Ok(self.inner.pread_bit_one(position, whence.into())?)
    }

    fn pwrite_bit_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: bool,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_bit_one(position, whence.into(), value)?)
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IoError> {
        Ok(self.inner.pread_bit_array(position, whence.into(), size)?)
    }

    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: Vec<bool>,
    ) -> Result<(), IoError> {
        Ok(self
            .inner
            .pwrite_bit_array(position, whence.into(), &values)?)
    }

    /// Stream `size` bytes from this buffer into `sink`, copying in chunks.
    #[allow(clippy::too_many_arguments)]
    fn pread_io(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
        sink: &mut ByteBuffer,
        sink_position: usize,
        sink_whence: Whence,
    ) -> Result<(), IoError> {
        Ok(self.inner.pread_io(
            position,
            whence.into(),
            size,
            &mut sink.inner,
            sink_position,
            sink_whence.into(),
        )?)
    }

    /// Stream `size` bytes from `source` into this buffer, copying in chunks.
    #[allow(clippy::too_many_arguments)]
    fn pwrite_io(
        &mut self,
        position: usize,
        whence: Whence,
        source: &ByteBuffer,
        source_position: usize,
        source_whence: Whence,
        size: usize,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_io(
            position,
            whence.into(),
            &source.inner,
            source_position,
            source_whence.into(),
            size,
        )?)
    }
}

/// A growable, bit-granular in-memory buffer (its bit size need not be a multiple of
/// eight).
#[pyclass]
#[derive(Default)]
pub struct BitBuffer {
    inner: yggdryl_core::BitBuffer,
}

#[pymethods]
impl BitBuffer {
    #[new]
    fn new() -> Self {
        Self {
            inner: yggdryl_core::BitBuffer::new(),
        }
    }

    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_core::BitBuffer::from_bytes(data),
        }
    }

    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_bytes())
    }

    fn byte_size(&self) -> usize {
        self.inner.byte_size()
    }

    fn bit_size(&self) -> usize {
        self.inner.bit_size()
    }

    fn byte_capacity(&self) -> usize {
        self.inner.byte_capacity()
    }

    fn bit_capacity(&self) -> usize {
        self.inner.bit_capacity()
    }

    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IoError> {
        Ok(self.inner.resize_byte_capacity(capacity)?)
    }

    fn resize_bit_capacity(&mut self, capacity: usize) -> Result<usize, IoError> {
        Ok(self.inner.resize_bit_capacity(capacity)?)
    }

    fn resize_bytes(&mut self, size: usize) -> Result<(), IoError> {
        Ok(self.inner.resize_bytes(size)?)
    }

    fn resize_bits(&mut self, size: usize) -> Result<(), IoError> {
        Ok(self.inner.resize_bits(size)?)
    }

    fn tell(&self) -> usize {
        self.inner.tell()
    }

    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IoError> {
        Ok(self.inner.seek(position, whence.into())?)
    }

    fn pread_byte_one(&self, position: usize, whence: Whence) -> Result<u8, IoError> {
        Ok(self.inner.pread_byte_one(position, whence.into())?)
    }

    fn pwrite_byte_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: u8,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_byte_one(position, whence.into(), value)?)
    }

    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Bound<'py, PyBytes>, IoError> {
        let bytes = self.inner.pread_byte_array(position, whence.into(), size)?;
        Ok(PyBytes::new_bound(py, &bytes))
    }

    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: Vec<u8>,
    ) -> Result<(), IoError> {
        Ok(self
            .inner
            .pwrite_byte_array(position, whence.into(), &values)?)
    }

    fn pread_bit_one(&self, position: usize, whence: Whence) -> Result<bool, IoError> {
        Ok(self.inner.pread_bit_one(position, whence.into())?)
    }

    fn pwrite_bit_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: bool,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_bit_one(position, whence.into(), value)?)
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IoError> {
        Ok(self.inner.pread_bit_array(position, whence.into(), size)?)
    }

    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: Vec<bool>,
    ) -> Result<(), IoError> {
        Ok(self
            .inner
            .pwrite_bit_array(position, whence.into(), &values)?)
    }

    /// Stream `size` bytes from this buffer into `sink`, copying in chunks.
    #[allow(clippy::too_many_arguments)]
    fn pread_io(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
        sink: &mut BitBuffer,
        sink_position: usize,
        sink_whence: Whence,
    ) -> Result<(), IoError> {
        Ok(self.inner.pread_io(
            position,
            whence.into(),
            size,
            &mut sink.inner,
            sink_position,
            sink_whence.into(),
        )?)
    }

    /// Stream `size` bytes from `source` into this buffer, copying in chunks.
    #[allow(clippy::too_many_arguments)]
    fn pwrite_io(
        &mut self,
        position: usize,
        whence: Whence,
        source: &BitBuffer,
        source_position: usize,
        source_whence: Whence,
        size: usize,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_io(
            position,
            whence.into(),
            &source.inner,
            source_position,
            source_whence.into(),
            size,
        )?)
    }
}

/// Populates the `core` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version, module)?)?;
    module.add_function(wrap_pyfunction!(hello, module)?)?;
    module.add_class::<Whence>()?;
    module.add_class::<ByteBuffer>()?;
    module.add_class::<BitBuffer>()?;
    Ok(())
}
