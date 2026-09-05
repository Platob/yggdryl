//! `pyarrow.fs.FileSystem` as the core Arrow-compatible filesystem seam.

use std::any::Any;
use std::io::{ErrorKind, SeekFrom};

use pyo3::exceptions::{
    PyFileExistsError, PyFileNotFoundError, PyIsADirectoryError, PyNotADirectoryError,
    PyNotImplementedError, PyOSError, PyPermissionError, PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use yggdryl::holder::fs::{
    ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, OutputMetadata,
    RandomAccessReader, mask_uri,
};
use yggdryl::{Error, Result};

/// One held `pyarrow.fs.FileSystem`.
///
/// Keeping the original object is load-bearing: two handlers of the same
/// Python type can have different roots, credentials, or bytes.
pub(crate) struct PyFileSystem {
    filesystem: Py<PyAny>,
    name: String,
}

impl PyFileSystem {
    pub(crate) fn new(filesystem: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name = filesystem.getattr("type_name")?.extract::<String>()?;
        Ok(Self {
            filesystem: filesystem.clone().unbind(),
            name,
        })
    }

    pub(crate) fn original(&self, py: Python<'_>) -> Py<PyAny> {
        self.filesystem.clone_ref(py)
    }

    fn with_gil<T>(
        &self,
        operation: &'static str,
        path: &str,
        call: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<T>,
    ) -> Result<T> {
        Python::attach(|py| {
            call(self.filesystem.bind(py))
                .map_err(|error| foreign(py, &error, operation, path, self.type_name()))
        })
    }

    fn with_gil_file<T>(
        &self,
        operation: &'static str,
        path: &str,
        call: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<T>,
    ) -> Result<T> {
        Python::attach(|py| {
            let filesystem = self.filesystem.bind(py);
            call(filesystem).map_err(|error| {
                foreign_file_operation(py, filesystem, &error, operation, path, self.type_name())
            })
        })
    }

    fn with_gil_files<T>(
        &self,
        operation: &'static str,
        source: &str,
        target: &str,
        call: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<T>,
    ) -> Result<T> {
        Python::attach(|py| {
            let filesystem = self.filesystem.bind(py);
            call(filesystem).map_err(|error| {
                if should_classify_file_error(py, &error) {
                    for path in [source, target] {
                        if is_directory(filesystem, path) {
                            return is_directory_error(path);
                        }
                    }
                }
                foreign(py, &error, operation, source, self.type_name())
            })
        })
    }

    fn with_gil_directory<T>(
        &self,
        operation: &'static str,
        path: &str,
        call: impl FnOnce(&Bound<'_, PyAny>) -> PyResult<T>,
    ) -> Result<T> {
        Python::attach(|py| {
            let filesystem = self.filesystem.bind(py);
            call(filesystem).map_err(|error| {
                if should_classify_file_error(py, &error)
                    && filesystem
                        .call_method1("get_file_info", (path,))
                        .and_then(|info| file_info_from_py(&info))
                        .is_ok_and(|info| info.kind == yggdryl::IOKind::File)
                {
                    return Error::Io(std::io::Error::new(
                        ErrorKind::NotADirectory,
                        format!("expected a directory at {:?}, got a file", mask_uri(path)),
                    ));
                }
                foreign(py, &error, operation, path, self.type_name())
            })
        })
    }
}

fn safe_message(error: &PyErr) -> String {
    mask_uri(&error.to_string())
}

fn io_error(kind: ErrorKind, error: &PyErr) -> Error {
    Error::Io(std::io::Error::new(kind, safe_message(error)))
}

fn has_errno(py: Python<'_>, error: &PyErr, name: &str) -> bool {
    let Ok(expected) = py.import("errno").and_then(|module| module.getattr(name)) else {
        return false;
    };
    error
        .value(py)
        .getattr("errno")
        .and_then(|actual| actual.eq(&expected))
        .unwrap_or(false)
}

/// Preserve Python's filesystem error class while crossing the Rust seam.
fn foreign(
    py: Python<'_>,
    error: &PyErr,
    operation: &'static str,
    path: &str,
    backend: &str,
) -> Error {
    let safe_path = mask_uri(path);
    if error.is_instance_of::<PyFileNotFoundError>(py) {
        Error::absent("resource", safe_path)
    } else if error.is_instance_of::<PyFileExistsError>(py) {
        Error::conflict("resource", "resource", safe_path)
    } else if error.is_instance_of::<PyPermissionError>(py) {
        io_error(ErrorKind::PermissionDenied, error)
    } else if error.is_instance_of::<PyNotADirectoryError>(py) {
        io_error(ErrorKind::NotADirectory, error)
    } else if error.is_instance_of::<PyIsADirectoryError>(py) {
        io_error(ErrorKind::IsADirectory, error)
    } else if error.is_instance_of::<PyNotImplementedError>(py)
        || py
            .import("io")
            .and_then(|io| io.getattr("UnsupportedOperation"))
            .is_ok_and(|class| error.matches(py, class).unwrap_or(false))
        || error.is_instance_of::<PyOSError>(py)
            && ["ENOTSUP", "EOPNOTSUPP", "ENOSYS"]
                .iter()
                .any(|name| has_errno(py, error, name))
    {
        Error::unsupported(operation, backend)
    } else if error.is_instance_of::<PyOSError>(py) && has_errno(py, error, "ENOTEMPTY") {
        io_error(ErrorKind::DirectoryNotEmpty, error)
    } else {
        io_error(ErrorKind::Other, error)
    }
}

fn should_classify_file_error(py: Python<'_>, error: &PyErr) -> bool {
    if error.is_instance_of::<PyPermissionError>(py) {
        return true;
    }
    error.is_instance_of::<PyOSError>(py)
        && !error.is_instance_of::<PyFileNotFoundError>(py)
        && !error.is_instance_of::<PyFileExistsError>(py)
        && !error.is_instance_of::<PyNotADirectoryError>(py)
        && !error.is_instance_of::<PyIsADirectoryError>(py)
        && !has_errno(py, error, "ENOTEMPTY")
        && !["ENOTSUP", "EOPNOTSUPP", "ENOSYS"]
            .iter()
            .any(|name| has_errno(py, error, name))
}

fn is_directory(filesystem: &Bound<'_, PyAny>, path: &str) -> bool {
    filesystem
        .call_method1("get_file_info", (path,))
        .and_then(|info| file_info_from_py(&info))
        .is_ok_and(|info| info.kind == yggdryl::IOKind::Directory)
}

fn is_directory_error(path: &str) -> Error {
    Error::Io(std::io::Error::new(
        ErrorKind::IsADirectory,
        format!("expected a file at {:?}, got a directory", mask_uri(path)),
    ))
}

fn foreign_file_operation(
    py: Python<'_>,
    filesystem: &Bound<'_, PyAny>,
    error: &PyErr,
    operation: &'static str,
    path: &str,
    backend: &str,
) -> Error {
    // Some PyArrow backends surface a directory passed to a strict file
    // operation as PermissionError or a generic OSError (notably on Windows).
    // Classify only after the attempted operation failed. A failed stat, or
    // any non-directory answer, leaves the original failure untouched.
    if should_classify_file_error(py, error) && is_directory(filesystem, path) {
        return is_directory_error(path);
    }
    foreign(py, error, operation, path, backend)
}

/// Preserve a successful `PyArrow` `NativeFile`, but normalize the platform's
/// ambiguous directory failure for direct public opens.
pub(crate) fn direct_file_error(
    py: Python<'_>,
    filesystem: &Bound<'_, PyAny>,
    error: &PyErr,
    path: &str,
) -> PyErr {
    if should_classify_file_error(py, error) && is_directory(filesystem, path) {
        return PyIsADirectoryError::new_err(format!(
            "expected a file at {:?}, got a directory",
            mask_uri(path)
        ));
    }
    let message = safe_message(error);
    error
        .get_type(py)
        .call1((message.clone(),))
        .map_or_else(|_| PyOSError::new_err(message), PyErr::from_value)
}

/// Translate a core storage failure back to the closest built-in exception.
pub(crate) fn storage_error(error: Error) -> PyErr {
    let message = mask_uri(&error.to_string());
    match error {
        Error::Absent { .. } => PyFileNotFoundError::new_err(message),
        Error::Conflict { .. } => PyFileExistsError::new_err(message),
        Error::Unsupported { .. } => Python::attach(|py| {
            py.import("io")
                .and_then(|io| io.getattr("UnsupportedOperation"))
                .and_then(|class| class.call1((message.clone(),)))
                .map_or_else(
                    |_| PyNotImplementedError::new_err(message),
                    PyErr::from_value,
                )
        }),
        Error::Io(error) => match error.kind() {
            ErrorKind::NotFound => PyFileNotFoundError::new_err(message),
            ErrorKind::AlreadyExists => PyFileExistsError::new_err(message),
            ErrorKind::PermissionDenied => PyPermissionError::new_err(message),
            ErrorKind::NotADirectory => PyNotADirectoryError::new_err(message),
            ErrorKind::IsADirectory => PyIsADirectoryError::new_err(message),
            ErrorKind::DirectoryNotEmpty => Python::attach(|py| {
                let errno = py
                    .import("errno")
                    .and_then(|module| module.getattr("ENOTEMPTY"))
                    .and_then(|value| value.extract::<i32>())
                    .unwrap_or(39);
                PyOSError::new_err((errno, message))
            }),
            _ => PyOSError::new_err(message),
        },
        _ => PyValueError::new_err(message),
    }
}

fn file_info_from_py(info: &Bound<'_, PyAny>) -> PyResult<FileInfo> {
    let path: String = info.getattr("path")?.extract()?;
    let kind_name = info.getattr("type")?.getattr("name")?.extract::<String>()?;
    let size = info
        .getattr("size")?
        .extract::<Option<i64>>()?
        .and_then(|size| u64::try_from(size).ok());
    let mtime_ns = info.getattr("mtime_ns")?.extract::<Option<i64>>()?;
    match kind_name.as_str() {
        "File" => Ok(FileInfo::file(path, size, mtime_ns)),
        "Directory" => Ok(FileInfo::directory(path, mtime_ns)),
        "NotFound" => Ok(FileInfo::not_found(path)),
        other => Err(PyValueError::new_err(format!(
            "unsupported Arrow file type {other:?}"
        ))),
    }
}

fn metadata_dict<'py>(
    py: Python<'py>,
    metadata: Option<&OutputMetadata>,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let value = PyDict::new(py);
    for (key, item) in metadata.iter() {
        value.set_item(key, item)?;
    }
    Ok(Some(value))
}

#[derive(Clone)]
pub(crate) enum StickyFailure {
    Absent {
        expected: &'static str,
        path: String,
    },
    Conflict {
        expected: &'static str,
        actual: &'static str,
        path: String,
    },
    Unsupported {
        operation: &'static str,
        filesystem: String,
    },
    Io {
        kind: ErrorKind,
        message: String,
    },
    Other(String),
}

impl StickyFailure {
    pub(crate) fn new(error: Error) -> Self {
        match error {
            Error::Absent { expected, path } => Self::Absent {
                expected,
                path: path.to_string(),
            },
            Error::Conflict {
                expected,
                actual,
                path,
            } => Self::Conflict {
                expected,
                actual,
                path: path.to_string(),
            },
            Error::Unsupported {
                operation,
                filesystem,
            } => Self::Unsupported {
                operation,
                filesystem: filesystem.to_string(),
            },
            Error::Io(error) => Self::Io {
                kind: error.kind(),
                message: error.to_string(),
            },
            error => Self::Other(error.to_string()),
        }
    }

    pub(crate) fn error(&self) -> Error {
        match self {
            Self::Absent { expected, path } => Error::absent(expected, path),
            Self::Conflict {
                expected,
                actual,
                path,
            } => Error::conflict(expected, actual, path),
            Self::Unsupported {
                operation,
                filesystem,
            } => Error::unsupported(operation, filesystem),
            Self::Io { kind, message } => Error::Io(std::io::Error::new(*kind, message.clone())),
            Self::Other(message) => Error::Io(std::io::Error::other(message.clone())),
        }
    }
}

fn require_stream_open(
    close_attempted: bool,
    failure: Option<&StickyFailure>,
    kind: &'static str,
) -> Result<()> {
    if let Some(failure) = failure {
        return Err(failure.error());
    }
    if close_attempted {
        Err(Error::Io(std::io::Error::new(
            ErrorKind::BrokenPipe,
            format!("I/O operation on closed {kind}"),
        )))
    } else {
        Ok(())
    }
}

fn remember<T>(failure: &mut Option<StickyFailure>, result: Result<T>) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let sticky = StickyFailure::new(error);
            let returned = sticky.error();
            *failure = Some(sticky);
            Err(returned)
        }
    }
}

