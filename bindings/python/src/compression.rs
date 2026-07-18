//! The `yggdryl.compression` submodule — the four native compression codecs and the
//! media-type → codec resolver.
//!
//! Mirrors `yggdryl_core::compression`: the [`Compression`](yggdryl_core::compression::Compression)
//! contract's concrete codecs — [`Gzip`] / [`Zlib`] / [`Zstd`] / [`Lzma`] (the native flate2 /
//! zstd / xz2 cores, compiled in by the binding's `compression` feature) — plus the
//! module-level [`codec_for`] resolver that maps a mime **essence** string or a
//! [`MimeType`](crate::mimetype::MimeType) to the matching codec, or `None`.
//!
//! Each codec is a small configured operator: a constructor takes an optional compression
//! `level`, and `compress` / `decompress` run the codec over `bytes`. A failed
//! compress/decompress raises a guided `ValueError` carrying the core error text unchanged.
//! Every method is one or two lines over `yggdryl_core`.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyBytes;

use crate::mimetype::MimeType;
use yggdryl_core::compression::{self as core, Compression};
use yggdryl_core::io::IoError;

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

// The process-wide **cached default-level codec singletons** — one shared Python object per
// format, resolved once and handed back by reference (`Py::clone_ref` is a refcount bump, not a
// new object). So neither the module [`codec_for`] resolver nor any `IOBase` source's
// `compression()` accessor constructs a codec per call (the "resolve shared instances once"
// rule; these mirror the core's `codec_ref_for` shared `'static` codecs). Each codec is
// immutable (a fixed level, no setters), so sharing one instance is safe.
static GZIP_CODEC: GILOnceCell<Py<Gzip>> = GILOnceCell::new();
static ZLIB_CODEC: GILOnceCell<Py<Zlib>> = GILOnceCell::new();
static ZSTD_CODEC: GILOnceCell<Py<Zstd>> = GILOnceCell::new();
static LZMA_CODEC: GILOnceCell<Py<Lzma>> = GILOnceCell::new();

/// Builds the concrete `yggdryl.compression` codec object for a core codec (resolved by
/// [`codec_for`] or a source's `compression()`), routing on the codec's own
/// [`name`](Compression::name), or `None` when nothing resolved. Returns the **shared cached
/// singleton** for the format (built once, then cloned by reference) — the single place the
/// boxed core codec becomes one of the four typed Python classes, shared by the module resolver
/// and every `IOBase` source's `compression()` accessor.
pub(crate) fn codec_to_object(
    py: Python<'_>,
    codec: Option<Box<dyn Compression>>,
) -> PyResult<Option<PyObject>> {
    let Some(codec) = codec else {
        return Ok(None);
    };
    let object = match codec.name() {
        "gzip" => GZIP_CODEC
            .get_or_try_init(py, || {
                Py::new(
                    py,
                    Gzip {
                        inner: core::Gzip::new(),
                    },
                )
            })?
            .clone_ref(py)
            .into_any(),
        "zlib" => ZLIB_CODEC
            .get_or_try_init(py, || {
                Py::new(
                    py,
                    Zlib {
                        inner: core::Zlib::new(),
                    },
                )
            })?
            .clone_ref(py)
            .into_any(),
        "zstd" => ZSTD_CODEC
            .get_or_try_init(py, || {
                Py::new(
                    py,
                    Zstd {
                        inner: core::Zstd::new(),
                    },
                )
            })?
            .clone_ref(py)
            .into_any(),
        "xz" => LZMA_CODEC
            .get_or_try_init(py, || {
                Py::new(
                    py,
                    Lzma {
                        inner: core::Lzma::new(),
                    },
                )
            })?
            .clone_ref(py)
            .into_any(),
        _ => return Ok(None),
    };
    Ok(Some(object))
}

/// Runs `op` with the core `&dyn Compression` behind any `yggdryl.compression` codec object,
/// raising a guided `TypeError` when `codec` is not one of the four codec classes — the shared
/// adapter behind every source's explicit `compress_with` / `decompress_with`.
pub(crate) fn with_codec<R>(
    codec: &Bound<'_, PyAny>,
    op: impl FnOnce(&dyn Compression) -> R,
) -> PyResult<R> {
    if let Ok(c) = codec.extract::<PyRef<'_, Gzip>>() {
        Ok(op(&c.inner))
    } else if let Ok(c) = codec.extract::<PyRef<'_, Zlib>>() {
        Ok(op(&c.inner))
    } else if let Ok(c) = codec.extract::<PyRef<'_, Zstd>>() {
        Ok(op(&c.inner))
    } else if let Ok(c) = codec.extract::<PyRef<'_, Lzma>>() {
        Ok(op(&c.inner))
    } else {
        Err(PyTypeError::new_err(format!(
            "expected a yggdryl.compression codec (Gzip / Zlib / Zstd / Lzma), got {}",
            codec.repr()?
        )))
    }
}

/// **Gzip** (RFC 1952) over the native DEFLATE core. Construct with an optional `level`
/// (`0` fastest/none … `9` smallest; default `6`).
#[pyclass(module = "yggdryl.compression")]
#[derive(Clone)]
pub struct Gzip {
    pub(crate) inner: core::Gzip,
}

#[pymethods]
impl Gzip {
    /// A gzip codec at compression `level` (`0` … `9`), or the balanced default (`6`) when
    /// omitted.
    #[new]
    #[pyo3(signature = (level = None))]
    fn new(level: Option<u32>) -> Self {
        Self {
            inner: level.map_or_else(core::Gzip::new, core::Gzip::with_level),
        }
    }

