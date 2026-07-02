//! The `yggdryl.core` submodule — thin wrappers over the `yggdryl-core` crate.
//!
//! `ByteBuffer` / `BitBuffer` expose the positioned byte- and bit-IO surface. Some
//! core conveniences are intentionally not surfaced here: the `pread_io` /
//! `pwrite_io` streams (they borrow two resources at once; a Python caller composes
//! the same effect from `pread_byte_array` + `pwrite_byte_array`) and the generic
//! owning adapters `RawIOCursor` / `IOCursor` (a moving cursor) and `RawIOSlice` /
//! `IOSlice` (a bounded byte window), which are Rust-core conveniences over the same
//! positioned surface.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3::wrap_pyfunction;
use yggdryl_core::RawIOBase;

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
    /// An empty buffer.
    #[new]
    fn new() -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::new(),
        }
    }

    /// A buffer over `data`.
    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::from_bytes(data),
        }
    }

    /// The buffer's bytes.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_bytes())
    }

    /// The buffer's size, in bytes.
    fn byte_size(&self) -> usize {
        self.inner.byte_size()
    }

    /// The buffer's size, in bits (eight times the byte size).
    fn bit_size(&self) -> usize {
        self.inner.bit_size()
    }

    /// The number of bytes the buffer can hold without reallocating.
    fn byte_capacity(&self) -> usize {
        self.inner.byte_capacity()
    }

    /// The number of bits the buffer can hold without reallocating.
    fn bit_capacity(&self) -> usize {
        self.inner.bit_capacity()
    }

    /// Request room for `capacity` bytes, returning the resulting capacity.
    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IoError> {
        Ok(self.inner.resize_byte_capacity(capacity)?)
    }

    /// Request room for `capacity` bits, returning the resulting bit capacity.
    fn resize_bit_capacity(&mut self, capacity: usize) -> Result<usize, IoError> {
        Ok(self.inner.resize_bit_capacity(capacity)?)
    }

    /// Set the buffer's size to `size` bytes, truncating or zero-filling.
    fn resize_bytes(&mut self, size: usize) -> Result<(), IoError> {
        Ok(self.inner.resize_bytes(size)?)
    }

    /// Set the buffer's size to `size` bits, rounded up to whole bytes.
    fn resize_bits(&mut self, size: usize) -> Result<(), IoError> {
        Ok(self.inner.resize_bits(size)?)
    }

    /// Read one byte.
    fn pread_byte_one(&self, position: usize, whence: Whence) -> Result<u8, IoError> {
        Ok(self.inner.pread_byte_one(position, whence.into())?)
    }

    /// Write one byte.
    fn pwrite_byte_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: u8,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_byte_one(position, whence.into(), value)?)
    }

    /// Read `size` bytes.
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

    /// Write bytes (an empty `bytes` is a no-op).
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

    /// Read one bit (MSB-first).
    fn pread_bit_one(&self, position: usize, whence: Whence) -> Result<bool, IoError> {
        Ok(self.inner.pread_bit_one(position, whence.into())?)
    }

    /// Write one bit (MSB-first).
    fn pwrite_bit_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: bool,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_bit_one(position, whence.into(), value)?)
    }

    /// Read `size` bits (MSB-first).
    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IoError> {
        Ok(self.inner.pread_bit_array(position, whence.into(), size)?)
    }

    /// Write bits (MSB-first; an empty list is a no-op).
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
    /// An empty buffer.
    #[new]
    fn new() -> Self {
        Self {
            inner: yggdryl_core::BitBuffer::new(),
        }
    }

    /// A buffer over `data` (a whole number of bytes).
    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            inner: yggdryl_core::BitBuffer::from_bytes(data),
        }
    }

    /// The buffer's backing bytes (trailing padding bits are always zero).
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_bytes())
    }

    /// The buffer's size, in bytes (rounded up).
    fn byte_size(&self) -> usize {
        self.inner.byte_size()
    }

    /// The buffer's exact size, in bits.
    fn bit_size(&self) -> usize {
        self.inner.bit_size()
    }

    /// The number of bytes the buffer can hold without reallocating.
    fn byte_capacity(&self) -> usize {
        self.inner.byte_capacity()
    }

    /// The number of bits the buffer can hold without reallocating.
    fn bit_capacity(&self) -> usize {
        self.inner.bit_capacity()
    }

    /// Request room for `capacity` bytes, returning the resulting capacity.
    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IoError> {
        Ok(self.inner.resize_byte_capacity(capacity)?)
    }

    /// Request room for `capacity` bits, returning the resulting bit capacity.
    fn resize_bit_capacity(&mut self, capacity: usize) -> Result<usize, IoError> {
        Ok(self.inner.resize_bit_capacity(capacity)?)
    }

    /// Set the buffer's size to `size` bytes, truncating or zero-filling.
    fn resize_bytes(&mut self, size: usize) -> Result<(), IoError> {
        Ok(self.inner.resize_bytes(size)?)
    }

    /// Set the buffer's size to an exact `size` bits.
    fn resize_bits(&mut self, size: usize) -> Result<(), IoError> {
        Ok(self.inner.resize_bits(size)?)
    }

    /// Read one byte.
    fn pread_byte_one(&self, position: usize, whence: Whence) -> Result<u8, IoError> {
        Ok(self.inner.pread_byte_one(position, whence.into())?)
    }

    /// Write one byte.
    fn pwrite_byte_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: u8,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_byte_one(position, whence.into(), value)?)
    }

    /// Read `size` bytes.
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

    /// Write bytes (an empty `bytes` is a no-op).
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

    /// Read one bit (MSB-first).
    fn pread_bit_one(&self, position: usize, whence: Whence) -> Result<bool, IoError> {
        Ok(self.inner.pread_bit_one(position, whence.into())?)
    }

    /// Write one bit (MSB-first).
    fn pwrite_bit_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: bool,
    ) -> Result<(), IoError> {
        Ok(self.inner.pwrite_bit_one(position, whence.into(), value)?)
    }

    /// Read `size` bits (MSB-first).
    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IoError> {
        Ok(self.inner.pread_bit_array(position, whence.into(), size)?)
    }

    /// Write bits (MSB-first; an empty list is a no-op).
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