fn close_python_stream(
    stream: &Py<PyAny>,
    close_attempted: &mut bool,
    failure: &mut Option<StickyFailure>,
) -> Result<()> {
    if *close_attempted {
        return failure
            .as_ref()
            .map_or(Ok(()), |failure| Err(failure.error()));
    }
    *close_attempted = true;
    let prior = failure.clone();
    let result = Python::attach(|py| {
        stream
            .bind(py)
            .call_method0("close")
            .map(|_| ())
            .map_err(|error| foreign(py, &error, "close", "<stream>", "pyarrow"))
    });
    if let Some(prior) = prior {
        return Err(prior.error());
    }
    remember(failure, result)
}

struct PyInputStream {
    stream: Py<PyAny>,
    position: u64,
    close_attempted: bool,
    failure: Option<StickyFailure>,
}

impl PyInputStream {
    fn new(stream: Bound<'_, PyAny>) -> Self {
        Self {
            stream: stream.unbind(),
            position: 0,
            close_attempted: false,
            failure: None,
        }
    }

    fn read_inner(&mut self, buffer: &mut [u8]) -> Result<usize> {
        require_stream_open(self.close_attempted, self.failure.as_ref(), "input stream")?;
        let result = Python::attach(|py| {
            let value = self
                .stream
                .bind(py)
                .call_method1("read", (buffer.len(),))
                .map_err(|error| foreign(py, &error, "read", "<stream>", "pyarrow"))?;
            let bytes = value
                .extract::<Vec<u8>>()
                .map_err(|error| foreign(py, &error, "read", "<stream>", "pyarrow"))?;
            let count = bytes.len().min(buffer.len());
            buffer[..count].copy_from_slice(&bytes[..count]);
            self.position = self.position.saturating_add(count as u64);
            Ok(count)
        });
        remember(&mut self.failure, result)
    }
}

