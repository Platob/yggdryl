//! The `yggdryl.buffer` submodule — typed native-type buffers.
//!
//! Exposes one immutable buffer class per native primitive
//! ([`I8Buffer`] … [`F64Buffer`]) plus the bit-packed [`BooleanBuffer`], mirroring
//! `yggdryl-buffer`. Each buffer carries optional headers (a `dict[bytes, bytes]`) and
//! hands out the matching [`yggdryl.field`](crate::field) class via `field(name,
//! nullable)`. The Arrow `from_arrow` / `to_arrow` interop is Rust-only (an
//! `arrow_buffer` value does not cross the FFI boundary), exactly as for
//! `yggdryl.io.ByteBuffer`.

// The `#[pymethods]` macro emits identity `.into()` conversions on `PyResult`
// returns that clippy flags as useless; silence it at module scope.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use yggdryl_http::{Headers, HeadersBased};

use crate::io::{
    ByteBuffer, ByteCursor, F32Cursor, F32Slice, F64Cursor, F64Slice, I16Cursor, I16Slice,
    I32Cursor, I32Slice, I64Cursor, I64Slice, I8Cursor, I8Slice, U16Cursor, U16Slice, U32Cursor,
    U32Slice, U64Cursor, U64Slice, U8Cursor, U8Slice,
};

/// Maps a core [`yggdryl_buffer::BufferError`] to a Python `ValueError`.
fn buffer_err(error: yggdryl_buffer::BufferError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Builds a Python `dict[bytes, bytes]` from a buffer's headers (or `None`).
fn headers_to_dict<'py>(py: Python<'py>, headers: Option<&Headers>) -> Option<Bound<'py, PyDict>> {
    headers.map(|meta| {
        let dict = PyDict::new_bound(py);
        for (key, value) in meta.pairs() {
            dict.set_item(PyBytes::new_bound(py, key), PyBytes::new_bound(py, value))
                .expect("inserting into a fresh dict cannot fail");
        }
        dict
    })
}

