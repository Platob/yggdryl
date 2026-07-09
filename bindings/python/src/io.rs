//! The `yggdryl.io` submodule — cursor-oriented byte IO.
//!
//! Exposes the [`Whence`] seek origin, [`ByteBuffer`] (storage), the byte
//! [`ByteCursor`] (the positioned reader/writer, `std::io::Cursor`-style), and the
//! element-typed cursors [`I8Cursor`] … [`F64Cursor`] — one concrete class per
//! primitive, mirroring the core `TypedCursor<T>` (`tell` / `seek` count in `T`
//! units; `byte_*` / `bit_*` reach the byte and bit positions) — and the wide-integer
//! cursors [`I96Cursor`] / [`I128Cursor`] / [`I256Cursor`], whose values marshal as
//! arbitrary-precision Python `int`. A cursor's `byte_size` / `size` report the bytes
//! / elements **remaining** from the current position. The generic `IOBase` /
//! `TypedIOBase` / `IOCursor` traits themselves are Rust-only.

// The `#[pymethods]` macro emits identity `.into()` conversions on `PyResult`
// returns that clippy flags as useless; silence it at module scope.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict};

use yggdryl_core::{IOBase, IOCursor, IOSlice, IoPrimitive, TypedIOBase};

/// Marshals a wide integer to a Python `int` via `int.from_bytes` (signed, little-
/// endian) — abi3-safe and uniform for `i96` / `i128` / `i256`, none of which map to
/// a fixed-width Python scalar.
fn wide_to_py<T: IoPrimitive>(py: Python<'_>, value: T) -> PyResult<PyObject> {
    let bytes = PyBytes::new_bound(py, &value.to_le_vec());
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("signed", true)?;
    let int = py.import_bound("builtins")?.getattr("int")?;
    Ok(int
        .call_method("from_bytes", (bytes, "little"), Some(&kwargs))?
        .unbind())
}

/// Marshals a Python `int` to a wide integer via `int.to_bytes` (signed, little-
/// endian); an out-of-range value raises `OverflowError` from Python, so the range
/// is checked for free.
fn wide_from_py<T: IoPrimitive>(obj: &Bound<'_, PyAny>) -> PyResult<T> {
    let kwargs = PyDict::new_bound(obj.py());
    kwargs.set_item("signed", true)?;
    let bytes: Vec<u8> = obj
        .call_method("to_bytes", (T::WIDTH, "little"), Some(&kwargs))?
        .extract()?;
    Ok(T::from_le_slice(&bytes))
}

/// Maps a core [`yggdryl_core::IoError`] to a Python `ValueError`.
fn io_err(error: yggdryl_core::IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The seek origin a position is measured from — `SEEK_SET` / `SEEK_CUR` /
/// `SEEK_END`.
#[pyclass(module = "yggdryl.io", eq, eq_int, frozen, hash)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Whence {
    /// From the start of the resource.
    Start,
    /// From the current cursor position.
    Current,
    /// From the end of the resource.
    End,
}

impl From<Whence> for yggdryl_core::Whence {
    fn from(whence: Whence) -> Self {
        match whence {
            Whence::Start => Self::Start,
            Whence::Current => Self::Current,
            Whence::End => Self::End,
        }
    }
}

/// An immutable byte store — pure storage; positioned IO is done via a
/// [`ByteCursor`] from [`byte_cursor`](ByteBuffer::byte_cursor).
#[pyclass(module = "yggdryl.io")]
#[derive(Clone)]
pub struct ByteBuffer {
    pub(crate) inner: yggdryl_core::ByteBuffer,
}

#[pymethods]
impl ByteBuffer {
    /// Creates a buffer, optionally holding a copy of `data`.
    #[new]
    #[pyo3(signature = (data = None))]
    fn new(data: Option<&[u8]>) -> Self {
        let inner = match data {
            Some(bytes) => yggdryl_core::ByteBuffer::from_bytes(bytes),
            None => yggdryl_core::ByteBuffer::new(),
        };
        Self { inner }
    }

    /// Creates an empty buffer preallocated for `capacity` bytes.
    #[staticmethod]
    fn with_byte_capacity(capacity: usize) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::with_byte_capacity(capacity),
        }
    }

    /// Creates an empty buffer preallocated for `capacity` bits.
    #[staticmethod]
    fn with_bit_capacity(capacity: usize) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::with_bit_capacity(capacity),
        }
    }

    /// The number of bytes held.
    fn byte_size(&self) -> usize {
        self.inner.byte_size()
    }

    /// The number of bits held.
    fn bit_size(&self) -> usize {
        self.inner.bit_size()
    }

    /// Whether the buffer holds no bytes.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The number of bytes that can be held without reallocating.
    fn byte_capacity(&self) -> usize {
        self.inner.byte_capacity()
    }

    /// The number of bits that can be held without reallocating.
    fn bit_capacity(&self) -> usize {
        self.inner.bit_capacity()
    }

    /// A copy of the backing bytes.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_bytes())
    }

    /// Serialises the buffer to its byte content.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a buffer from its byte content.
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::deserialize_bytes(bytes),
        }
    }

    /// Opens a [`ByteCursor`] over this buffer (the buffer stays intact).
    fn byte_cursor(&self) -> ByteCursor {
        ByteCursor {
            inner: self.inner.byte_cursor(),
        }
    }

    /// Opens a [`ByteSlice`] over the byte window `[offset, offset + len)` (clamped).
    fn byte_slice(&self, offset: u64, len: usize) -> ByteSlice {
        ByteSlice {
            inner: self.inner.byte_slice(offset, len),
        }
    }

    fn __len__(&self) -> usize {
        self.inner.byte_size()
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
            .get_type_bound::<ByteBuffer>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!("ByteBuffer(byte_size={})", self.inner.byte_size())
    }
}