impl ByteReader for PyInputStream {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.read_inner(buffer)
    }

    fn tell(&self) -> u64 {
        self.position
    }

    fn close(&mut self) -> Result<()> {
        close_python_stream(&self.stream, &mut self.close_attempted, &mut self.failure)
    }

    fn closed(&self) -> bool {
        self.close_attempted
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

struct PyRandomAccessFile {
    stream: Py<PyAny>,
    position: u64,
    close_attempted: bool,
    failure: Option<StickyFailure>,
}

impl PyRandomAccessFile {
    fn new(stream: Bound<'_, PyAny>) -> Self {
        Self {
            stream: stream.unbind(),
            position: 0,
            close_attempted: false,
            failure: None,
        }
    }

    fn require_open(&self) -> Result<()> {
        require_stream_open(self.close_attempted, self.failure.as_ref(), "input file")
    }
}

impl ByteReader for PyRandomAccessFile {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.require_open()?;
        let result = Python::attach(|py| {
            let value = self
                .stream
                .bind(py)
                .call_method1("read", (buffer.len(),))
                .map_err(|error| foreign(py, &error, "read", "<stream>", "pyarrow"))?;
            let bytes = value
                .extract::<Vec<u8>>()
                .map_err(|error| foreign(py, &error, "read", "<stream>", "pyarrow"))?;
            let count = bytes.len().min(buffer.len());
            buffer[..count].copy_from_slice(&bytes[..count]);
            self.position = self.position.saturating_add(count as u64);
            Ok(count)
        });
        remember(&mut self.failure, result)
    }

    fn tell(&self) -> u64 {
        self.position
    }

    fn close(&mut self) -> Result<()> {
        close_python_stream(&self.stream, &mut self.close_attempted, &mut self.failure)
    }

    fn closed(&self) -> bool {
        self.close_attempted
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl RandomAccessReader for PyRandomAccessFile {
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.require_open()?;
        let result = Python::attach(|py| {
            let value = self
                .stream
                .bind(py)
                .call_method1("read_at", (buffer.len(), offset))
                .map_err(|error| foreign(py, &error, "read_at", "<stream>", "pyarrow"))?;
            let bytes = value
                .extract::<Vec<u8>>()
                .map_err(|error| foreign(py, &error, "read_at", "<stream>", "pyarrow"))?;
            let count = bytes.len().min(buffer.len());
            buffer[..count].copy_from_slice(&bytes[..count]);
            Ok(count)
        });
        remember(&mut self.failure, result)
    }

    fn seek(&mut self, from: SeekFrom) -> Result<u64> {
        self.require_open()?;
        let (offset, whence) = match from {
            SeekFrom::Start(offset) => (i128::from(offset), 0),
            SeekFrom::Current(offset) => (i128::from(offset), 1),
            SeekFrom::End(offset) => (i128::from(offset), 2),
        };
        let offset = i64::try_from(offset).map_err(|_| {
            Error::Io(std::io::Error::new(
                ErrorKind::InvalidInput,
                "seek offset exceeds i64",
            ))
        })?;
        let result = Python::attach(|py| {
            let stream = self.stream.bind(py);
            stream
                .call_method1("seek", (offset, whence))
                .and_then(|_| stream.call_method0("tell"))
                .and_then(|value| value.extract::<u64>())
                .map_err(|error| foreign(py, &error, "seek", "<stream>", "pyarrow"))
        });
        let position = remember(&mut self.failure, result)?;
        self.position = position;
        Ok(position)
    }

    fn into_random_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

struct PyOutputStream {
    stream: Py<PyAny>,
    position: u64,
    close_attempted: bool,
    failure: Option<StickyFailure>,
}

impl PyOutputStream {
    fn new(stream: Bound<'_, PyAny>, position: u64) -> Self {
        Self {
            stream: stream.unbind(),
            position,
            close_attempted: false,
            failure: None,
        }
    }

    fn require_open(&self) -> Result<()> {
        require_stream_open(self.close_attempted, self.failure.as_ref(), "output stream")
    }
}

impl ByteWriter for PyOutputStream {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.require_open()?;
        let result = Python::attach(|py| {
            self.stream
                .bind(py)
                .call_method1("write", (PyBytes::new(py, bytes),))
                .and_then(|value| value.extract::<usize>())
                .map_err(|error| foreign(py, &error, "write", "<stream>", "pyarrow"))
        });
        let written = remember(&mut self.failure, result)?;
        self.position = self.position.saturating_add(written as u64);
        Ok(written)
    }

    fn tell(&self) -> u64 {
        self.position
    }

    fn flush(&mut self) -> Result<()> {
        self.require_open()?;
        let result = Python::attach(|py| {
            self.stream
                .bind(py)
                .call_method0("flush")
                .map(|_| ())
                .map_err(|error| foreign(py, &error, "flush", "<stream>", "pyarrow"))
        });
        remember(&mut self.failure, result)
    }

    fn close(&mut self) -> Result<()> {
        close_python_stream(&self.stream, &mut self.close_attempted, &mut self.failure)
    }

    fn closed(&self) -> bool {
        self.close_attempted
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl FileSystem for PyFileSystem {
    fn type_name(&self) -> &str {
        &self.name
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        self.try_equals(other).unwrap_or(false)
    }

    fn try_equals(&self, other: &dyn FileSystem) -> Result<bool> {
        let Some(other) = other.as_any().downcast_ref::<Self>() else {
            return Ok(false);
        };
        self.with_gil("equals", "<filesystem>", |filesystem| {
            Python::attach(|py| {
                if self.filesystem.bind(py).is(other.filesystem.bind(py)) {
                    return Ok(true);
                }
                filesystem
                    .call_method1("equals", (other.filesystem.bind(py),))
                    .and_then(|value| value.extract::<bool>())
            })
        })
    }

    fn normalize_path(&self, path: &str) -> Result<String> {
        self.with_gil("normalize_path", path, |filesystem| {
            filesystem
                .call_method1("normalize_path", (path,))?
                .extract()
        })
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.with_gil("get_file_info", path, |filesystem| {
            let info = filesystem.call_method1("get_file_info", (path,))?;
            file_info_from_py(&info)
        })
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        let entries: Result<Vec<FileInfo>> =
            self.with_gil("get_file_info", &selector.base_dir, |filesystem| {
                let py = filesystem.py();
                let class = py.import("pyarrow.fs")?.getattr("FileSelector")?;
                let kwargs = PyDict::new(py);
                kwargs.set_item("base_dir", &selector.base_dir)?;
                kwargs.set_item("recursive", selector.recursive)?;
                kwargs.set_item("allow_not_found", selector.allow_not_found)?;
                let selector = class.call((), Some(&kwargs))?;
                let infos = filesystem.call_method1("get_file_info", (selector,))?;
                let mut entries = infos
                    .try_iter()?
                    .map(|entry| file_info_from_py(&entry?))
                    .collect::<PyResult<Vec<_>>>()?;
                entries.sort_by(|left, right| left.path.cmp(&right.path));
                Ok(entries)
            });
        match entries {
            Ok(entries) => FileInfos::new(entries.into_iter().map(Ok)),
            Err(error) => FileInfos::failing(error),
        }
    }

    fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
        self.with_gil("create_dir", path, |filesystem| {
            let kwargs = PyDict::new(filesystem.py());
            kwargs.set_item("recursive", recursive)?;
            filesystem.call_method("create_dir", (path,), Some(&kwargs))?;
            Ok(())
        })
    }

    fn delete_dir(&self, path: &str) -> Result<()> {
        // PyArrow exposes recursive `delete_dir` only. Listing first cannot
        // make it an atomic empty-directory delete: a concurrent child could
        // otherwise be erased between the list and mutation.
        let _ = path;
        Err(Error::unsupported(
            "non-recursive directory deletion",
            self.type_name(),
        ))
    }

    fn delete_dir_recursive(&self, path: &str) -> Result<()> {
        self.with_gil_directory("delete_dir", path, |filesystem| {
            filesystem.call_method1("delete_dir", (path,))?;
            Ok(())
        })
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
        self.with_gil_directory("delete_dir_contents", path, |filesystem| {
            let kwargs = PyDict::new(filesystem.py());
            kwargs.set_item("missing_dir_ok", missing_dir_ok)?;
            filesystem.call_method("delete_dir_contents", (path,), Some(&kwargs))?;
            Ok(())
        })
    }

    fn delete_root_dir_contents(&self) -> Result<()> {
        self.with_gil("delete_root_dir_contents", "<root>", |filesystem| {
            let kwargs = PyDict::new(filesystem.py());
            kwargs.set_item("accept_root_dir", true)?;
            filesystem.call_method("delete_dir_contents", ("",), Some(&kwargs))?;
            Ok(())
        })
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        self.with_gil_file("delete_file", path, |filesystem| {
            filesystem.call_method1("delete_file", (path,))?;
            Ok(())
        })
    }

    fn copy_file(&self, source: &str, target: &str) -> Result<()> {
        self.with_gil_files("copy_file", source, target, |filesystem| {
            filesystem.call_method1("copy_file", (source, target))?;
            Ok(())
        })
    }

    fn move_file(&self, source: &str, target: &str) -> Result<()> {
        self.with_gil_files("move", source, target, |filesystem| {
            filesystem.call_method1("move", (source, target))?;
            Ok(())
        })
    }

    fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
        self.with_gil_file("open_input_file", path, |filesystem| {
            let stream = filesystem.call_method1("open_input_file", (path,))?;
            Ok(PyRandomAccessFile::new(stream))
        })
        .map(|stream| Box::new(stream) as Box<dyn RandomAccessReader>)
    }

    fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
        self.with_gil_file("open_input_stream", path, |filesystem| {
            let kwargs = PyDict::new(filesystem.py());
            kwargs.set_item("compression", filesystem.py().None())?;
            let stream = filesystem.call_method("open_input_stream", (path,), Some(&kwargs))?;
            Ok(PyInputStream::new(stream))
        })
        .map(|stream| Box::new(stream) as Box<dyn ByteReader>)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.with_gil_file("open_output_stream", path, |filesystem| {
            let py = filesystem.py();
            let kwargs = PyDict::new(py);
            kwargs.set_item("compression", py.None())?;
            if let Some(metadata) = metadata_dict(py, metadata)? {
                kwargs.set_item("metadata", metadata)?;
            }
            let stream = filesystem.call_method("open_output_stream", (path,), Some(&kwargs))?;
            Ok(PyOutputStream::new(stream, 0))
        })
        .map(|stream| Box::new(stream) as Box<dyn ByteWriter>)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.with_gil_file("open_append_stream", path, |filesystem| {
            let py = filesystem.py();
            let kwargs = PyDict::new(py);
            kwargs.set_item("compression", py.None())?;
            if let Some(metadata) = metadata_dict(py, metadata)? {
                kwargs.set_item("metadata", metadata)?;
            }
            let stream = filesystem.call_method("open_append_stream", (path,), Some(&kwargs))?;
            let position = match stream
                .call_method0("tell")
                .and_then(|value| value.extract())
            {
                Ok(position) => position,
                Err(error) => {
                    let _ = stream.call_method0("close");
                    return Err(error);
                }
            };
            Ok(PyOutputStream::new(stream, position))
        })
        .map(|stream| Box::new(stream) as Box<dyn ByteWriter>)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Return whether `value` is any `pyarrow.fs.FileSystem` implementation.
pub(crate) fn is_arrow_filesystem(value: &Bound<'_, PyAny>) -> PyResult<bool> {
    let class = value.py().import("pyarrow.fs")?.getattr("FileSystem")?;
    value.is_instance(&class)
}
