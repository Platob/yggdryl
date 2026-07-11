//! The `yggdryl.compression` submodule — thin wrappers over the compression
//! codecs in `yggdryl-core`.
//!
//! Only the concrete codecs cross the FFI boundary. The core's `Encoder` /
//! `Decoder` / `Compression` traits (and their generic `Typed*` variants) are
//! Rust-only contracts — generics and marker traits cannot be expressed across the
//! binding — so they are **not** replicated here; this is a deliberate, documented
//! omission per `CLAUDE.md`. Exposes [`Gzip`] and [`Zstd`]. The one-shot
//! `CompressIO` (compress/decompress an IO with any codec) is Rust-only — it takes a
//! generic `dyn` codec that does not cross the FFI boundary; use `compress_stream`
//! or `encode_byte_array` instead.

// The `#[pymethods]` macro emits identity `.into()` conversions on `PyResult`
// returns that clippy flags as useless; the offending code is generated in
// sibling `const _` blocks, so the allow must sit at module scope to reach them.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl_compression::{Compression, CompressionDecoder, CompressionEncoder};
use yggdryl_core::{Decoder, Encoder};

use crate::io::ByteCursor;

/// Maps a core [`yggdryl_core::EncodeError`] to a Python `ValueError`.
fn encode_err(error: yggdryl_core::EncodeError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Maps a core [`yggdryl_core::DecodeError`] to a Python `ValueError`.
fn decode_err(error: yggdryl_core::DecodeError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The gzip (RFC 1952) compression codec.
///
/// Mirrors `yggdryl_compression::Gzip`: construct with a level in `0..=9` (default `6`),
/// then `encode_byte_array` / `decode_byte_array` to compress / decompress.
#[pyclass(module = "yggdryl.compression", frozen)]
#[derive(Clone)]
pub struct Gzip {
    inner: yggdryl_compression::Gzip,
}

#[pymethods]
impl Gzip {
    /// Creates a gzip codec at `level` (`0..=9`, default `6`).
    #[new]
    #[pyo3(signature = (level = yggdryl_compression::Gzip::DEFAULT_LEVEL))]
    fn new(level: u32) -> PyResult<Self> {
        Ok(Self {
            inner: yggdryl_compression::Gzip::new(level).map_err(encode_err)?,
        })
    }

    /// The lowercase codec name (`"gzip"`).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// The configured compression level.
    #[getter]
    fn level(&self) -> u32 {
        self.inner.level()
    }

    /// Compresses `data`, returning the gzip stream.
    fn encode_byte_array<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.encode_byte_array(data).map_err(encode_err)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Decompresses the gzip `data` stream.
    fn decode_byte_array<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decode_byte_array(data).map_err(decode_err)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Stream-compresses every byte remaining under `source` into `sink`, returning
    /// the number of bytes written. Both cursors advance.
    fn compress_stream(&self, source: &mut ByteCursor, sink: &mut ByteCursor) -> PyResult<u64> {
        self.inner
            .compress_stream(&mut source.inner, &mut sink.inner)
            .map_err(encode_err)
    }

    /// Stream-decompresses every byte remaining under `source` into `sink`,
    /// returning the number of bytes written. Both cursors advance.
    fn decompress_stream(&self, source: &mut ByteCursor, sink: &mut ByteCursor) -> PyResult<u64> {
        self.inner
            .decompress_stream(&mut source.inner, &mut sink.inner)
            .map_err(decode_err)
    }

    /// Serialises the codec to bytes (the single level byte).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a codec from [`serialize_bytes`](Gzip::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        Ok(Self {
            inner: yggdryl_compression::Gzip::deserialize_bytes(bytes).map_err(decode_err)?,
        })
    }

    /// Enables `pickle` round-trips through the byte codec.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<Gzip>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!("Gzip(level={})", self.inner.level())
    }

    /// Content equality — two codecs are equal iff their `serialize_bytes` match.
    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// Content hash, consistent with [`__eq__`](Gzip::__eq__).
    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }
}

/// The Zstandard (RFC 8878) compression codec.
///
/// Mirrors `yggdryl_compression::Zstd`: construct with a level in `level_range()` (default
/// `3`), then `encode_byte_array` / `decode_byte_array` or the streaming pair.
#[pyclass(module = "yggdryl.compression", frozen)]
#[derive(Clone)]
pub struct Zstd {
    inner: yggdryl_compression::Zstd,
}

#[pymethods]
impl Zstd {
    /// Creates a zstd codec at `level` (default `3`).
    #[new]
    #[pyo3(signature = (level = yggdryl_compression::Zstd::DEFAULT_LEVEL))]
    fn new(level: i32) -> PyResult<Self> {
        Ok(Self {
            inner: yggdryl_compression::Zstd::new(level).map_err(encode_err)?,
        })
    }

    /// The inclusive `(min, max)` levels this build of zstd accepts.
    #[staticmethod]
    fn level_range() -> (i32, i32) {
        yggdryl_compression::Zstd::level_range()
    }

    /// The lowercase codec name (`"zstd"`).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// The configured compression level.
    #[getter]
    fn level(&self) -> i32 {
        self.inner.level()
    }

    /// Compresses `data`, returning the zstd frame.
    fn encode_byte_array<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.encode_byte_array(data).map_err(encode_err)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Decompresses the zstd `data` frame.
    fn decode_byte_array<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decode_byte_array(data).map_err(decode_err)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Stream-compresses everything under `source` into `sink`.
    fn compress_stream(&self, source: &mut ByteCursor, sink: &mut ByteCursor) -> PyResult<u64> {
        self.inner
            .compress_stream(&mut source.inner, &mut sink.inner)
            .map_err(encode_err)
    }

    /// Stream-decompresses everything under `source` into `sink`.
    fn decompress_stream(&self, source: &mut ByteCursor, sink: &mut ByteCursor) -> PyResult<u64> {
        self.inner
            .decompress_stream(&mut source.inner, &mut sink.inner)
            .map_err(decode_err)
    }

    /// Serialises the codec to bytes (the 4-byte level).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a codec from `serialize_bytes`.
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        Ok(Self {
            inner: yggdryl_compression::Zstd::deserialize_bytes(bytes).map_err(decode_err)?,
        })
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<Zstd>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!("Zstd(level={})", self.inner.level())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }
}

/// Populates the `compression` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Gzip>()?;
    module.add_class::<Zstd>()?;
    Ok(())
}