/// A positioned, advancing cursor over a [`ByteBuffer`]. Reads and writes happen at
/// the cursor and advance it; a write copies the shared bytes out first.
#[pyclass(module = "yggdryl.io")]
#[derive(Clone)]
pub struct ByteCursor {
    pub(crate) inner: yggdryl_core::ByteCursor,
}

#[pymethods]
impl ByteCursor {
    /// The current position, in bytes from the start (the byte cursor's native
    /// unit; `bit_tell` gives it in bits).
    fn tell(&self) -> PyResult<u64> {
        self.inner.byte_tell().map_err(io_err)
    }

    /// Moves the cursor to `offset` bytes relative to `whence`, returning the new
    /// position. A negative `offset` seeks backward.
    #[pyo3(signature = (offset, whence = Whence::Start))]
    fn seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
        self.inner.byte_seek(offset, whence.into()).map_err(io_err)
    }

    /// The current position, in bits from the start (`tell * 8`).
    fn bit_tell(&self) -> PyResult<u64> {
        self.inner.bit_tell().map_err(io_err)
    }

    /// Moves the cursor to `offset` bits relative to `whence`, returning the new bit
    /// position. The resolved bit position must be byte-aligned (a multiple of 8).
    #[pyo3(signature = (offset, whence = Whence::Start))]
    fn bit_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
        self.inner.bit_seek(offset, whence.into()).map_err(io_err)
    }

    /// The current position (mirror of `tell`, infallible).
    fn position(&self) -> u64 {
        self.inner.position()
    }

    /// Sets the current position.
    fn set_position(&mut self, position: u64) {
        self.inner.set_position(position);
    }

    /// The number of bytes the resource holds.
    fn byte_size(&self) -> PyResult<usize> {
        self.inner.byte_size().map_err(io_err)
    }

    /// The number of bits the resource holds.
    fn bit_size(&self) -> PyResult<usize> {
        self.inner.bit_size().map_err(io_err)
    }

    /// The number of bytes as a 64-bit value.
    fn large_byte_size(&self) -> PyResult<u64> {
        self.inner.large_byte_size().map_err(io_err)
    }

    /// The number of bits as a 64-bit value.
    fn large_bit_size(&self) -> PyResult<u64> {
        self.inner.large_bit_size().map_err(io_err)
    }

    /// The number of bytes that can be held without reallocating.
    fn byte_capacity(&self) -> PyResult<usize> {
        self.inner.byte_capacity().map_err(io_err)
    }

    /// The number of bits that can be held without reallocating.
    fn bit_capacity(&self) -> PyResult<usize> {
        self.inner.bit_capacity().map_err(io_err)
    }

    /// The number of `u8` values held (equals `byte_size`).
    fn size(&self) -> PyResult<usize> {
        TypedIOBase::<u8>::size(&self.inner).map_err(io_err)
    }

    /// The `u8` capacity (equals `byte_capacity`).
    fn capacity(&self) -> PyResult<usize> {
        TypedIOBase::<u8>::capacity(&self.inner).map_err(io_err)
    }

    /// The default `u8` value used to fill a gap opened past the end on a grow (`0`).
    fn default_value(&self) -> u8 {
        TypedIOBase::<u8>::default_value(&self.inner)
    }

    /// The little-endian bytes of `count` default values — the gap-fill pattern
    /// (`b"\x00" * count` for the byte cursor).
    fn default_byte_array<'py>(&self, py: Python<'py>, count: usize) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(
            py,
            &TypedIOBase::<u8>::default_byte_array(&self.inner, count),
        )
    }

    /// Reads up to `size` bytes at `whence`, advancing the cursor.
    #[pyo3(signature = (size, whence = Whence::Start))]
    fn pread_byte_array<'py>(
        &mut self,
        py: Python<'py>,
        size: usize,
        whence: Whence,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = self
            .inner
            .pread_byte_array(size, whence.into())
            .map_err(io_err)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Writes `data` at `whence`, advancing the cursor.
    #[pyo3(signature = (data, whence = Whence::Start))]
    fn pwrite_byte_array(&mut self, data: &[u8], whence: Whence) -> PyResult<usize> {
        self.inner
            .pwrite_byte_array(data, whence.into())
            .map_err(io_err)
    }

    /// Reads up to `len(buf)` bytes at `whence` **into** the writable `buf` (e.g. a
    /// `bytearray`), advancing the cursor, and returns the number read. Reuse `buf`
    /// to read with **zero per-call allocation** — no `bytes` object crosses the FFI.
    #[pyo3(signature = (buf, whence = Whence::Start))]
    fn pread_into(&mut self, buf: &Bound<'_, PyByteArray>, whence: Whence) -> PyResult<usize> {
        // SAFETY: the GIL is held and we do not run arbitrary Python while the
        // mutable borrow is live, so the bytearray cannot be resized underneath us.
        let slice = unsafe { buf.as_bytes_mut() };
        self.inner.pread_into(slice, whence.into()).map_err(io_err)
    }

    /// Reads a single byte at `whence`, advancing the cursor.
    #[pyo3(signature = (whence = Whence::Start))]
    fn pread_one(&mut self, whence: Whence) -> PyResult<u8> {
        TypedIOBase::<u8>::pread_one(&mut self.inner, whence.into()).map_err(io_err)
    }

    /// Writes a single byte at `whence`, advancing the cursor.
    #[pyo3(signature = (value, whence = Whence::Start))]
    fn pwrite_one(&mut self, value: u8, whence: Whence) -> PyResult<usize> {
        TypedIOBase::<u8>::pwrite_one(&mut self.inner, value, whence.into()).map_err(io_err)
    }

    /// Copies up to `size` bytes from this cursor into `sink`, advancing both.
    #[pyo3(signature = (sink, size, whence = Whence::Start))]
    fn pread_io(&mut self, sink: &mut ByteCursor, size: usize, whence: Whence) -> PyResult<u64> {
        self.inner
            .pread_io(&mut sink.inner, size, whence.into())
            .map_err(io_err)
    }

    /// Copies up to `size` bytes from `source` into this cursor, advancing both.
    #[pyo3(signature = (source, size, whence = Whence::Start))]
    fn pwrite_io(&mut self, source: &mut ByteCursor, size: usize, whence: Whence) -> PyResult<u64> {
        self.inner
            .pwrite_io(&mut source.inner, size, whence.into())
            .map_err(io_err)
    }

    /// The cursor's current bytes, including any writes.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_bytes())
    }

    /// Freezes the cursor's bytes into a new [`ByteBuffer`].
    fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }

    fn __repr__(&self) -> String {
        format!("ByteCursor(position={})", self.inner.position())
    }
}