/// Generates the pyo3 wrapper class for one numeric buffer type.
macro_rules! py_primitive_buffer {
    ($( ($name:ident, $ty:ty, $cursor:ident, $slice:ident, $field:ident) ),+ $(,)?) => {
        $(
            #[doc = concat!("An immutable, cheaply-shared contiguous buffer of `", stringify!($ty), "` values.")]
            #[pyclass(module = "yggdryl.buffer")]
            #[derive(Clone)]
            pub struct $name {
                pub(crate) inner: yggdryl_buffer::$name,
            }

            #[pymethods]
            impl $name {
                /// Creates a buffer, optionally holding a copy of `values`.
                #[new]
                #[pyo3(signature = (values = None))]
                fn new(values: Option<Vec<$ty>>) -> Self {
                    let inner = match values {
                        Some(values) => yggdryl_buffer::$name::from_vec(values),
                        None => yggdryl_buffer::$name::new(),
                    };
                    Self { inner }
                }

                /// The number of values held.
                fn __len__(&self) -> usize {
                    self.inner.len()
                }

                /// The number of values held.
                fn len(&self) -> usize {
                    self.inner.len()
                }

                /// Whether the buffer holds no values.
                fn is_empty(&self) -> bool {
                    self.inner.is_empty()
                }

                /// The value at `index`, or `None` if out of bounds.
                fn get(&self, index: usize) -> Option<$ty> {
                    self.inner.get(index)
                }

                /// A `list` of the values.
                fn to_list(&self) -> Vec<$ty> {
                    self.inner.to_vec()
                }

                /// The values' little-endian bytes.
                fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, self.inner.as_bytes())
                }

                /// Serialises the values to their little-endian bytes.
                fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, &self.inner.serialize_bytes())
                }

                #[doc = concat!("Reconstructs a buffer from little-endian `", stringify!($ty), "` bytes.")]
                #[staticmethod]
                fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                    yggdryl_buffer::$name::deserialize_bytes(bytes)
                        .map(|inner| Self { inner })
                        .map_err(buffer_err)
                }

                /// Freezes the values into a `ByteBuffer` of their little-endian bytes.
                fn to_byte_buffer(&self) -> ByteBuffer {
                    ByteBuffer {
                        inner: self.inner.to_byte_buffer(),
                    }
                }

                /// Decodes a `ByteBuffer` of little-endian bytes into a buffer.
                #[staticmethod]
                fn from_byte_buffer(buffer: &ByteBuffer) -> PyResult<Self> {
                    yggdryl_buffer::$name::from_byte_buffer(&buffer.inner)
                        .map(|inner| Self { inner })
                        .map_err(buffer_err)
                }

                /// Opens a `ByteCursor` over the values' bytes.
                fn byte_cursor(&self) -> ByteCursor {
                    ByteCursor {
                        inner: self.inner.byte_cursor(),
                    }
                }

                #[doc = concat!("Opens a `", stringify!($cursor), "` over the values (native `", stringify!($ty), "` units).")]
                fn cursor(&self) -> $cursor {
                    $cursor {
                        inner: self.inner.cursor(),
                    }
                }

                #[doc = concat!("Opens a `", stringify!($slice), "` over the `offset..offset+len` window of values (in `", stringify!($ty), "` units).")]
                fn slice(&self, offset: usize, len: usize) -> $slice {
                    $slice {
                        inner: self.inner.slice(offset, len),
                    }
                }

                /// The buffer's headers as a `dict[bytes, bytes]`, or `None`.
                #[getter]
                fn headers<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyDict>> {
                    headers_to_dict(py, self.inner.headers())
                }

                /// Returns a copy of this buffer with `headers` (a `dict[bytes, bytes]`)
                /// attached — carried into the field this buffer hands out; it does not
                /// affect the buffer's byte-content equality.
                fn with_headers(&self, headers: HashMap<Vec<u8>, Vec<u8>>) -> Self {
                    Self {
                        inner: self.inner.clone().with_headers(Headers::from_pairs(headers)),
                    }
                }

                #[doc = concat!("Builds the matching `", stringify!($field), "` named `name`, carrying this buffer's headers.")]
                #[pyo3(signature = (name, nullable = false))]
                fn field(&self, name: String, nullable: bool) -> crate::field::$field {
                    crate::field::$field {
                        inner: self.inner.field(name, nullable),
                    }
                }

                fn __eq__(&self, other: &Self) -> bool {
                    self.inner == other.inner
                }

                fn __hash__(&self) -> u64 {
                    let mut hasher = DefaultHasher::new();
                    self.inner.hash(&mut hasher);
                    hasher.finish()
                }

                fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
                    let ctor = py
                        .get_type_bound::<$name>()
                        .getattr("deserialize_bytes")?
                        .unbind();
                    let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                        .into_any()
                        .unbind();
                    Ok((ctor, (state,)))
                }

                fn __repr__(&self) -> String {
                    format!(concat!(stringify!($name), "(len={})"), self.inner.len())
                }
            }
        )+
    };
}

py_primitive_buffer!(
    (I8Buffer, i8, I8Cursor, I8Slice, I8Field),
    (I16Buffer, i16, I16Cursor, I16Slice, I16Field),
    (I32Buffer, i32, I32Cursor, I32Slice, I32Field),
    (I64Buffer, i64, I64Cursor, I64Slice, I64Field),
    (U8Buffer, u8, U8Cursor, U8Slice, U8Field),
    (U16Buffer, u16, U16Cursor, U16Slice, U16Field),
    (U32Buffer, u32, U32Cursor, U32Slice, U32Field),
    (U64Buffer, u64, U64Cursor, U64Slice, U64Field),
    (F32Buffer, f32, F32Cursor, F32Slice, F32Field),
    (F64Buffer, f64, F64Cursor, F64Slice, F64Field),
);