    /// The codec's mime **essence** (`"application/gzip"`).
    #[getter]
    fn essence(&self) -> &'static str {
        self.inner.essence()
    }

    /// The codec's short **name** (`"gzip"`).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// Compresses `data` (bytes / bytearray) into a new `bytes`.
    fn compress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.compress(&data).map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Decompresses `data` (a gzip stream) into a new `bytes`, raising a guided `ValueError`
    /// on a corrupt stream.
    fn decompress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decompress(&data).map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    fn __repr__(&self) -> String {
        format!("Gzip(essence={:?})", self.inner.essence())
    }
}

/// **Zlib** (RFC 1950) over the native DEFLATE core. Construct with an optional `level`
/// (`0` … `9`; default `6`).
#[pyclass(module = "yggdryl.compression")]
#[derive(Clone)]
pub struct Zlib {
    pub(crate) inner: core::Zlib,
}

#[pymethods]
impl Zlib {
    /// A zlib codec at compression `level` (`0` … `9`), or the balanced default (`6`) when
    /// omitted.
    #[new]
    #[pyo3(signature = (level = None))]
    fn new(level: Option<u32>) -> Self {
        Self {
            inner: level.map_or_else(core::Zlib::new, core::Zlib::with_level),
        }
    }

    /// The codec's mime **essence** (`"application/zlib"`).
    #[getter]
    fn essence(&self) -> &'static str {
        self.inner.essence()
    }

    /// The codec's short **name** (`"zlib"`).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// Compresses `data` (bytes / bytearray) into a new `bytes`.
    fn compress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.compress(&data).map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Decompresses `data` (a zlib stream) into a new `bytes`, raising a guided `ValueError`
    /// on a corrupt stream.
    fn decompress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decompress(&data).map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    fn __repr__(&self) -> String {
        format!("Zlib(essence={:?})", self.inner.essence())
    }
}

/// **Zstandard** over the native `libzstd` core. Construct with an optional `level`
/// (`1` fastest … `22` smallest; default `3`).
#[pyclass(module = "yggdryl.compression")]
#[derive(Clone)]
pub struct Zstd {
    pub(crate) inner: core::Zstd,
}

#[pymethods]
impl Zstd {
    /// A zstd codec at compression `level` (`1` … `22`), or the balanced default (`3`) when
    /// omitted.
    #[new]
    #[pyo3(signature = (level = None))]
    fn new(level: Option<i32>) -> Self {
        Self {
            inner: level.map_or_else(core::Zstd::new, core::Zstd::with_level),
        }
    }

    /// The codec's mime **essence** (`"application/zstd"`).
    #[getter]
    fn essence(&self) -> &'static str {
        self.inner.essence()
    }

    /// The codec's short **name** (`"zstd"`).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// Compresses `data` (bytes / bytearray) into a new `bytes`.
    fn compress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.compress(&data).map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Decompresses `data` (a zstd stream) into a new `bytes`, raising a guided `ValueError`
    /// on a corrupt stream.
    fn decompress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decompress(&data).map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    fn __repr__(&self) -> String {
        format!("Zstd(essence={:?})", self.inner.essence())
    }
}

/// **LZMA / XZ** over the native `liblzma` core. Construct with an optional `level` preset
/// (`0` fastest … `9` smallest; default `6`).
#[pyclass(module = "yggdryl.compression")]
#[derive(Clone)]
pub struct Lzma {
    pub(crate) inner: core::Lzma,
}

#[pymethods]
impl Lzma {
    /// An xz codec at compression `level` preset (`0` … `9`), or the balanced default (`6`)
    /// when omitted.
    #[new]
    #[pyo3(signature = (level = None))]
    fn new(level: Option<u32>) -> Self {
        Self {
            inner: level.map_or_else(core::Lzma::new, core::Lzma::with_level),
        }
    }

    /// The codec's mime **essence** (`"application/x-xz"`).
    #[getter]
    fn essence(&self) -> &'static str {
        self.inner.essence()
    }

    /// The codec's short **name** (`"xz"`).
    #[getter]
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    /// Compresses `data` (bytes / bytearray) into a new `bytes`.
    fn compress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.compress(&data).map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// Decompresses `data` (an xz stream) into a new `bytes`, raising a guided `ValueError`
    /// on a corrupt stream.
    fn decompress<'py>(&self, py: Python<'py>, data: Vec<u8>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decompress(&data).map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    fn __repr__(&self) -> String {
        format!("Lzma(essence={:?})", self.inner.essence())
    }
}

/// Resolves the compression codec for `mime` — a mime **essence** `str` (`"application/gzip"`)
/// or a [`MimeType`](crate::mimetype::MimeType) — returning the matching [`Gzip`] / [`Zlib`] /
/// [`Zstd`] / [`Lzma`] instance, or `None` when the type is not a supported compression. Raises
/// a `TypeError` for any other argument.
#[pyfunction]
fn codec_for(py: Python<'_>, mime: &Bound<'_, PyAny>) -> PyResult<Option<PyObject>> {
    let essence = if let Ok(s) = mime.extract::<String>() {
        s
    } else if let Ok(mime) = mime.extract::<PyRef<'_, MimeType>>() {
        mime.inner.essence().to_string()
    } else {
        return Err(PyTypeError::new_err(format!(
            "codec_for expects a mime essence str or a yggdryl.mimetype.MimeType, got {}",
            mime.repr()?
        )));
    };
    codec_to_object(py, core::codec_for(&essence))
}

/// Populates the `compression` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Gzip>()?;
    module.add_class::<Zlib>()?;
    module.add_class::<Zstd>()?;
    module.add_class::<Lzma>()?;
    module.add_function(wrap_pyfunction!(codec_for, module)?)?;
    Ok(())
}