/// Generates the pyo3 wrappers for one primitive's typed cursor accessors into their
/// own `#[pymethods]` block (macros are not allowed inside a single block).
macro_rules! py_primitive_io {
    ($( ($ty:ty, $read_one:ident, $write_one:ident, $read_arr:ident, $write_arr:ident) ),+ $(,)?) => {
        #[pymethods]
        impl ByteCursor {
            $(
                #[doc = concat!("Reads a little-endian `", stringify!($ty), "` at `whence`, advancing.")]
                #[pyo3(signature = (whence = Whence::Start))]
                fn $read_one(&mut self, whence: Whence) -> PyResult<$ty> {
                    self.inner.$read_one(whence.into()).map_err(io_err)
                }

                #[doc = concat!("Writes a little-endian `", stringify!($ty), "` at `whence`, advancing.")]
                #[pyo3(signature = (value, whence = Whence::Start))]
                fn $write_one(&mut self, value: $ty, whence: Whence) -> PyResult<usize> {
                    self.inner.$write_one(value, whence.into()).map_err(io_err)
                }

                #[doc = concat!("Reads up to `count` little-endian `", stringify!($ty), "` values at `whence`.")]
                #[pyo3(signature = (count, whence = Whence::Start))]
                fn $read_arr(&mut self, count: usize, whence: Whence) -> PyResult<Vec<$ty>> {
                    self.inner.$read_arr(count, whence.into()).map_err(io_err)
                }

                #[doc = concat!("Writes the little-endian `", stringify!($ty), "` values in `data` at `whence`.")]
                #[pyo3(signature = (data, whence = Whence::Start))]
                fn $write_arr(&mut self, data: Vec<$ty>, whence: Whence) -> PyResult<usize> {
                    self.inner.$write_arr(&data, whence.into()).map_err(io_err)
                }
            )+
        }
    };
}

py_primitive_io!(
    (i8, pread_i8, pwrite_i8, pread_i8_array, pwrite_i8_array),
    (u8, pread_u8, pwrite_u8, pread_u8_array, pwrite_u8_array),
    (
        i16,
        pread_i16,
        pwrite_i16,
        pread_i16_array,
        pwrite_i16_array
    ),
    (
        u16,
        pread_u16,
        pwrite_u16,
        pread_u16_array,
        pwrite_u16_array
    ),
    (
        i32,
        pread_i32,
        pwrite_i32,
        pread_i32_array,
        pwrite_i32_array
    ),
    (
        u32,
        pread_u32,
        pwrite_u32,
        pread_u32_array,
        pwrite_u32_array
    ),
    (
        i64,
        pread_i64,
        pwrite_i64,
        pread_i64_array,
        pwrite_i64_array
    ),
    (
        u64,
        pread_u64,
        pwrite_u64,
        pread_u64_array,
        pwrite_u64_array
    ),
    (
        f32,
        pread_f32,
        pwrite_f32,
        pread_f32_array,
        pwrite_f32_array
    ),
    (
        f64,
        pread_f64,
        pwrite_f64,
        pread_f64_array,
        pwrite_f64_array
    ),
);

