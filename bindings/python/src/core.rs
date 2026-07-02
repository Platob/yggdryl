//! The `yggdryl.core` submodule — thin wrappers over the `yggdryl-core` crate.
//!
//! `ByteBuffer` / `BitBuffer` expose the positioned byte- and bit-IO surface, and
//! `ByteBufferCursor` / `BitBufferCursor` (a moving cursor) and `ByteBufferSlice` /
//! `BitBufferSlice` (a bounded byte window) wrap the core `RawIOCursor` / `RawIOSlice`
//! adapters over a copy of a buffer's bytes. Only the `pread_io` / `pwrite_io`
//! streams stay Rust-only (they borrow two resources at once); a Python caller
//! composes the same effect from `pread_byte_array` + `pwrite_byte_array`.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use pyo3::wrap_pyfunction;
use yggdryl_core::{RawIOBase, RawIOCursor, RawIOSlice, Seekable};

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

/// Generates the shared `RawIOBase` surface for an adapter wrapper whose `inner`
/// field implements `RawIOBase` over a buffer reachable with `get_ref`, so the
/// delegating surface is written once. Per-type extras (a factory, the cursor's
/// `tell`/`seek`, the slice's `start`/`end`) are pasted into the same `#[pymethods]`
/// block (pyo3 wants one block per class without the `multiple-pymethods` feature).
macro_rules! raw_io_adapter_py {
    ($ty:ident, { $($extra:tt)* }) => {
        #[pymethods]
        impl $ty {
            /// The wrapped resource's bytes.
            fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, self.inner.get_ref().as_bytes())
            }

            /// The size, in bytes.
            fn byte_size(&self) -> usize {
                self.inner.byte_size()
            }

            /// The size, in bits.
            fn bit_size(&self) -> usize {
                self.inner.bit_size()
            }

            /// The number of bytes the resource can hold without reallocating.
            fn byte_capacity(&self) -> usize {
                self.inner.byte_capacity()
            }

            /// The number of bits the resource can hold without reallocating.
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

            /// Set the size to `size` bytes, truncating or zero-filling.
            fn resize_bytes(&mut self, size: usize) -> Result<(), IoError> {
                Ok(self.inner.resize_bytes(size)?)
            }

            /// Set the size to `size` bits.
            fn resize_bits(&mut self, size: usize) -> Result<(), IoError> {
                Ok(self.inner.resize_bits(size)?)
            }

            /// Read one byte.
            fn pread_byte_one(&self, position: usize, whence: Whence) -> Result<u8, IoError> {
                Ok(self.inner.pread_byte_one(position, whence.into())?)
            }

            /// Write one byte.
            fn pwrite_byte_one(&mut self, position: usize, whence: Whence, value: u8) -> Result<(), IoError> {
                Ok(self.inner.pwrite_byte_one(position, whence.into(), value)?)
            }

            /// Read `size` bytes.
            fn pread_byte_array<'py>(&self, py: Python<'py>, position: usize, whence: Whence, size: usize) -> Result<Bound<'py, PyBytes>, IoError> {
                let bytes = self.inner.pread_byte_array(position, whence.into(), size)?;
                Ok(PyBytes::new_bound(py, &bytes))
            }

            /// Write bytes (an empty `bytes` is a no-op).
            fn pwrite_byte_array(&mut self, position: usize, whence: Whence, values: Vec<u8>) -> Result<(), IoError> {
                Ok(self.inner.pwrite_byte_array(position, whence.into(), &values)?)
            }

            /// Read one bit (MSB-first).
            fn pread_bit_one(&self, position: usize, whence: Whence) -> Result<bool, IoError> {
                Ok(self.inner.pread_bit_one(position, whence.into())?)
            }

            /// Write one bit (MSB-first).
            fn pwrite_bit_one(&mut self, position: usize, whence: Whence, value: bool) -> Result<(), IoError> {
                Ok(self.inner.pwrite_bit_one(position, whence.into(), value)?)
            }

            /// Read `size` bits (MSB-first).
            fn pread_bit_array(&self, position: usize, whence: Whence, size: usize) -> Result<Vec<bool>, IoError> {
                Ok(self.inner.pread_bit_array(position, whence.into(), size)?)
            }

            /// Write bits (MSB-first; an empty list is a no-op).
            fn pwrite_bit_array(&mut self, position: usize, whence: Whence, values: Vec<bool>) -> Result<(), IoError> {
                Ok(self.inner.pwrite_bit_array(position, whence.into(), &values)?)
            }

            $($extra)*
        }
    };
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

    /// A moving cursor over a copy of this buffer's bytes.
    fn cursor(&self) -> ByteBufferCursor {
        ByteBufferCursor {
            inner: self.inner.clone().cursor(),
        }
    }

    /// A view bounded to the byte window `[start, end)` over a copy of this buffer.
    fn slice(&self, start: usize, end: usize) -> ByteBufferSlice {
        ByteBufferSlice {
            inner: self.inner.clone().slice(start, end),
        }
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

    /// A moving cursor over a copy of this buffer's bytes.
    fn cursor(&self) -> BitBufferCursor {
        BitBufferCursor {
            inner: self.inner.clone().cursor(),
        }
    }

    /// A view bounded to the byte window `[start, end)` over a copy of this buffer.
    fn slice(&self, start: usize, end: usize) -> BitBufferSlice {
        BitBufferSlice {
            inner: self.inner.clone().slice(start, end),
        }
    }
}

