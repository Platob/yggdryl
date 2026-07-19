//! The `io` layer of the Python binding — mirrors `yggdryl_core::io`'s folder tree: one
//! file per core module (`kind`, `mode`, `memory`, `local`). The io-root value types
//! (`Headers`, `IOMode`, `IOKind`) register on the `yggdryl.io` submodule; `memory` (the
//! in-heap sources), `local` (the local-filesystem `LocalIO` access point and the raw
//! `Mmap`, moved here from `memory` with the core's `io::local` family), and `uri` register
//! their own Python submodules.

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes};

use yggdryl_core::io::memory::IOBase;

pub mod amd;
pub mod kind;
pub mod local;
pub mod meminfo;
pub mod memory;
pub mod mode;

/// Populates the `io` submodule with the root value types shared by every source.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<mode::IOMode>()?;
    module.add_class::<kind::IOKind>()?;
    module.add_class::<meminfo::MemoryInfo>()?;
    Ok(())
}

/// Opens a core [`Uri`](yggdryl_core::uri::Uri) into the **concrete** binding source its scheme
/// selects — a `mem://` address into a [`Heap`](memory::Heap), a `file://` (or plain-path) one
/// into a [`LocalIO`](local::LocalIO) — the shared dispatch behind the module-level
/// [`open`](open_target), [`Uri.open`](crate::uri::Uri), and [`Url.open`](crate::uri::Url).
/// Mirrors the core `yggdryl_core::io::open` scheme routing, but hands back the concrete Python
/// class (dynamic typing makes the core's uniform `AnyIO` wrapper unnecessary here).
pub(crate) fn open_core_uri(py: Python<'_>, uri: &yggdryl_core::uri::Uri) -> PyResult<PyObject> {
    // `scheme()` is total: a scheme-less URI (a bare path via `from_path`) reads as the `"uri"`
    // sentinel and routes to the local family alongside `"file"`, mirroring the core `open`'s
    // `None | Some("file")` arm.
    match uri.scheme() {
        "mem" => {
            let heap = memory::Heap {
                inner: yggdryl_core::io::memory::Heap::at_uri(uri.clone()),
            };
            Ok(Py::new(py, heap)?.into_any())
        }
        "file" | "uri" => {
            let inner = yggdryl_core::io::local::LocalIO::from_uri(uri)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
            Ok(Py::new(py, local::LocalIO { inner })?.into_any())
        }
        other => Err(PyValueError::new_err(format!(
            "cannot open the `{other}://` scheme; open a `file://` (or plain path) as a LocalIO \
             or a `mem://` as a Heap"
        ))),
    }
}

/// The module-level `open()` — dispatches on the runtime type of `target` and returns the
/// **concrete** opened source: a `bytes` / `bytearray` wraps into a [`Heap`](memory::Heap); a
/// [`Uri`](crate::uri::Uri) / [`Url`](crate::uri::Url) or a `str` address routes by scheme
/// through [`open_core_uri`]; an `os.PathLike` (a `pathlib.Path`) resolves through `os.fspath`
/// into a [`LocalIO`](local::LocalIO). The binding analogue of Python's builtin `open`.
pub(crate) fn open_target(py: Python<'_>, target: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    // A yggdryl.uri.Uri / Url addresses a source directly — dispatch by its scheme.
    if let Ok(uri) = target.extract::<PyRef<'_, crate::uri::Uri>>() {
        return open_core_uri(py, &uri.inner);
    }
    if let Ok(url) = target.extract::<PyRef<'_, crate::uri::Url>>() {
        return open_core_uri(py, url.inner.as_uri());
    }
    // Raw bytes / bytearray wrap directly into an in-heap buffer.
    if let Ok(bytes) = target.downcast::<PyBytes>() {
        let heap = memory::Heap {
            inner: yggdryl_core::io::memory::Heap::from_slice(bytes.as_bytes()),
        };
        return Ok(Py::new(py, heap)?.into_any());
    }
    if let Ok(bytes) = target.downcast::<PyByteArray>() {
        let heap = memory::Heap {
            inner: yggdryl_core::io::memory::Heap::from_vec(bytes.to_vec()),
        };
        return Ok(Py::new(py, heap)?.into_any());
    }
    // A str is a plain path or a `mem://` / `file://` URI.
    if let Ok(s) = target.extract::<String>() {
        let uri = yggdryl_core::uri::Uri::parse_str(&s)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        return open_core_uri(py, &uri);
    }
    // An os.PathLike (a pathlib.Path, …) resolves through os.fspath to a local file.
    let os = py.import_bound("os")?;
    if let Ok(fspath) = os.call_method1("fspath", (target,)) {
        if let Ok(s) = fspath.extract::<String>() {
            let io = local::LocalIO {
                inner: yggdryl_core::io::local::LocalIO::from_path(s),
            };
            return Ok(Py::new(py, io)?.into_any());
        }
    }
    Err(PyTypeError::new_err(format!(
        "cannot open {}: expected a str path, bytes / bytearray, a yggdryl.uri.Uri / Url, or an \
         os.PathLike (e.g. pathlib.Path)",
        target.repr()?
    )))
}

/// Opens `uri`'s source, reads **all** of its bytes, and wraps them in a fresh
/// [`Cursor`](memory::Cursor) — the "redirect this address to a byte cursor" convenience behind
/// [`Uri.cursor`](crate::uri::Uri) / [`Url.cursor`](crate::uri::Url). The cursor owns an
/// independent in-heap copy, so it works the same whatever scheme resolved the source.
pub(crate) fn cursor_over_uri(uri: &yggdryl_core::uri::Uri) -> PyResult<memory::Cursor> {
    let io = yggdryl_core::io::open(uri).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let bytes = io.pread_vec(0, io.byte_size() as usize);
    Ok(memory::Cursor {
        inner: yggdryl_core::io::memory::Heap::from_vec(bytes).cursor(),
    })
}