/// Generates one element-typed cursor class (`yggdryl_core::TypedCursor<$ty>`) whose
/// native unit is `$ty` — `tell` / `seek` count in `$ty` values, while `byte_*` /
/// `bit_*` reach the underlying byte and bit positions. Mirrors the core
/// `TypedCursor<T>`, one concrete class per primitive (the byte / `u8` case is
/// `ByteCursor`).
macro_rules! py_typed_cursor {
    ($( ($name:ident, $ty:ty) ),+ $(,)?) => {
        $(
            #[doc = concat!("A positioned, advancing cursor whose native unit is a `", stringify!($ty), "` value.")]
            #[pyclass(module = "yggdryl.io")]
            #[derive(Clone)]
            pub struct $name {
                pub(crate) inner: yggdryl_core::TypedCursor<$ty>,
            }

            #[pymethods]
            impl $name {
                #[doc = concat!("Creates an empty cursor preallocated for `capacity` `", stringify!($ty), "` values.")]
                #[staticmethod]
                fn with_capacity(capacity: usize) -> Self {
                    Self {
                        inner: <yggdryl_core::TypedCursor<$ty> as TypedIOBase<$ty>>::with_capacity(capacity),
                    }
                }

                #[doc = concat!("The current position, in `", stringify!($ty), "` values from the start.")]
                fn tell(&self) -> PyResult<u64> {
                    TypedIOBase::<$ty>::tell(&self.inner).map_err(io_err)
                }

                #[doc = concat!("Moves the cursor to `offset` `", stringify!($ty), "` values relative to `whence`. A negative `offset` seeks backward.")]
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    TypedIOBase::<$ty>::seek(&mut self.inner, offset, whence.into()).map_err(io_err)
                }

                /// The current position, in bytes from the start.
                fn byte_tell(&self) -> PyResult<u64> {
                    self.inner.byte_tell().map_err(io_err)
                }

                /// Moves the cursor to `offset` bytes relative to `whence`.
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn byte_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    self.inner.byte_seek(offset, whence.into()).map_err(io_err)
                }

                /// The current position, in bits from the start (`byte_tell * 8`).
                fn bit_tell(&self) -> PyResult<u64> {
                    self.inner.bit_tell().map_err(io_err)
                }

                /// Moves the cursor to `offset` bits relative to `whence`; the resolved
                /// bit position must be byte-aligned (a multiple of 8).
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn bit_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    self.inner.bit_seek(offset, whence.into()).map_err(io_err)
                }

                /// The current position in bytes (mirror of `byte_tell`, infallible).
                fn position(&self) -> u64 {
                    self.inner.position()
                }

                /// Sets the current byte position.
                fn set_position(&mut self, position: u64) {
                    self.inner.set_position(position);
                }

                #[doc = concat!("The number of `", stringify!($ty), "` values held.")]
                fn size(&self) -> PyResult<usize> {
                    TypedIOBase::<$ty>::size(&self.inner).map_err(io_err)
                }

                #[doc = concat!("The `", stringify!($ty), "` capacity without reallocating.")]
                fn capacity(&self) -> PyResult<usize> {
                    TypedIOBase::<$ty>::capacity(&self.inner).map_err(io_err)
                }

                /// The number of bytes the resource holds.
                fn byte_size(&self) -> PyResult<usize> {
                    self.inner.byte_size().map_err(io_err)
                }

                /// The number of bits the resource holds.
                fn bit_size(&self) -> PyResult<usize> {
                    self.inner.bit_size().map_err(io_err)
                }

                /// The number of bytes that can be held without reallocating.
                fn byte_capacity(&self) -> PyResult<usize> {
                    self.inner.byte_capacity().map_err(io_err)
                }

                /// The number of bits that can be held without reallocating.
                fn bit_capacity(&self) -> PyResult<usize> {
                    self.inner.bit_capacity().map_err(io_err)
                }

                #[doc = concat!("Reads a single `", stringify!($ty), "` at `whence`, advancing the cursor.")]
                #[pyo3(signature = (whence = Whence::Start))]
                fn pread_one(&mut self, whence: Whence) -> PyResult<$ty> {
                    TypedIOBase::<$ty>::pread_one(&mut self.inner, whence.into()).map_err(io_err)
                }

                #[doc = concat!("Writes a single `", stringify!($ty), "` at `whence`, advancing the cursor.")]
                #[pyo3(signature = (value, whence = Whence::Start))]
                fn pwrite_one(&mut self, value: $ty, whence: Whence) -> PyResult<usize> {
                    TypedIOBase::<$ty>::pwrite_one(&mut self.inner, value, whence.into()).map_err(io_err)
                }

                #[doc = concat!("Reads up to `count` `", stringify!($ty), "` values at `whence`, advancing the cursor.")]
                #[pyo3(signature = (count, whence = Whence::Start))]
                fn pread_array(&mut self, count: usize, whence: Whence) -> PyResult<Vec<$ty>> {
                    TypedIOBase::<$ty>::pread_array(&mut self.inner, count, whence.into()).map_err(io_err)
                }

                #[doc = concat!("Writes the `", stringify!($ty), "` values in `data` at `whence`, advancing the cursor.")]
                #[pyo3(signature = (data, whence = Whence::Start))]
                fn pwrite_array(&mut self, data: Vec<$ty>, whence: Whence) -> PyResult<usize> {
                    TypedIOBase::<$ty>::pwrite_array(&mut self.inner, &data, whence.into()).map_err(io_err)
                }

                #[doc = concat!("The default `", stringify!($ty), "` value used to fill a gap opened past the end on a grow (`0`).")]
                fn default_value(&self) -> $ty {
                    TypedIOBase::<$ty>::default_value(&self.inner)
                }

                /// The little-endian bytes of `count` default values — the gap-fill
                /// pattern (all-zero for every native primitive).
                fn default_byte_array<'py>(&self, py: Python<'py>, count: usize) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, &TypedIOBase::<$ty>::default_byte_array(&self.inner, count))
                }

                /// Reads up to `size` raw bytes at `whence`, advancing the cursor.
                #[pyo3(signature = (size, whence = Whence::Start))]
                fn pread_byte_array<'py>(&mut self, py: Python<'py>, size: usize, whence: Whence) -> PyResult<Bound<'py, PyBytes>> {
                    let out = self.inner.pread_byte_array(size, whence.into()).map_err(io_err)?;
                    Ok(PyBytes::new_bound(py, &out))
                }

                /// Writes raw `data` bytes at `whence`, advancing the cursor.
                #[pyo3(signature = (data, whence = Whence::Start))]
                fn pwrite_byte_array(&mut self, data: &[u8], whence: Whence) -> PyResult<usize> {
                    self.inner.pwrite_byte_array(data, whence.into()).map_err(io_err)
                }

                /// The cursor's current bytes, including any writes.
                fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, self.inner.as_bytes())
                }

                /// Freezes the cursor's bytes into a new `ByteBuffer`.
                fn to_byte_buffer(&self) -> ByteBuffer {
                    ByteBuffer {
                        inner: self.inner.to_byte_buffer(),
                    }
                }

                fn __repr__(&self) -> String {
                    format!(concat!(stringify!($name), "(position={})"), self.inner.position())
                }
            }
        )+
    };
}

py_typed_cursor!(
    (I8Cursor, i8),
    (U8Cursor, u8),
    (I16Cursor, i16),
    (U16Cursor, u16),
    (I32Cursor, i32),
    (U32Cursor, u32),
    (I64Cursor, i64),
    (U64Cursor, u64),
    (F32Cursor, f32),
    (F64Cursor, f64),
);