/// A moving cursor over a copy of a [`ByteBuffer`]'s bytes (a `RawIOCursor`): every
/// read and write advances its position, measured from `Whence.Current`.
#[pyclass]
pub struct ByteBufferCursor {
    inner: RawIOCursor<yggdryl_core::ByteBuffer>,
}

raw_io_adapter_py!(ByteBufferCursor, {
    /// A cursor over `data`.
    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            inner: RawIOCursor::new(yggdryl_core::ByteBuffer::from_bytes(data)),
        }
    }

    /// The cursor position, in bytes.
    fn tell(&self) -> usize {
        self.inner.tell()
    }

    /// Move the cursor, returning the new position.
    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IoError> {
        Ok(self.inner.seek(position, whence.into())?)
    }
});

/// A moving cursor over a copy of a [`BitBuffer`]'s bytes (a `RawIOCursor`).
#[pyclass]
pub struct BitBufferCursor {
    inner: RawIOCursor<yggdryl_core::BitBuffer>,
}

raw_io_adapter_py!(BitBufferCursor, {
    /// A cursor over `data`.
    #[staticmethod]
    fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            inner: RawIOCursor::new(yggdryl_core::BitBuffer::from_bytes(data)),
        }
    }

    /// The cursor position, in bytes.
    fn tell(&self) -> usize {
        self.inner.tell()
    }

    /// Move the cursor, returning the new position.
    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IoError> {
        Ok(self.inner.seek(position, whence.into())?)
    }
});

/// A view of a copy of a [`ByteBuffer`] bounded to the byte window `[start, end)` (a
/// `RawIOSlice`): access outside the window raises `ValueError`.
#[pyclass]
pub struct ByteBufferSlice {
    inner: RawIOSlice<yggdryl_core::ByteBuffer>,
}

raw_io_adapter_py!(ByteBufferSlice, {
    /// A window `[start, end)` over `data`.
    #[staticmethod]
    fn from_bytes(data: Vec<u8>, start: usize, end: usize) -> Self {
        Self {
            inner: RawIOSlice::new(yggdryl_core::ByteBuffer::from_bytes(data), start, end),
        }
    }

    /// The window's start byte offset.
    fn start(&self) -> usize {
        self.inner.start()
    }

    /// The window's end byte offset (exclusive).
    fn end(&self) -> usize {
        self.inner.end()
    }
});

/// A view of a copy of a [`BitBuffer`] bounded to the byte window `[start, end)` (a
/// `RawIOSlice`).
#[pyclass]
pub struct BitBufferSlice {
    inner: RawIOSlice<yggdryl_core::BitBuffer>,
}

raw_io_adapter_py!(BitBufferSlice, {
    /// A window `[start, end)` over `data`.
    #[staticmethod]
    fn from_bytes(data: Vec<u8>, start: usize, end: usize) -> Self {
        Self {
            inner: RawIOSlice::new(yggdryl_core::BitBuffer::from_bytes(data), start, end),
        }
    }

    /// The window's start byte offset.
    fn start(&self) -> usize {
        self.inner.start()
    }

    /// The window's end byte offset (exclusive).
    fn end(&self) -> usize {
        self.inner.end()
    }
});

/// Populates the `core` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version, module)?)?;
    module.add_function(wrap_pyfunction!(hello, module)?)?;
    module.add_class::<Whence>()?;
    module.add_class::<ByteBuffer>()?;
    module.add_class::<BitBuffer>()?;
    module.add_class::<ByteBufferCursor>()?;
    module.add_class::<BitBufferCursor>()?;
    module.add_class::<ByteBufferSlice>()?;
    module.add_class::<BitBufferSlice>()?;
    Ok(())
}