/// An immutable, bit-packed (LSB-first) buffer of `bool` values.
#[pyclass(module = "yggdryl.buffer")]
#[derive(Clone)]
pub struct BooleanBuffer {
    pub(crate) inner: yggdryl_buffer::BooleanBuffer,
}

#[pymethods]
impl BooleanBuffer {
    /// Creates a buffer, optionally packing `values`.
    #[new]
    #[pyo3(signature = (values = None))]
    fn new(values: Option<Vec<bool>>) -> Self {
        let inner = match values {
            Some(values) => yggdryl_buffer::BooleanBuffer::from_bits(&values),
            None => yggdryl_buffer::BooleanBuffer::new(),
        };
        Self { inner }
    }

    /// Wraps `bytes` (LSB-first packed bits) as a buffer of `len` bits.
    #[staticmethod]
    fn from_bytes(bytes: &[u8], len: usize) -> PyResult<Self> {
        yggdryl_buffer::BooleanBuffer::from_bytes(bytes, len)
            .map(|inner| Self { inner })
            .map_err(buffer_err)
    }

    /// The number of bits held.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// The number of bits held.
    fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the buffer holds no bits.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The boolean at `index`, or `None` if out of bounds.
    fn get(&self, index: usize) -> Option<bool> {
        self.inner.get(index)
    }

    /// A `list` of the boolean values.
    fn to_list(&self) -> Vec<bool> {
        self.inner.to_vec()
    }

    /// The packed bytes (LSB-first).
    fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_bytes())
    }

    /// The number of set (`True`) bits.
    fn count_set_bits(&self) -> usize {
        self.inner.count_set_bits()
    }

    /// Serialises to an 8-byte little-endian bit length followed by the packed bytes.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a buffer from `serialize_bytes`.
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        yggdryl_buffer::BooleanBuffer::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(buffer_err)
    }

    /// Freezes the packed bytes into a `ByteBuffer` (the bit length is not carried).
    fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }

    /// Reads `len` packed bits from a `ByteBuffer`.
    #[staticmethod]
    fn from_byte_buffer(buffer: &ByteBuffer, len: usize) -> PyResult<Self> {
        yggdryl_buffer::BooleanBuffer::from_byte_buffer(&buffer.inner, len)
            .map(|inner| Self { inner })
            .map_err(buffer_err)
    }

    /// The buffer's headers as a `dict[bytes, bytes]`, or `None`.
    #[getter]
    fn headers<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyDict>> {
        headers_to_dict(py, self.inner.headers())
    }

    /// Returns a copy of this buffer with `headers` (a `dict[bytes, bytes]`) attached.
    fn with_headers(&self, headers: HashMap<Vec<u8>, Vec<u8>>) -> Self {
        Self {
            inner: self
                .inner
                .clone()
                .with_headers(Headers::from_pairs(headers)),
        }
    }

    /// Builds the matching `BooleanField` named `name`, carrying this buffer's headers.
    #[pyo3(signature = (name, nullable = false))]
    fn field(&self, name: String, nullable: bool) -> crate::field::BooleanField {
        crate::field::BooleanField {
            inner: self.inner.field(name, nullable),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<BooleanBuffer>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!("BooleanBuffer(len={})", self.inner.len())
    }
}

/// Populates the `buffer` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<I8Buffer>()?;
    module.add_class::<I16Buffer>()?;
    module.add_class::<I32Buffer>()?;
    module.add_class::<I64Buffer>()?;
    module.add_class::<U8Buffer>()?;
    module.add_class::<U16Buffer>()?;
    module.add_class::<U32Buffer>()?;
    module.add_class::<U64Buffer>()?;
    module.add_class::<F32Buffer>()?;
    module.add_class::<F64Buffer>()?;
    module.add_class::<BooleanBuffer>()?;
    Ok(())
}