/// Generates one wide-integer cursor class (`yggdryl_core::TypedCursor<$ty>` for a
/// wide integer) whose values marshal to/from Python `int`. Same surface as the
/// native typed cursors; only the element marshalling differs.
macro_rules! py_wide_cursor {
    ($( ($name:ident, $ty:ty, $label:literal) ),+ $(,)?) => {
        $(
            #[doc = concat!("A positioned, advancing cursor over ", $label, " values (marshalled as Python `int`).")]
            #[pyclass(module = "yggdryl.io")]
            #[derive(Clone)]
            pub struct $name {
                pub(crate) inner: yggdryl_core::TypedCursor<$ty>,
            }

            #[pymethods]
            impl $name {
                #[doc = concat!("Creates an empty cursor preallocated for `capacity` ", $label, " values.")]
                #[staticmethod]
                fn with_capacity(capacity: usize) -> Self {
                    Self {
                        inner: <yggdryl_core::TypedCursor<$ty> as TypedIOBase<$ty>>::with_capacity(capacity),
                    }
                }

                /// Opens a cursor over a copy of `data` (little-endian bytes).
                #[staticmethod]
                fn from_bytes(data: &[u8]) -> Self {
                    Self {
                        inner: yggdryl_core::TypedCursor::new(yggdryl_core::ByteBuffer::from_bytes(data)),
                    }
                }

                /// The current position, in element units from the start.
                fn tell(&self) -> PyResult<u64> {
                    TypedIOBase::<$ty>::tell(&self.inner).map_err(io_err)
                }

                /// Moves the cursor to `offset` element units relative to `whence`.
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    TypedIOBase::<$ty>::seek(&mut self.inner, offset, whence.into()).map_err(io_err)
                }

                /// The current position, in bytes from the start.
                fn byte_tell(&self) -> PyResult<u64> {
                    self.inner.byte_tell().map_err(io_err)
                }

                /// Moves the cursor to `offset` bytes relative to `whence`.
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn byte_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    self.inner.byte_seek(offset, whence.into()).map_err(io_err)
                }

                /// The current position, in bits from the start.
                fn bit_tell(&self) -> PyResult<u64> {
                    self.inner.bit_tell().map_err(io_err)
                }

                /// Moves the cursor to `offset` bits relative to `whence` (byte-aligned).
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn bit_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    self.inner.bit_seek(offset, whence.into()).map_err(io_err)
                }

                /// The current byte position (mirror of `byte_tell`).
                fn position(&self) -> u64 {
                    self.inner.position()
                }

                /// Sets the current byte position.
                fn set_position(&mut self, position: u64) {
                    self.inner.set_position(position);
                }

                /// The number of element values **remaining** from the current position.
                fn size(&self) -> PyResult<usize> {
                    TypedIOBase::<$ty>::size(&self.inner).map_err(io_err)
                }

                /// The element capacity without reallocating.
                fn capacity(&self) -> PyResult<usize> {
                    TypedIOBase::<$ty>::capacity(&self.inner).map_err(io_err)
                }

                /// The number of bytes remaining.
                fn byte_size(&self) -> PyResult<usize> {
                    self.inner.byte_size().map_err(io_err)
                }

                /// The number of bits remaining.
                fn bit_size(&self) -> PyResult<usize> {
                    self.inner.bit_size().map_err(io_err)
                }

                /// The byte capacity without reallocating.
                fn byte_capacity(&self) -> PyResult<usize> {
                    self.inner.byte_capacity().map_err(io_err)
                }

                /// The bit capacity without reallocating.
                fn bit_capacity(&self) -> PyResult<usize> {
                    self.inner.bit_capacity().map_err(io_err)
                }

                /// Reads a single value at `whence` (as a Python `int`), advancing.
                #[pyo3(signature = (whence = Whence::Start))]
                fn pread_one(&mut self, py: Python<'_>, whence: Whence) -> PyResult<PyObject> {
                    let value = TypedIOBase::<$ty>::pread_one(&mut self.inner, whence.into()).map_err(io_err)?;
                    wide_to_py(py, value)
                }

                /// Writes a single value (a Python `int`) at `whence`, advancing.
                #[pyo3(signature = (value, whence = Whence::Start))]
                fn pwrite_one(&mut self, value: &Bound<'_, PyAny>, whence: Whence) -> PyResult<usize> {
                    let v: $ty = wide_from_py(value)?;
                    TypedIOBase::<$ty>::pwrite_one(&mut self.inner, v, whence.into()).map_err(io_err)
                }

                /// Reads up to `count` values at `whence` as a list of Python `int`.
                #[pyo3(signature = (count, whence = Whence::Start))]
                fn pread_array(&mut self, py: Python<'_>, count: usize, whence: Whence) -> PyResult<Vec<PyObject>> {
                    let values = TypedIOBase::<$ty>::pread_array(&mut self.inner, count, whence.into()).map_err(io_err)?;
                    values.into_iter().map(|v| wide_to_py(py, v)).collect()
                }

                /// Writes the values in `data` (Python `int`s) at `whence`.
                #[pyo3(signature = (data, whence = Whence::Start))]
                fn pwrite_array(&mut self, data: Vec<Bound<'_, PyAny>>, whence: Whence) -> PyResult<usize> {
                    let values: Vec<$ty> = data.iter().map(wide_from_py).collect::<PyResult<_>>()?;
                    TypedIOBase::<$ty>::pwrite_array(&mut self.inner, &values, whence.into()).map_err(io_err)
                }

                /// The default value (`0`) used to fill a gap on a grow, as a Python `int`.
                fn default_value(&self, py: Python<'_>) -> PyResult<PyObject> {
                    wide_to_py(py, TypedIOBase::<$ty>::default_value(&self.inner))
                }

                /// The little-endian bytes of `count` default values (all-zero).
                fn default_byte_array<'py>(&self, py: Python<'py>, count: usize) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, &TypedIOBase::<$ty>::default_byte_array(&self.inner, count))
                }

                /// The cursor's current bytes, including any writes.
                fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, self.inner.as_bytes())
                }

                /// Freezes the cursor's bytes into a new `ByteBuffer`.
                fn to_byte_buffer(&self) -> ByteBuffer {
                    ByteBuffer { inner: self.inner.to_byte_buffer() }
                }

                fn __repr__(&self) -> String {
                    format!(concat!(stringify!($name), "(position={})"), self.inner.position())
                }
            }
        )+
    };
}

