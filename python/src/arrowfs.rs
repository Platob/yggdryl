//! `pyarrow.fs.FileSystem` as a core [`ArrowFileSystem`], implemented once.
//!
//! The core vtable is modeled on Arrow's own `FileSystem` API, so this is a
//! transcription rather than an adaptation: each method acquires the GIL,
//! calls the stable `pyarrow.fs` surface, and hands the answer back. There is
//! no per-backend code anywhere - `S3FileSystem`, `GcsFileSystem`,
//! `LocalFileSystem`, `SubTreeFileSystem`, and a custom
//! `PyFileSystem(FileSystemHandler)` (which is also how `fsspec` arrives) all
//! reach the same seven calls, so a Yggdryl handle over any of them is one
//! constructor away.
//!
//! `pyarrow.fs` is synchronous, so nothing here bridges a runtime. `Py<PyAny>`
//! is `Send + Sync`; the GIL is taken per call and never held across a
//! boundary.

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl::arrowfs::{ArrowFileSystem, FileInfo, FileInfos};
use yggdryl::{Error, IOKind, Result};

/// A held `pyarrow.fs.FileSystem`, presented as the core vtable.
pub(crate) struct PyArrowFileSystem {
    filesystem: Py<PyAny>,
    /// The filesystem's `type_name`, read once at construction.
    ///
    /// It is the one thing the core asks for outside a fallible call, and it
    /// never changes for a given filesystem, so reading it eagerly keeps
    /// `type_name` from needing the GIL.
    name: String,
}

impl PyArrowFileSystem {
    /// Hold `filesystem`, reading the name its own API reports.
    ///
    /// An unreadable name is not a failure: it only decides the scheme a
    /// handle's location carries, and `arrowfs` is the generic one the core
    /// falls back to anyway.
    pub(crate) fn new(filesystem: &Bound<'_, PyAny>) -> Self {
        let name = filesystem
            .getattr("type_name")
            .and_then(|name| name.extract::<String>())
            .unwrap_or_else(|_| "arrowfs".to_owned());
        Self {
            filesystem: filesystem.clone().unbind(),
            name,
        }
    }

    /// Run `call` under the GIL, mapping a Python exception to a core error.
    ///
    /// The exception's own text crosses unchanged, per the error contract:
    /// what a bucket, a handler, or a credential chain said is what the
    /// caller needs, and rewording it would only hide it.
    fn with_gil<T>(&self, call: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<T>) -> Result<T> {
        Python::attach(|py| call(self.filesystem.bind(py)).map_err(|error| foreign(&error)))
    }
}

/// Carry a Python exception across as a core I/O failure, message intact.
fn foreign(error: &PyErr) -> Error {
    // The whole exception rather than only its value: `PermissionError()` and
    // `FileNotFoundError()` carry no text at all, and a handler that raises
    // one is saying everything through the class. Rendering the `PyErr`
    // spells `TypeName: message`, so the class survives when the message is
    // empty - which is what the JavaScript binding already does with its own
    // errors, and what keeps the two boundaries answering alike.
    Error::Io(std::io::Error::other(error.to_string()))
}

/// Read one `pyarrow.fs.FileInfo` into the core's shape.
fn file_info_from_py(info: &Bound<'_, PyAny>) -> PyResult<FileInfo> {
    let path: String = info.getattr("path")?.extract()?;
    // `FileType` is an enum whose name is the stable spelling; comparing the
    // name avoids importing pyarrow.fs merely to reach the enum members.
    let kind = match info
        .getattr("type")?
        .getattr("name")?
        .extract::<String>()?
        .as_str()
    {
        "File" => IOKind::File,
        "Directory" => IOKind::Directory,
        _ => IOKind::Unknown,
    };
    let size = if kind == IOKind::File {
        info.getattr("size")
            .ok()
            .and_then(|size| size.extract::<u64>().ok())
            .unwrap_or(0)
    } else {
        0
    };
    Ok(FileInfo { path, kind, size })
}

impl ArrowFileSystem for PyArrowFileSystem {
    fn type_name(&self) -> &str {
        &self.name
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.with_gil(|filesystem| {
            let info = filesystem.call_method1("get_file_info", (path,))?;
            file_info_from_py(&info)
        })
    }