py_wide_cursor!(
    (I96Cursor, yggdryl_core::i96, "96-bit signed integer"),
    (I128Cursor, i128, "128-bit signed integer"),
    (I256Cursor, yggdryl_core::i256, "256-bit signed integer"),
);

/// A bounded, non-growing byte **window** `[offset, offset + len)` over a `ByteBuffer`.
#[pyclass(module = "yggdryl.io")]
#[derive(Clone)]
pub struct ByteSlice {
    pub(crate) inner: yggdryl_core::ByteSlice,
}

#[pymethods]
impl ByteSlice {
    /// Opens a window `[offset, offset + len)` over a copy of `data` (clamped).
    #[staticmethod]
    fn from_bytes(data: &[u8], offset: u64, len: usize) -> Self {
        Self {
            inner: yggdryl_core::ByteSlice::new(
                yggdryl_core::ByteBuffer::from_bytes(data),
                offset,
                len,
            ),
        }
    }

    /// The window's start offset within the origin resource, in bytes.
    fn slice_offset(&self) -> u64 {
        self.inner.slice_offset()
    }

    /// The window's length in bytes (its fixed extent).
    fn slice_len(&self) -> usize {
        self.inner.slice_len()
    }

    /// The current position, in bytes from the window start.
    fn tell(&self) -> PyResult<u64> {
        self.inner.byte_tell().map_err(io_err)
    }

    /// Moves to `offset` bytes relative to `whence` (within the window).
    #[pyo3(signature = (offset, whence = Whence::Start))]
    fn seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
        self.inner.byte_seek(offset, whence.into()).map_err(io_err)
    }

    /// The current position in bits from the window start (`tell * 8`).
    fn bit_tell(&self) -> PyResult<u64> {
        self.inner.bit_tell().map_err(io_err)
    }

    /// Moves to `offset` bits relative to `whence` (byte-aligned).
    #[pyo3(signature = (offset, whence = Whence::Start))]
    fn bit_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
        self.inner.bit_seek(offset, whence.into()).map_err(io_err)
    }

    /// The current position (mirror of `tell`).
    fn position(&self) -> u64 {
        self.inner.position()
    }

    /// Sets the current position (within the window).
    fn set_position(&mut self, position: u64) {
        self.inner.set_position(position);
    }

    /// The number of bytes **remaining** in the window from the current position.
    fn byte_size(&self) -> PyResult<usize> {
        self.inner.byte_size().map_err(io_err)
    }

    /// The number of bits remaining in the window.
    fn bit_size(&self) -> PyResult<usize> {
        self.inner.bit_size().map_err(io_err)
    }

    /// The window's byte capacity (its fixed length).
    fn byte_capacity(&self) -> PyResult<usize> {
        self.inner.byte_capacity().map_err(io_err)
    }

    /// Reads up to `size` bytes at `whence`, clamped to the window.
    #[pyo3(signature = (size, whence = Whence::Start))]
    fn pread_byte_array<'py>(
        &mut self,
        py: Python<'py>,
        size: usize,
        whence: Whence,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = self
            .inner
            .pread_byte_array(size, whence.into())
            .map_err(io_err)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Writes `data` at `whence`, clamped to the window (never grows); returns bytes written.
    #[pyo3(signature = (data, whence = Whence::Start))]
    fn pwrite_byte_array(&mut self, data: &[u8], whence: Whence) -> PyResult<usize> {
        self.inner
            .pwrite_byte_array(data, whence.into())
            .map_err(io_err)
    }

    /// Reads up to `len(buf)` bytes at `whence` into the writable `buf`, clamped.
    #[pyo3(signature = (buf, whence = Whence::Start))]
    fn pread_into(&mut self, buf: &Bound<'_, PyByteArray>, whence: Whence) -> PyResult<usize> {
        // SAFETY: the GIL is held; no arbitrary Python runs while the borrow is live.
        let slice = unsafe { buf.as_bytes_mut() };
        self.inner.pread_into(slice, whence.into()).map_err(io_err)
    }

    /// The window's current bytes, including any writes.
    fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_bytes())
    }

    /// Freezes the window's bytes into a new `ByteBuffer`.
    fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ByteSlice(offset={}, len={})",
            self.inner.slice_offset(),
            self.inner.slice_len()
        )
    }
}

/// Generates one element-typed slice class (`yggdryl_core::TypedSlice<$ty>`), the
/// bounded, non-growing sibling of the typed cursors.
macro_rules! py_typed_slice {
    ($( ($name:ident, $ty:ty) ),+ $(,)?) => {
        $(
            #[doc = concat!("A bounded window over `", stringify!($ty), "` values (native units).")]
            #[pyclass(module = "yggdryl.io")]
            #[derive(Clone)]
            pub struct $name {
                pub(crate) inner: yggdryl_core::TypedSlice<$ty>,
            }

            #[pymethods]
            impl $name {
                /// Opens a window over a copy of `data` spanning the byte range `[offset, offset+len)`.
                #[staticmethod]
                fn from_bytes(data: &[u8], offset: u64, len: usize) -> Self {
                    Self { inner: yggdryl_core::TypedSlice::new(yggdryl_core::ByteBuffer::from_bytes(data), offset, len) }
                }

                /// The window's start offset within the origin resource, in bytes.
                fn slice_offset(&self) -> u64 { self.inner.slice_offset() }
                /// The window's length in bytes.
                fn slice_len(&self) -> usize { self.inner.slice_len() }

                #[doc = concat!("The current position, in `", stringify!($ty), "` values from the window start.")]
                fn tell(&self) -> PyResult<u64> { TypedIOBase::<$ty>::tell(&self.inner).map_err(io_err) }

                #[doc = concat!("Moves to `offset` `", stringify!($ty), "` values relative to `whence`.")]
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    TypedIOBase::<$ty>::seek(&mut self.inner, offset, whence.into()).map_err(io_err)
                }

                /// The current position, in bytes from the window start.
                fn byte_tell(&self) -> PyResult<u64> { self.inner.byte_tell().map_err(io_err) }
                /// Moves to `offset` bytes relative to `whence`.
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn byte_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    self.inner.byte_seek(offset, whence.into()).map_err(io_err)
                }
                /// The current position, in bits from the window start.
                fn bit_tell(&self) -> PyResult<u64> { self.inner.bit_tell().map_err(io_err) }
                /// Moves to `offset` bits relative to `whence` (byte-aligned).
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn bit_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    self.inner.bit_seek(offset, whence.into()).map_err(io_err)
                }
                /// The current byte position (mirror of `byte_tell`).
                fn position(&self) -> u64 { self.inner.position() }
                /// Sets the current byte position (within the window).
                fn set_position(&mut self, position: u64) { self.inner.set_position(position); }

                #[doc = concat!("The number of `", stringify!($ty), "` values remaining from the current position.")]
                fn size(&self) -> PyResult<usize> { TypedIOBase::<$ty>::size(&self.inner).map_err(io_err) }
                #[doc = concat!("The `", stringify!($ty), "` capacity (the window's fixed length).")]
                fn capacity(&self) -> PyResult<usize> { TypedIOBase::<$ty>::capacity(&self.inner).map_err(io_err) }
                /// The number of bytes remaining in the window.
                fn byte_size(&self) -> PyResult<usize> { self.inner.byte_size().map_err(io_err) }
                /// The number of bits remaining in the window.
                fn bit_size(&self) -> PyResult<usize> { self.inner.bit_size().map_err(io_err) }
                /// The window's byte capacity.
                fn byte_capacity(&self) -> PyResult<usize> { self.inner.byte_capacity().map_err(io_err) }

                #[doc = concat!("Reads a single `", stringify!($ty), "` at `whence`, advancing.")]
                #[pyo3(signature = (whence = Whence::Start))]
                fn pread_one(&mut self, whence: Whence) -> PyResult<$ty> {
                    TypedIOBase::<$ty>::pread_one(&mut self.inner, whence.into()).map_err(io_err)
                }
                #[doc = concat!("Writes a single `", stringify!($ty), "` at `whence` (clamped to the window).")]
                #[pyo3(signature = (value, whence = Whence::Start))]
                fn pwrite_one(&mut self, value: $ty, whence: Whence) -> PyResult<usize> {
                    TypedIOBase::<$ty>::pwrite_one(&mut self.inner, value, whence.into()).map_err(io_err)
                }
                #[doc = concat!("Reads up to `count` `", stringify!($ty), "` values at `whence`, clamped.")]
                #[pyo3(signature = (count, whence = Whence::Start))]
                fn pread_array(&mut self, count: usize, whence: Whence) -> PyResult<Vec<$ty>> {
                    TypedIOBase::<$ty>::pread_array(&mut self.inner, count, whence.into()).map_err(io_err)
                }
                #[doc = concat!("Writes the `", stringify!($ty), "` values in `data` at `whence` (only whole values that fit).")]
                #[pyo3(signature = (data, whence = Whence::Start))]
                fn pwrite_array(&mut self, data: Vec<$ty>, whence: Whence) -> PyResult<usize> {
                    TypedIOBase::<$ty>::pwrite_array(&mut self.inner, &data, whence.into()).map_err(io_err)
                }

                /// The window's current bytes.
                fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, self.inner.as_bytes())
                }
                /// Freezes the window's bytes into a new `ByteBuffer`.
                fn to_byte_buffer(&self) -> ByteBuffer { ByteBuffer { inner: self.inner.to_byte_buffer() } }
            }
        )+
    };
}

py_typed_slice!(
    (I8Slice, i8),
    (U8Slice, u8),
    (I16Slice, i16),
    (U16Slice, u16),
    (I32Slice, i32),
    (U32Slice, u32),
    (I64Slice, i64),
    (U64Slice, u64),
    (F32Slice, f32),
    (F64Slice, f64),
);