    fn list(&self, path: &str, recursive: bool) -> FileInfos {
        // `pyarrow.fs` answers a selector with a *list* of file infos, so the
        // foreign call is what collects, not this wrapper: there is no shape
        // in that API to pull one entry at a time from. The bound is the
        // foreign call's own answer, and it is stated here because this is
        // where the collection happens.
        let entries: Result<Vec<FileInfo>> = self.with_gil(|filesystem| {
            let py = filesystem.py();
            // A selector is how pyarrow.fs asks for the contents of a prefix,
            // and `allow_not_found` is what makes a missing directory list
            // empty rather than raise - the core's contract exactly.
            let selector = py.import("pyarrow.fs")?.getattr("FileSelector")?;
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("base_dir", path)?;
            kwargs.set_item("recursive", recursive)?;
            kwargs.set_item("allow_not_found", true)?;
            let selector = selector.call((), Some(&kwargs))?;
            let entries = filesystem.call_method1("get_file_info", (selector,))?;
            entries
                .try_iter()?
                .map(|entry| file_info_from_py(&entry?))
                .collect()
        });
        match entries {
            Ok(entries) => FileInfos::new(entries.into_iter().map(Ok)),
            Err(error) => FileInfos::failing(error),
        }
    }

    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        // A missing file reads zero bytes rather than raising, per the vtable
        // contract, and asking first is the only way to tell that apart from
        // a genuine transport failure.
        if self.file_info(path)?.kind != IOKind::File {
            return Ok(0);
        }
        let wanted = buffer.len();
        let bytes = self.with_gil(|filesystem| {
            let stream = filesystem.call_method1("open_input_file", (path,))?;
            // `read_at` is the ranged read every pyarrow filesystem offers and
            // the one that becomes a single ranged GET on an object store.
            // Only a stream without it pays for a seek.
            let chunk = if stream.hasattr("read_at").unwrap_or(false) {
                stream.call_method1("read_at", (wanted, offset))?
            } else {
                stream.call_method1("seek", (offset,))?;
                stream.call_method1("read", (wanted,))?
            };
            let bytes = chunk.extract::<Vec<u8>>()?;
            stream.call_method0("close")?;
            Ok(bytes)
        })?;
        let count = bytes.len().min(wanted);
        buffer[..count].copy_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn write_full(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.with_gil(|filesystem| {
            let py = filesystem.py();
            // Creating the parents first is the core's contract; a filesystem
            // that needs no directories (an object store) treats it as a
            // no-op, so this is not a per-backend branch.
            if let Some((parent, _)) = path.rsplit_once('/') {
                if !parent.is_empty() {
                    let kwargs = pyo3::types::PyDict::new(py);
                    kwargs.set_item("recursive", true)?;
                    let _ = filesystem.call_method("create_dir", (parent,), Some(&kwargs));
                }
            }
            // `compression=None` is load-bearing, not a default spelled out:
            // `open_output_stream` otherwise defaults to `"detect"` and picks
            // a codec from the suffix, so writing to `trades.json.gz` would
            // gzip the bytes on the way down. This method's contract is to
            // store exactly what it was handed - the content coding belongs
            // to the handle, where `Coded` already applied it, and applying
            // it again here would store a value nothing can read back.
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("compression", py.None())?;
            let stream = filesystem.call_method("open_output_stream", (path,), Some(&kwargs))?;
            stream.call_method1("write", (PyBytes::new(py, bytes),))?;
            stream.call_method0("close")?;
            Ok(())
        })
    }

    fn create_dir(&self, path: &str) -> Result<()> {
        self.with_gil(|filesystem| {
            let kwargs = pyo3::types::PyDict::new(filesystem.py());
            kwargs.set_item("recursive", true)?;
            filesystem.call_method("create_dir", (path,), Some(&kwargs))?;
            Ok(())
        })
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        // Missing is success, per the vtable contract.
        if self.file_info(path)?.kind != IOKind::File {
            return Ok(());
        }
        self.with_gil(|filesystem| {
            filesystem.call_method1("delete_file", (path,))?;
            Ok(())
        })
    }
}

/// Return whether `value` is a `pyarrow.fs.FileSystem`.
///
/// The check walks the type's own bases by module and name rather than
/// importing `pyarrow.fs` to compare against the class, so a caller who never
/// installed `PyArrow` never pays an import - and never receives an
/// `ImportError` about a library they are not using. Every filesystem `PyArrow`
/// ships, and every `PyFileSystem` wrapping a custom `FileSystemHandler`,
/// derives from that one base.
pub(crate) fn is_arrow_filesystem(value: &Bound<'_, PyAny>) -> bool {
    let Ok(bases) = value.get_type().getattr("__mro__") else {
        return false;
    };
    let Ok(bases) = bases.try_iter() else {
        return false;
    };
    for base in bases.flatten() {
        let named = base
            .getattr("__qualname__")
            .and_then(|name| name.extract::<String>())
            .is_ok_and(|name| name == "FileSystem");
        let inside = base
            .getattr("__module__")
            .and_then(|module| module.extract::<String>())
            .is_ok_and(|module| module == "pyarrow.fs" || module.starts_with("pyarrow."));
        if named && inside {
            return true;
        }
    }
    false
}