/// Generates one wide-integer slice class (`yggdryl_core::TypedSlice<$ty>`) whose
/// values marshal to/from Python `int`.
macro_rules! py_wide_slice {
    ($( ($name:ident, $ty:ty, $label:literal) ),+ $(,)?) => {
        $(
            #[doc = concat!("A bounded window over ", $label, " values (marshalled as Python `int`).")]
            #[pyclass(module = "yggdryl.io")]
            #[derive(Clone)]
            pub struct $name {
                pub(crate) inner: yggdryl_core::TypedSlice<$ty>,
            }

            #[pymethods]
            impl $name {
                /// Opens a window over a copy of `data` spanning the byte range `[offset, offset+len)`.
                #[staticmethod]
                fn from_bytes(data: &[u8], offset: u64, len: usize) -> Self {
                    Self { inner: yggdryl_core::TypedSlice::new(yggdryl_core::ByteBuffer::from_bytes(data), offset, len) }
                }
                /// The window's start offset within the origin resource, in bytes.
                fn slice_offset(&self) -> u64 { self.inner.slice_offset() }
                /// The window's length in bytes.
                fn slice_len(&self) -> usize { self.inner.slice_len() }
                /// The current position, in element units from the window start.
                fn tell(&self) -> PyResult<u64> { TypedIOBase::<$ty>::tell(&self.inner).map_err(io_err) }
                /// Moves to `offset` element units relative to `whence`.
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    TypedIOBase::<$ty>::seek(&mut self.inner, offset, whence.into()).map_err(io_err)
                }
                /// The current position, in bytes from the window start.
                fn byte_tell(&self) -> PyResult<u64> { self.inner.byte_tell().map_err(io_err) }
                /// Moves to `offset` bytes relative to `whence`.
                #[pyo3(signature = (offset, whence = Whence::Start))]
                fn byte_seek(&mut self, offset: i64, whence: Whence) -> PyResult<u64> {
                    self.inner.byte_seek(offset, whence.into()).map_err(io_err)
                }
                /// The current byte position (mirror of `byte_tell`).
                fn position(&self) -> u64 { self.inner.position() }
                /// Sets the current byte position (within the window).
                fn set_position(&mut self, position: u64) { self.inner.set_position(position); }
                /// The number of element values remaining from the current position.
                fn size(&self) -> PyResult<usize> { TypedIOBase::<$ty>::size(&self.inner).map_err(io_err) }
                /// The element capacity (the window's fixed length).
                fn capacity(&self) -> PyResult<usize> { TypedIOBase::<$ty>::capacity(&self.inner).map_err(io_err) }
                /// The number of bytes remaining in the window.
                fn byte_size(&self) -> PyResult<usize> { self.inner.byte_size().map_err(io_err) }
                /// Reads a single value at `whence` (as a Python `int`), advancing.
                #[pyo3(signature = (whence = Whence::Start))]
                fn pread_one(&mut self, py: Python<'_>, whence: Whence) -> PyResult<PyObject> {
                    let value = TypedIOBase::<$ty>::pread_one(&mut self.inner, whence.into()).map_err(io_err)?;
                    wide_to_py(py, value)
                }
                /// Writes a single value (a Python `int`) at `whence` (clamped to the window).
                #[pyo3(signature = (value, whence = Whence::Start))]
                fn pwrite_one(&mut self, value: &Bound<'_, PyAny>, whence: Whence) -> PyResult<usize> {
                    let v: $ty = wide_from_py(value)?;
                    TypedIOBase::<$ty>::pwrite_one(&mut self.inner, v, whence.into()).map_err(io_err)
                }
                /// Reads up to `count` values at `whence` as a list of Python `int`.
                #[pyo3(signature = (count, whence = Whence::Start))]
                fn pread_array(&mut self, py: Python<'_>, count: usize, whence: Whence) -> PyResult<Vec<PyObject>> {
                    let values = TypedIOBase::<$ty>::pread_array(&mut self.inner, count, whence.into()).map_err(io_err)?;
                    values.into_iter().map(|v| wide_to_py(py, v)).collect()
                }
                /// Writes the values in `data` (Python `int`s) at `whence` (only whole values that fit).
                #[pyo3(signature = (data, whence = Whence::Start))]
                fn pwrite_array(&mut self, data: Vec<Bound<'_, PyAny>>, whence: Whence) -> PyResult<usize> {
                    let values: Vec<$ty> = data.iter().map(wide_from_py).collect::<PyResult<_>>()?;
                    TypedIOBase::<$ty>::pwrite_array(&mut self.inner, &values, whence.into()).map_err(io_err)
                }
                /// The window's current bytes.
                fn as_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, self.inner.as_bytes())
                }
                /// Freezes the window's bytes into a new `ByteBuffer`.
                fn to_byte_buffer(&self) -> ByteBuffer { ByteBuffer { inner: self.inner.to_byte_buffer() } }
            }
        )+
    };
}

py_wide_slice!(
    (I96Slice, yggdryl_core::i96, "96-bit signed integer"),
    (I128Slice, i128, "128-bit signed integer"),
    (I256Slice, yggdryl_core::i256, "256-bit signed integer"),
);

/// Populates the `io` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Whence>()?;
    module.add_class::<ByteBuffer>()?;
    module.add_class::<ByteCursor>()?;
    module.add_class::<ByteSlice>()?;
    module.add_class::<I96Cursor>()?;
    module.add_class::<I128Cursor>()?;
    module.add_class::<I256Cursor>()?;
    module.add_class::<I96Slice>()?;
    module.add_class::<I128Slice>()?;
    module.add_class::<I256Slice>()?;
    module.add_class::<I8Cursor>()?;
    module.add_class::<U8Cursor>()?;
    module.add_class::<I16Cursor>()?;
    module.add_class::<U16Cursor>()?;
    module.add_class::<I32Cursor>()?;
    module.add_class::<U32Cursor>()?;
    module.add_class::<I64Cursor>()?;
    module.add_class::<U64Cursor>()?;
    module.add_class::<F32Cursor>()?;
    module.add_class::<F64Cursor>()?;
    module.add_class::<I8Slice>()?;
    module.add_class::<U8Slice>()?;
    module.add_class::<I16Slice>()?;
    module.add_class::<U16Slice>()?;
    module.add_class::<I32Slice>()?;
    module.add_class::<U32Slice>()?;
    module.add_class::<I64Slice>()?;
    module.add_class::<U64Slice>()?;
    module.add_class::<F32Slice>()?;
    module.add_class::<F64Slice>()?;
    Ok(())
}
