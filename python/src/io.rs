//! `IOBase`, exposed to Python with the method names `pathlib` already uses.
//!
//! The core trait is positional and fully random-access, so there are no modes
//! to open with and no cursor to keep: `read_bytes`, `write_bytes`, `iterdir`,
//! `glob`, `mkdir`, `touch`, and `unlink` all mean here what they mean on a
//! `pathlib.Path`, and each one is answered by the core implementation for the
//! backend the location names. Code written against `pathlib` therefore runs
//! against a local directory, and the same code will run against a bucket when
//! that backend lands, because only the handle changes.

use pyo3::exceptions::{PyIsADirectoryError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple, PyType};

use yggdryl::generic::{Holder, RecordOptions};
use yggdryl::io::IOBase as _;
use yggdryl::io::LineRecordOptions;

use crate::field::PyField;
use crate::record::{
    Frames, PyRecordOptions, batch_reader_from_any, batch_reader_from_value,
    batch_reader_to_pyarrow, core_record_options_from_value, frame_batch_reader, frame_from_reader,
    frames_batch_reader, frames_from_reader,
};
use crate::uri::{PyUrl, core_url_from_value};
use crate::value_error;

/// A random-access resource: a local file, a directory, or a memory buffer.
#[pyclass(name = "IOBase", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyIOBase {
    inner: Holder,
}

/// Rebuild a foreign-filesystem handle, keeping the filesystem it stands on.
///
/// `None` for anything else, so the local rebuild stays the default path.
fn rebuilt_arrow_holder(inner: &Holder) -> Option<Holder> {
    match inner {
        Holder::ArrowFolder(folder) => Some(Holder::ArrowFolder(folder.clone())),
        Holder::ArrowFile(file) => Some(Holder::ArrowFile(yggdryl::arrowfs::File::new(
            file.filesystem().clone(),
            file.url().clone(),
        ))),
        Holder::ArrowPath(path) => Some(Holder::ArrowPath(yggdryl::arrowfs::Path::new(
            path.filesystem().clone(),
            path.url().clone(),
        ))),
        _ => None,
    }
}

/// Address a foreign-filesystem handle's location as a container.
fn arrow_folder_holder(inner: &Holder) -> Option<Holder> {
    let folder = match inner {
        Holder::ArrowFolder(folder) => folder.clone(),
        Holder::ArrowFile(file) => {
            yggdryl::arrowfs::Folder::new(file.filesystem().clone(), file.url().clone())
        }
        Holder::ArrowPath(path) => {
            yggdryl::arrowfs::Folder::new(path.filesystem().clone(), path.url().clone())
        }
        _ => return None,
    };
    Some(Holder::ArrowFolder(folder))
}

impl PyIOBase {
    pub(crate) fn from_core(inner: Holder) -> Self {
        Self { inner }
    }

    /// Describe a local location, as whichever role fits what is there.
    ///
    /// The specialized roles are what a record caller needs: a leaf reports the
    /// media type its name implies, which is where the encoding comes from,
    /// while the generic location reports only that it is a file. A handle for
    /// a location that does not exist yet is therefore a leaf.
    fn located(path: &std::path::Path) -> PyResult<Self> {
        Holder::local(path)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Build a second handle on the same location.
    ///
    /// A handle owns backend state - a mapping, an open descriptor, a staged
    /// value - so it is not copied; the location it describes is what gets
    /// rebuilt. A handle on a foreign Arrow filesystem rebuilds onto that
    /// same filesystem, because its location alone would not say where it
    /// lives.
    fn rebuilt(&self) -> PyResult<Self> {
        if let Some(holder) = rebuilt_arrow_holder(&self.inner) {
            return Ok(Self::from_core(holder));
        }
        let url = self.inner.url().ok_or_else(|| {
            PyValueError::new_err("an in-memory resource has no location to rebuild from")
        })?;
        Self::located(&url.to_path().map_err(value_error)?)
    }

    /// Build a container handle on the same location.
    ///
    /// A table is a folder, so a caller who names one that does not exist yet
    /// still gets a handle that can resolve children; [`Holder::local`] would
    /// have decided it was a file, because nothing is there to look at. A
    /// foreign filesystem's handle becomes a container on that filesystem, so
    /// a table reached this way never learns which backend it stands on.
    pub(crate) fn folder_holder(&self) -> PyResult<Holder> {
        if let Some(holder) = arrow_folder_holder(&self.inner) {
            return Ok(holder);
        }
        let url = self
            .inner
            .url()
            .ok_or_else(|| PyValueError::new_err("an in-memory resource is not a container"))?;
        Holder::folder(url.to_path().map_err(value_error)?).map_err(value_error)
    }

    /// Build a handle on `path` over a held `pyarrow.fs.FileSystem`.
    fn over_arrow_fs(filesystem: &Bound<'_, PyAny>, path: &Bound<'_, PyAny>) -> PyResult<Self> {
        let location = crate::uri::path_string_from_value(path)?;
        let backend: std::sync::Arc<dyn yggdryl::arrowfs::ArrowFileSystem> =
            std::sync::Arc::new(crate::arrowfs::PyArrowFileSystem::new(filesystem)?);
        let url =
            yggdryl::arrowfs::location_url(backend.as_ref(), &location).map_err(value_error)?;
        Ok(Self::from_core(yggdryl::arrowfs::located(backend, url)))
    }

    /// Resolve the options a record call runs under.
    ///
    /// Omitting them is the normal case: the encoding then comes from the
    /// handle's own media type, so it is never guessed.
    fn resolve_options(&self, options: Option<&Bound<'_, PyAny>>) -> PyResult<RecordOptions> {
        match options {
            Some(options) => core_record_options_from_value(options),
            None => self.inner.record_options().map_err(value_error),
        }
    }

    /// Name the path and base type a native scanner can open directly,
    /// with the bytes published at their exact length first.
    ///
    /// `None` means the resource is not one a foreign scanner mmaps - it
    /// lives in memory, carries content codings, or is an Arrow *stream*,
    /// which the dataset scanners do not read - and the caller streams
    /// through the native reader instead. When a target exists the handle is
    /// closed first: closing publishes the bytes at their exact length,
    /// which is what handing the path to another reader means.
    fn published_scan_target(&mut self) -> PyResult<Option<(String, yggdryl::MimeType)>> {
        let media_type = self.inner.media_type();
        if !media_type.encodings().is_empty() {
            return Ok(None);
        }
        let base = media_type.base().clone();
        // Parquet is the one format both foreign scanners mmap today: the
        // IPC media writes stream framing whichever suffix names it, and a
        // stream is read through the native reader, not off the file.
        if base != yggdryl::MimeType::PARQUET {
            return Ok(None);
        }
        let Some(url) = self.inner.url() else {
            return Ok(None);
        };
        let Ok(path) = url.to_path() else {
            return Ok(None);
        };
        let path = path.to_string_lossy().into_owned();
        self.inner.close().map_err(value_error)?;
        Ok(Some((path, base)))
    }

    /// Turn an iterable of record instances into one streamed batch reader.
    ///
    /// The first row is peeked to infer the class when none was named, then
    /// chained back in front, so the iterable is pulled exactly once. `None`
    /// means there was nothing to write and no class to write it with.
    fn records_reader<'py>(
        &self,
        py: Python<'py>,
        rows: &Bound<'py, PyAny>,
        cls: Option<&Bound<'py, PyAny>>,
        options: Option<&Bound<'py, PyAny>>,
        safe: bool,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        use pyo3::types::PyList;

        let mut iterator = rows.try_iter()?;
        let first = iterator.next().transpose()?;
        let cls = match (cls, &first) {
            (Some(cls), _) => cls.clone(),
            (None, Some(first)) => first.get_type().into_any(),
            (None, None) => return Ok(None),
        };
        let head = match &first {
            Some(first) => PyList::new(py, [first])?,
            None => PyList::empty(py),
        };
        let chained = py
            .import("itertools")?
            .getattr("chain")?
            .call1((head, iterator))?;
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("safe", safe)?;
        if let Some(batch_size) = {
            use yggdryl::generic::IORecordOptions;
            self.resolve_options(options)?.batch_size()
        } {
            kwargs.set_item("batch_size", batch_size)?;
        }
        cls.call_method("into_arrow_record_batch_reader", (chained,), Some(&kwargs))
            .map(Some)
    }
}

#[pymethods]
impl PyIOBase {
    /// Describe a resource by whatever already names or holds it.
    ///
    /// Accepts anything that names a location - a string, a `pathlib.Path`, a
    /// [`Url`][crate::uri::PyUrl], or another handle - and, because callers
    /// hold open files more often than paths, anything file-like too. A
    /// file-like object with a real filesystem name captures the *location*;
    /// a nameless stream - `io.BytesIO`, a socket wrapper, a decompressor -
    /// captures its *content*. An in-memory handle likewise captures content,
    /// media type included. For named locations nothing is opened, created,
    /// or read here, per the laziness contract.
    ///
    /// A `pyarrow.fs.FileSystem` as the first argument names the *backend*
    /// rather than the location, so the second says where on it:
    /// `IOBase(S3FileSystem(region=...), "bucket/key.parquet")`. Every
    /// filesystem PyArrow ships is accepted, as is a custom one wrapped in
    /// `PyFileSystem(FileSystemHandler)`, and what comes back is this same
    /// class - nothing filesystem-specific leaks into the surface.
    #[new]
    #[pyo3(signature = (value, path = None))]
    fn new(value: &Bound<'_, PyAny>, path: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        if crate::arrowfs::is_arrow_filesystem(value) {
            let path = path.ok_or_else(|| {
                PyValueError::new_err(
                    "expected a path on the filesystem as the second argument, got none",
                )
            })?;
            return Self::over_arrow_fs(value, path);
        }
        if let Some(path) = path {
            return Err(PyValueError::new_err(format!(
                "expected a pyarrow.fs.FileSystem to resolve {} against, got {}",
                path.repr()
                    .map(|text| text.to_string())
                    .unwrap_or_else(|_| "a second argument".to_owned()),
                value.get_type().name()?,
            )));
        }
        if let Ok(handle) = value.extract::<PyRef<'_, Self>>() {
            if handle.inner.kind() != yggdryl::IOKind::Memory {
                return handle.rebuilt();
            }
            // No location to rebuild from, so the content is what is taken.
            let bytes = handle.inner.read_all().map_err(value_error)?;
            let mut buffer = Holder::Buffer(yggdryl::io::Buffer::from_bytes(bytes));
            buffer.set_media_type(handle.inner.media_type().clone());
            return Ok(Self::from_core(buffer));
        }
        if value.hasattr("read")? && !value.is_instance_of::<pyo3::types::PyString>() {
            // An open file knows where it lives; `name` is an `int` for a
            // descriptor-opened one and absent on a plain stream, and neither
            // of those names a place.
            if let Ok(name) = value.getattr("name") {
                if name.is_instance_of::<pyo3::types::PyString>() {
                    return Self::located(&name.extract::<std::path::PathBuf>()?);
                }
            }
            let content = value.call_method0("read")?;
            let bytes = if let Ok(bytes) = content.extract::<Vec<u8>>() {
                bytes
            } else if let Ok(text) = content.extract::<String>() {
                text.into_bytes()
            } else {
                return Err(PyValueError::new_err(
                    "a file-like resource must read bytes or text",
                ));
            };
            return Ok(Self::from_core(Holder::Buffer(
                yggdryl::io::Buffer::from_bytes(bytes),
            )));
        }
        let url = core_url_from_value(value)?;
        Self::located(&url.to_path().map_err(value_error)?)
    }

    /// Describe a resource on any `pyarrow.fs.FileSystem`.
    ///
    /// This is the explicit spelling of what the constructor infers, and it
    /// is the whole surface a foreign filesystem needs: `S3FileSystem`,
    /// `GcsFileSystem`, `AzureFileSystem`, `LocalFileSystem`,
    /// `SubTreeFileSystem`, and a custom filesystem wrapped in
    /// `PyFileSystem(FileSystemHandler)` - which is also how `fsspec` arrives
    /// - all reach the same seven-method contract, so none of them needs code
    /// of its own here.
    ///
    /// ```python
    /// handle = IOBase.from_arrow_fs(S3FileSystem(region="eu-west-1"), "bucket/key.parquet")
    /// reader = handle.read_arrow_batch_reader()
    /// ```
    ///
    /// The result is an ordinary handle: `iterdir`, `glob`, `/`, and `parent`
    /// return handles that still carry the filesystem, and the three record
    /// methods work exactly as they do on a local file. Per the laziness
    /// contract nothing is opened, created, or read here.
    ///
    /// A write publishes when the handle closes, because an Arrow filesystem
    /// replaces whole files rather than writing ranges - so a file another
    /// reader will open is written inside a `with` block.
    #[classmethod]
    fn from_arrow_fs(
        _cls: &Bound<'_, PyType>,
        filesystem: &Bound<'_, PyAny>,
        path: &Bound<'_, PyAny>,
    ) -> PyResult<Self> {
        if !crate::arrowfs::is_arrow_filesystem(filesystem) {
            return Err(PyValueError::new_err(format!(
                "expected a pyarrow.fs.FileSystem, got {}",
                filesystem.get_type().name()?,
            )));
        }
        Self::over_arrow_fs(filesystem, path)
    }

    /// Describe an in-memory resource holding `data`.
    #[classmethod]
    #[pyo3(signature = (data = None))]
    fn from_bytes(_cls: &Bound<'_, PyType>, data: Option<Vec<u8>>) -> Self {
        Self::from_core(Holder::Buffer(yggdryl::io::Buffer::from_bytes(
            data.unwrap_or_default(),
        )))
    }

    /// The location this handle addresses.
    #[getter]
    fn url(&self) -> Option<PyUrl> {
        self.inner.url().cloned().map(PyUrl::from_core)
    }

    /// The final path component, as `pathlib.PurePath.name`.
    #[getter]
    fn name(&self) -> String {
        self.inner
            .url()
            .and_then(|url| url.file_name())
            .unwrap_or_default()
            .to_owned()
    }

    /// The media type of the bytes here.
    #[getter]
    fn media_type(&self) -> crate::media::PyMediaType {
        crate::media::PyMediaType::from_core(self.inner.media_type().clone())
    }

    /// Declare what the bytes here are, as a media type, MIME type, or string.
    ///
    /// A located resource infers this from its name, so the setter is what an
    /// in-memory buffer uses to say which record encoding it holds.
    #[setter]
    fn set_media_type(&mut self, media_type: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .set_media_type(crate::media::core_media_type_from_value(media_type)?);
        Ok(())
    }

    /// The number of bytes here, as `Path.stat().st_size`.
    #[getter]
    fn size(&self) -> u64 {
        self.inner.size()
    }

    /// The containing resource, as `PurePath.parent`.
    #[getter]
    fn parent(&self) -> Option<Self> {
        self.inner.parent().map(Self::from_core)
    }

    /// Resolve a child of this resource, as `PurePath.joinpath`.
    #[pyo3(signature = (*others))]
    fn joinpath(&self, others: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut resolved: Option<Holder> = None;
        for other in others {
            let name = crate::uri::path_string_from_value(&other)?;
            let base = resolved.as_ref().unwrap_or(&self.inner);
            resolved = Some(base.child_by(&name).map_err(value_error)?);
        }
        match resolved {
            Some(handle) => Ok(Self::from_core(handle)),
            // `joinpath()` with nothing to join is the same location.
            None => self.rebuilt(),
        }
    }

    /// `handle / "child"`, as `PurePath.__truediv__`.
    fn __truediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.inner
            .child_by(&crate::uri::path_string_from_value(other)?)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Return whether anything is here now, as `Path.exists`.
    fn exists(&self) -> bool {
        self.inner.kind() != yggdryl::IOKind::Unknown
    }

    /// Return whether this resource contains others, as `Path.is_dir`.
    fn is_dir(&self) -> bool {
        self.inner.is_container()
    }

    /// Return whether this resource holds bytes, as `Path.is_file`.
    fn is_file(&self) -> bool {
        self.inner.kind() == yggdryl::IOKind::File
    }

    /// Iterate the immediate children, as `Path.iterdir`.
    ///
    /// Private entries - names beginning with a dot - are skipped unless
    /// `include_private` asks for them.
    #[pyo3(signature = (include_private = false))]
    fn iterdir(&self, include_private: bool) -> PyResult<Vec<Self>> {
        Ok(self
            .inner
            .ls(false, include_private)
            .map_err(value_error)?
            .into_iter()
            .map(Self::from_core)
            .collect())
    }

    /// List the children, optionally descending, as the core `ls`.
    #[pyo3(signature = (recursive = false, include_private = false))]
    fn ls(&self, recursive: bool, include_private: bool) -> PyResult<Vec<Self>> {
        Ok(self
            .inner
            .ls(recursive, include_private)
            .map_err(value_error)?
            .into_iter()
            .map(Self::from_core)
            .collect())
    }

    /// Expand a glob against this resource, as `Path.glob`.
    #[pyo3(signature = (pattern, include_private = false))]
    fn glob(&self, pattern: &str, include_private: bool) -> PyResult<Vec<Self>> {
        Ok(self
            .inner
            .glob(pattern, include_private)
            .map_err(value_error)?
            .into_iter()
            .map(Self::from_core)
            .collect())
    }

    /// Expand a glob at any depth, as `Path.rglob`.
    #[pyo3(signature = (pattern, include_private = false))]
    fn rglob(&self, pattern: &str, include_private: bool) -> PyResult<Vec<Self>> {
        self.glob(&format!("**/{pattern}"), include_private)
    }

    /// The Hive partition pairs this resource's location spells out.
    #[getter]
    fn partitions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.inner.partitions())
    }

    /// Iterate the leaves beneath this one carrying every given partition.
    ///
    /// `filters` is a mapping or a sequence of pairs, so a partitioned write
    /// selects the parts it has to touch and leaves the rest alone.
    #[pyo3(signature = (filters, include_private = false))]
    fn children_where(
        &self,
        filters: &Bound<'_, PyAny>,
        include_private: bool,
    ) -> PyResult<Vec<Self>> {
        let pairs: Vec<(String, String)> =
            if let Ok(mapping) = filters.cast::<pyo3::types::PyDict>() {
                mapping
                    .iter()
                    .map(|(key, value)| Ok((key.extract::<String>()?, value.extract::<String>()?)))
                    .collect::<PyResult<Vec<_>>>()?
            } else {
                filters.extract()?
            };
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        Ok(self
            .inner
            .children_where(&borrowed, include_private)
            .map_err(value_error)?
            .map(Self::from_core)
            .collect())
    }

    /// Read every byte here, as `Path.read_bytes`.
    ///
    /// A resource that does not exist reads as empty rather than raising, per
    /// the laziness contract.
    fn read_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self.inner.read_all().map_err(value_error)?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Read every byte here as text, as `Path.read_text`.
    fn read_text(&self) -> PyResult<String> {
        let bytes = self.inner.read_all().map_err(value_error)?;
        String::from_utf8(bytes).map_err(|error| PyValueError::new_err(error.to_string()))
    }

    /// Replace what is here with `data`, as `Path.write_bytes`.
    fn write_bytes(&mut self, data: &[u8]) -> PyResult<usize> {
        self.inner.write_all_bytes(data).map_err(value_error)?;
        Ok(data.len())
    }

    /// Replace what is here with `text`, as `Path.write_text`.
    fn write_text(&mut self, text: &str) -> PyResult<usize> {
        self.write_bytes(text.as_bytes())
    }

    /// Read `length` bytes from `offset`, which `pathlib` cannot do.
    fn pread<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self.inner.read_range(offset, length).map_err(value_error)?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Write `data` at `offset`, growing and zero-filling as needed.
    fn pwrite(&mut self, offset: u64, data: &[u8]) -> PyResult<usize> {
        self.inner.pwrite(offset, data).map_err(value_error)
    }

    /// Append `data` after the last byte, returning the offset it landed at.
    fn append(&mut self, data: &[u8]) -> PyResult<u64> {
        self.inner.append(data).map_err(value_error)
    }

    /// Create this resource as a container, as `Path.mkdir`.
    ///
    /// Parents are created too, and an existing container is left alone, which
    /// is `mkdir(parents=True, exist_ok=True)`. An undecided location is what
    /// this decides: it becomes a container, and the handle keeps working as
    /// one afterwards - a plain byte write would have made it a file instead.
    fn mkdir(&mut self) -> PyResult<()> {
        let url = self.inner.url().ok_or_else(|| {
            PyValueError::new_err("an in-memory resource cannot become a directory")
        })?;
        let mut folder =
            Holder::folder(url.to_path().map_err(value_error)?).map_err(value_error)?;
        folder.truncate(0).map_err(value_error)?;
        self.inner = folder;
        Ok(())
    }

    /// Create this resource as an empty leaf, as `Path.touch`.
    ///
    /// An existing leaf keeps its bytes, as `touch` does.
    fn touch(&mut self) -> PyResult<()> {
        if self.inner.is_container() {
            return Err(PyIsADirectoryError::new_err(format!(
                "expected a file to touch, got the directory {}",
                self.name()
            )));
        }
        if self.exists() {
            return Ok(());
        }
        self.inner.write_all_bytes(b"").map_err(value_error)
    }

    /// Remove the bytes here, as `Path.unlink` on a leaf.
    fn unlink(&mut self) -> PyResult<()> {
        self.inner.clear().map_err(value_error)
    }

    /// Cut this resource to `size` bytes.
    fn truncate(&mut self, size: u64) -> PyResult<()> {
        self.inner.truncate(size).map_err(value_error)
    }

    /// Flush anything buffered, as `IOBase.flush`.
    fn flush(&mut self) -> PyResult<()> {
        self.inner.flush().map_err(value_error)
    }

    /// Materialize the resource and cache what repeated calls would re-derive.
    ///
    /// A handle works without this - every operation materializes what it needs
    /// - so calling it moves that cost to a known point. Opening a resource
    /// that does not exist yet succeeds without creating it.
    fn open(&mut self) -> PyResult<()> {
        self.inner.open().map_err(value_error)
    }

    /// Return whether cached state is currently held.
    fn is_open(&self) -> bool {
        self.inner.is_open()
    }

    /// Publish and release everything `open` cached.
    ///
    /// The handle stays usable afterwards; a later operation re-materializes.
    /// This is what publishes a written file at its exact length, which is why
    /// a `with` block is how a file meant for another reader is written.
    fn close(&mut self) -> PyResult<()> {
        self.inner.close().map_err(value_error)
    }

    /// Enter a scope, as `IOBase.open`.
    fn __enter__(mut slf: PyRefMut<'_, Self>) -> PyResult<PyRefMut<'_, Self>> {
        slf.inner.open().map_err(value_error)?;
        Ok(slf)
    }

    /// Leave a scope, as `IOBase.close`.
    ///
    /// The resource is published even when the block failed, and the exception
    /// is never swallowed.
    #[pyo3(signature = (exception_type = None, exception = None, traceback = None))]
    fn __exit__(
        &mut self,
        exception_type: Option<&Bound<'_, PyAny>>,
        exception: Option<&Bound<'_, PyAny>>,
        traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let _ = (exception_type, exception, traceback);
        self.close()?;
        Ok(false)
    }

    /// Copy every byte here into `target`, returning the count.
    fn copy_into(&self, target: &mut Self) -> PyResult<u64> {
        self.inner.copy_into(&mut target.inner).map_err(value_error)
    }

    /// A positioned view over this resource, as Python files position them.
    ///
    /// The cursor shares this handle - a write through the cursor is a write
    /// here - and owns only its position: `read`/`write` advance it, `seek`
    /// and `tell` move and report it, and two cursors advance independently.
    #[pyo3(signature = (position = 0))]
    fn cursor(slf: &Bound<'_, Self>, position: u64) -> PyIOCursor {
        PyIOCursor {
            handle: slf.clone().unbind(),
            position: std::sync::atomic::AtomicU64::new(position),
        }
    }

    /// The record settings for the encoding this handle's media type names.
    ///
    /// Every record method below defaults to exactly this, so the encoding is
    /// never guessed and never passed as a format argument.
    fn record_options(&self) -> PyResult<PyRecordOptions> {
        self.inner
            .record_options()
            .map(PyRecordOptions::from_core)
            .map_err(value_error)
    }

    /// Iterate the resource's decoded text lines, one line at a time.
    ///
    /// Any content codings the resource's name declares - `trades.jsonl.gz`,
    /// `log.txt.zst` - are decoded as streams, so a compressed resource is
    /// read without ever holding its decompressed value. Lines are what
    /// `\n` ends, a trailing `\r` belongs to the terminator, and the last
    /// line needs no terminator, exactly as `str.splitlines` treats them.
    /// With `pattern`, lines group into records: one starts at a matching
    /// line and carries every following line until the next match, the shape
    /// of a log whose entries open with a timestamp.
    #[pyo3(signature = (pattern = None))]
    fn read_lines(&self, pattern: Option<&str>) -> PyResult<PyLineIterator> {
        let handle = self.rebuilt()?;
        let inner: Box<dyn Iterator<Item = yggdryl::Result<String>> + Send> = match pattern {
            Some(pattern) => Box::new(
                handle
                    .inner
                    .into_read_lines_matching(pattern)
                    .map_err(value_error)?,
            ),
            None => Box::new(handle.inner.into_read_lines().map_err(value_error)?),
        };
        Ok(PyLineIterator {
            inner: std::sync::Mutex::new(inner),
        })
    }

    /// Project matched line records into a `pyarrow.RecordBatchReader`.
    ///
    /// A text-line surface beside `read_lines`, never a record method: lines
    /// group into records where `pattern` matches, and each record becomes
    /// one row - `url`, `rownum`, `date`, `time`, `unix`, `hash`, `header`,
    /// `message`, `offset`, `lines`, one nullable column per named capture
    /// group, then the constant `custom_fields` columns. A capture whose
    /// whole sub-pattern is one of the closed inference table's exact
    /// spellings types itself - `(?<thread_id>\d+)` is `int64` - and
    /// `capture_types` declares the rest (`{"price": "decimal(9, 2)"}`,
    /// values as anything naming a datatype), parsed strictly: a captured
    /// text the datatype cannot read is an error, never a silent null.
    /// Every column's datatype is one Iceberg accepts as declared;
    /// `schema_from_pattern` answers the same schema without a reader. The
    /// reader stays lazy across the boundary: `PyArrow` pulls one batch at a
    /// time through the C stream, content codings decoded as streams, a
    /// folder read leaf by leaf - so a season of compressed logs is readable
    /// from Python exactly as it is from Rust.
    #[pyo3(signature = (pattern, *, batch_size = None, custom_fields = None, capture_types = None, timestamp_capture = None))]
    fn read_arrow_lines<'py>(
        &self,
        py: Python<'py>,
        pattern: &str,
        batch_size: Option<usize>,
        custom_fields: Option<&Bound<'_, PyAny>>,
        capture_types: Option<&Bound<'_, PyAny>>,
        timestamp_capture: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut options = line_record_options(pattern, custom_fields, capture_types)?;
        options.set_batch_size(batch_size);
        if let Some(capture) = timestamp_capture {
            options
                .set_timestamp_capture(Some(capture.into()))
                .map_err(value_error)?;
        }
        // The borrowed core projection: it reopens a located leaf itself -
        // keeping a declared media-type override - and snapshots an
        // in-memory handle, so `from_bytes` parses exactly as a file does.
        let reader = self.inner.read_arrow_lines(&options).map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Read the canonical non-null struct root `Field` of this resource.
    #[pyo3(signature = (*, options = None))]
    fn read_arrow_field(&self, options: Option<&Bound<'_, PyAny>>) -> PyResult<PyField> {
        let options = self.resolve_options(options)?;
        self.inner
            .read_arrow_field(&options)
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// Read this resource as a `pyarrow.RecordBatchReader`.
    ///
    /// A schema on the options selects and casts during the read: the columns
    /// it names become the encoding's own projection, so the rest are skipped
    /// rather than read and discarded, and what comes back is the shape it
    /// declares. A handle addressing a folder reads across the partitions
    /// beneath it, so a caller never has to know which they addressed.
    #[pyo3(signature = (*, options = None))]
    fn read_arrow_batch_reader<'py>(
        &self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = self.resolve_options(options)?;
        let reader = self
            .inner
            .read_arrow_batch_reader(&options)
            .map_err(value_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Replace or merge this resource's rows with every batch `batches` yields.
    ///
    /// `batches` is anything `PyArrow` exports an Arrow C stream from - a
    /// `RecordBatchReader`, a `Table`, a `RecordBatch` - so nothing is copied
    /// on the way in. An empty `merge_by_names` overwrites; a non-empty one names the
    /// columns a row is matched on, so a matching row is updated and a
    /// non-matching one appended.
    #[pyo3(signature = (batches, *, options = None))]
    fn write_arrow_batch_reader(
        &mut self,
        batches: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let options = self.resolve_options(options)?;
        let batches = batch_reader_from_value(batches)?;
        self.inner
            .write_arrow_batch_reader(batches, &options)
            .map_err(value_error)
    }

    /// Add every batch `batches` yields after the rows this resource holds.
    #[pyo3(signature = (batches, *, options = None))]
    fn append_arrow_batch_reader(
        &mut self,
        batches: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let options = self.resolve_options(options)?;
        let batches = batch_reader_from_value(batches)?;
        self.inner
            .append_arrow_batch_reader(batches, &options)
            .map_err(value_error)
    }

    /// Read this resource's rows, as `read_arrow_batch_reader` does.
    ///
    /// This is the short name for the same call: the reader is the record shape
    /// in Python, so the generic read has nothing to infer and nothing to
    /// choose between.
    #[pyo3(signature = (*, options = None))]
    fn read_arrow<'py>(
        &self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        self.read_arrow_batch_reader(py, options)
    }

    /// Read this resource's rows as instances of a record class.
    ///
    /// `cls` is any [`yggdryl.records`] record or dataclass; each stored row
    /// becomes one instance, batch by batch, so nothing is collected. Omitting
    /// it builds the class at runtime from the resource's own schema - the
    /// rows then arrive as instances of a class you never had to declare.
    /// Rows cast flexibly onto the class's schema - names reconcile, widths
    /// convert, missing columns default - and `safe`/`errors` say how a value
    /// that will not convert is handled. A resource that does not exist reads
    /// as empty, so probing a location yields no rows rather than an error.
    #[pyo3(signature = (cls = None, *, options = None, safe = true, errors = "raise"))]
    fn read_records<'py>(
        &self,
        py: Python<'py>,
        cls: Option<&Bound<'py, PyAny>>,
        options: Option<&Bound<'py, PyAny>>,
        safe: bool,
        errors: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.read_arrow_batch_reader(py, options)?;
        let cls = if let Some(cls) = cls {
            cls.clone()
        } else {
            // The resource's own schema builds the class at runtime.
            let record = py.import("yggdryl.records")?.getattr("Record")?;
            record.call_method1("from_arrow_schema", (reader.getattr("schema")?,))?
        };
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("safe", safe)?;
        kwargs.set_item("errors", errors)?;
        kwargs.set_item("validate_schema", false)?;
        cls.call_method("from_arrow_record_batch_reader", (reader,), Some(&kwargs))
    }

    /// Replace or merge this resource's rows with record instances.
    ///
    /// `rows` is any iterable of record or dataclass instances; they become
    /// one streamed batch reader - nothing is collected - and are written
    /// exactly as [`write_arrow`](Self::write_arrow) writes. `cls` names the
    /// class when the iterable could be empty or mixed; omitted, the first
    /// row's class is the schema. An empty iterable with no class writes
    /// nothing at all, so a conditional write needs no emptiness check.
    #[pyo3(signature = (rows, *, cls = None, options = None, safe = true))]
    fn write_records(
        &mut self,
        py: Python<'_>,
        rows: &Bound<'_, PyAny>,
        cls: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
        safe: bool,
    ) -> PyResult<()> {
        match self.records_reader(py, rows, cls, options, safe)? {
            Some(reader) => self.write_arrow_batch_reader(&reader, options),
            None => Ok(()),
        }
    }

    /// Add record instances after the rows this resource holds.
    #[pyo3(signature = (rows, *, cls = None, options = None, safe = true))]
    fn append_records(
        &mut self,
        py: Python<'_>,
        rows: &Bound<'_, PyAny>,
        cls: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
        safe: bool,
    ) -> PyResult<()> {
        match self.records_reader(py, rows, cls, options, safe)? {
            Some(reader) => self.append_arrow_batch_reader(&reader, options),
            None => Ok(()),
        }
    }

    /// Replace or merge this resource's rows with whatever `data` holds.
    ///
    /// This is [`write_arrow_batch_reader`](Self::write_arrow_batch_reader)
    /// with the argument widened to everything a Python caller is likely to be
    /// holding: a `pyarrow` `RecordBatchReader`, `Table`, `RecordBatch`,
    /// `Dataset`, or `Scanner`, a `pandas` or `polars` frame, a list or
    /// generator of any of those, or an iterable of plain rows. Whatever
    /// arrives becomes one reader and is handed to the same core method, so the
    /// widening is inference and never a second way to write.
    ///
    /// Nothing that could stream is collected: a generator is pulled one item
    /// at a time, so a sequence of tables larger than memory writes exactly as
    /// a reader would. Rows arriving as mappings are grouped into batches and
    /// typed by the schema on the options, or by the first batch when no schema
    /// was declared.
    #[pyo3(signature = (data, *, options = None))]
    fn write_arrow(
        &mut self,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let options = self.resolve_options(options)?;
        let batches = batch_reader_from_any(data, &options)?;
        self.inner
            .write_arrow_batch_reader(batches, &options)
            .map_err(value_error)
    }

    /// Add whatever `data` holds after the rows this resource holds.
    ///
    /// The argument is inferred exactly as [`write_arrow`](Self::write_arrow)
    /// infers it.
    #[pyo3(signature = (data, *, options = None))]
    fn append_arrow(
        &mut self,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let options = self.resolve_options(options)?;
        let batches = batch_reader_from_any(data, &options)?;
        self.inner
            .append_arrow_batch_reader(batches, &options)
            .map_err(value_error)
    }

    /// Read this resource's rows as a lazy iterator of `pandas` frames.
    ///
    /// One frame per batch, converted when it is asked for, so a resource
    /// larger than memory reads frame by frame. `read_pandas_frame` is the one
    /// that returns every row in a single frame.
    ///
    /// `pandas` is imported here and nowhere else in this package, so a caller
    /// who does not use it never pays for it.
    #[pyo3(signature = (*, options = None))]
    fn read_pandas<'py>(
        &self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = self.resolve_options(options)?;
        let reader = self
            .inner
            .read_arrow_batch_reader(&options)
            .map_err(value_error)?;
        frames_from_reader(py, reader, Frames::Pandas)
    }

    /// Read every row of this resource as one `pandas` frame.
    #[pyo3(signature = (*, options = None))]
    fn read_pandas_frame<'py>(
        &self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = self.resolve_options(options)?;
        let reader = self
            .inner
            .read_arrow_batch_reader(&options)
            .map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Pandas)
    }

    /// Replace or merge this resource's rows with a stream of `pandas` frames.
    ///
    /// `frames` is one frame or any iterable of them, and an iterable is
    /// consumed one frame at a time. Anything that is not a `pandas` frame is
    /// refused by name, because `write_arrow` already accepts everything else.
    #[pyo3(signature = (frames, *, options = None))]
    fn write_pandas(
        &mut self,
        frames: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let options = self.resolve_options(options)?;
        let batches = frames_batch_reader(frames, Frames::Pandas, &options)?;
        self.inner
            .write_arrow_batch_reader(batches, &options)
            .map_err(value_error)
    }

    /// Replace or merge this resource's rows with exactly one `pandas` frame.
    #[pyo3(signature = (frame, *, options = None))]
    fn write_pandas_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let options = self.resolve_options(options)?;
        let batches = frame_batch_reader(frame, Frames::Pandas)?;
        self.inner
            .write_arrow_batch_reader(batches, &options)
            .map_err(value_error)
    }

    /// Read this resource's rows as a lazy iterator of `polars` frames.
    ///
    /// One frame per batch, exactly as `read_pandas` yields one pandas frame
    /// per batch. `polars` is imported here and nowhere else.
    #[pyo3(signature = (*, options = None))]
    fn read_polars<'py>(
        &self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = self.resolve_options(options)?;
        let reader = self
            .inner
            .read_arrow_batch_reader(&options)
            .map_err(value_error)?;
        frames_from_reader(py, reader, Frames::Polars)
    }

    /// Read every row of this resource as one `polars` frame.
    #[pyo3(signature = (*, options = None))]
    fn read_polars_frame<'py>(
        &self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = self.resolve_options(options)?;
        let reader = self
            .inner
            .read_arrow_batch_reader(&options)
            .map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Polars)
    }

    /// Scan this resource as a `polars.LazyFrame`.
    ///
    /// A local Parquet or Arrow leaf becomes the real lazy scan -
    /// `scan_parquet` or `scan_ipc`, predicate and projection pushdown
    /// included - and a local folder scans the matching leaves beneath it.
    /// Anything polars cannot scan natively - an in-memory buffer, a
    /// compressed name - reads through the native reader and turns lazy, so
    /// the call answers for every holder.
    #[pyo3(signature = (*, options = None))]
    fn scan_polars<'py>(
        &mut self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let polars = py.import("polars")?;
        if let Some((path, _)) = self.published_scan_target()? {
            return polars.call_method1("scan_parquet", (path,));
        }
        self.read_polars_frame(py, options)?.call_method0("lazy")
    }

    /// Scan this resource as a `pyarrow.dataset.Scanner`.
    ///
    /// A local Parquet or Arrow resource becomes a real dataset scan -
    /// column projection and predicate pushdown belong to the scanner - and
    /// anything else streams through the native reader, so the call answers
    /// for every holder.
    #[pyo3(signature = (*, options = None))]
    fn scan_arrow<'py>(
        &mut self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let dataset = py.import("pyarrow.dataset")?;
        if let Some((path, _)) = self.published_scan_target()? {
            let kwargs = pyo3::types::PyDict::new(py);
            kwargs.set_item("format", "parquet")?;
            let opened = dataset.call_method("dataset", (path,), Some(&kwargs))?;
            return opened.call_method0("scanner");
        }
        // A RecordBatchReader carries its own schema, and the scanner takes
        // it as it stands.
        let reader = self.read_arrow_batch_reader(py, options)?;
        dataset
            .getattr("Scanner")?
            .call_method1("from_batches", (reader,))
    }

    /// Replace or merge this resource's rows with a stream of `polars` frames.
    ///
    /// A `polars.LazyFrame` is accepted and collected, because polars offers no
    /// way to hand its rows over a batch at a time.
    #[pyo3(signature = (frames, *, options = None))]
    fn write_polars(
        &mut self,
        frames: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let options = self.resolve_options(options)?;
        let batches = frames_batch_reader(frames, Frames::Polars, &options)?;
        self.inner
            .write_arrow_batch_reader(batches, &options)
            .map_err(value_error)
    }

    /// Replace or merge this resource's rows with exactly one `polars` frame.
    #[pyo3(signature = (frame, *, options = None))]
    fn write_polars_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let options = self.resolve_options(options)?;
        let batches = frame_batch_reader(frame, Frames::Polars)?;
        self.inner
            .write_arrow_batch_reader(batches, &options)
            .map_err(value_error)
    }

    /// The location as text, so `str(handle)` names it.
    fn __fspath__(&self) -> PyResult<String> {
        self.inner
            .url()
            .ok_or_else(|| PyValueError::new_err("this resource has no file system path"))?
            .to_path()
            .map_err(value_error)
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn __len__(&self) -> usize {
        usize::try_from(self.inner.size()).unwrap_or(usize::MAX)
    }

    fn __iter__(&self) -> PyResult<PyIOBaseIterator> {
        Ok(PyIOBaseIterator {
            entries: self.iterdir(false)?.into_iter(),
        })
    }

    fn __str__(&self) -> String {
        self.inner
            .url()
            .map_or_else(|| "<memory>".to_owned(), ToString::to_string)
    }

    fn __repr__(&self) -> String {
        format!("IOBase({:?})", self.__str__())
    }
}

/// The iterator `for entry in handle` walks.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyIOBaseIterator {
    entries: std::vec::IntoIter<PyIOBase>,
}

#[pymethods]
impl PyIOBaseIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<PyIOBase> {
        self.entries.next()
    }

    fn __length_hint__(&self) -> usize {
        self.entries.len()
    }
}

/// Build the line projection's root Struct Field straight from a pattern.
///
/// The schema the reader emits, without a resource or a reader in sight:
/// named captures become typed columns - `(?<thread_id>\d+)` infers `int64`,
/// a `capture_types` entry declares - so a caller marks its partition columns
/// and creates the Iceberg table before the first log line exists.
#[pyfunction]
#[pyo3(signature = (pattern, *, custom_fields = None, capture_types = None))]
pub(crate) fn schema_from_pattern(
    pattern: &str,
    custom_fields: Option<&Bound<'_, PyAny>>,
    capture_types: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyField> {
    line_record_options(pattern, custom_fields, capture_types)
        .map(|options| PyField::from_inner(options.into_schema()))
}

/// Assemble validated line-record options from the boundary's arguments.
fn line_record_options(
    pattern: &str,
    custom_fields: Option<&Bound<'_, PyAny>>,
    capture_types: Option<&Bound<'_, PyAny>>,
) -> PyResult<LineRecordOptions> {
    let mut options = LineRecordOptions::new(pattern).map_err(value_error)?;
    if let Some(fields) = custom_fields {
        options = options
            .try_with_custom_fields(line_custom_fields(fields)?)
            .map_err(value_error)?;
    }
    if let Some(types) = capture_types {
        options = options
            .try_with_capture_types(line_capture_types(types)?)
            .map_err(value_error)?;
    }
    Ok(options)
}

/// Coerce the `capture_types` argument into the core's declarations.
///
/// The same shapes `custom_fields` takes - a mapping or an iterable of
/// pairs - with each value coerced through the one datatype inference, so a
/// `str` expression, a native `DataType`, or a `PyArrow` type all declare.
fn line_capture_types(types: &Bound<'_, PyAny>) -> PyResult<Vec<(String, yggdryl::DataType)>> {
    let entries = if types.hasattr("items")? {
        types.call_method0("items")?
    } else {
        types.clone()
    };
    entries
        .try_iter()?
        .map(|item| {
            let (name, value) = item?.extract::<(String, Bound<'_, PyAny>)>()?;
            Ok((name, crate::datatype::core_data_type_from_value(&value)?))
        })
        .collect()
}

/// Coerce the `custom_fields` argument into the core's ordered pairs.
///
/// Anything mapping-shaped - a dict, a `MappingProxyType`, a `ChainMap` -
/// answers `items()`, which keeps its own order; everything else is consumed
/// as an iterable of `(name, value)` pairs. Values convert through the one
/// Python-to-core conversion, so a `str`, `int`, `date`, or `Decimal` lands
/// as the typed constant it already is.
fn line_custom_fields(fields: &Bound<'_, PyAny>) -> PyResult<Vec<(String, yggdryl::Value)>> {
    let entries = if fields.hasattr("items")? {
        fields.call_method0("items")?
    } else {
        fields.clone()
    };
    let pairs: Vec<(String, Bound<'_, PyAny>)> = entries
        .try_iter()?
        .map(|item| item?.extract::<(String, Bound<'_, PyAny>)>())
        .collect::<PyResult<_>>()?;
    pairs
        .into_iter()
        .map(|(name, value)| Ok((name, crate::value::from_py(&value)?)))
        .collect()
}

/// Iterator over a resource's decoded text lines, one line at a time.
///
/// Built by [`PyIOBase::read_lines`]. The handle is rebuilt from its location
/// and owned by the iterator, so the lines outlive the handle that made them;
/// bytes stream through a fixed buffer and any content codings decode as
/// streams, so a compressed resource costs one buffer, not its decoded size.
#[pyclass(name = "LineIterator", module = "yggdryl._native")]
pub(crate) struct PyLineIterator {
    inner: std::sync::Mutex<Box<dyn Iterator<Item = yggdryl::Result<String>> + Send>>,
}

#[pymethods]
impl PyLineIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&self, py: Python<'_>) -> PyResult<Option<String>> {
        // The read and the decode run without the GIL, so another thread can
        // work while a line is fetched.
        let next = py.detach(|| {
            self.inner
                .lock()
                .map_err(|_| value_error("line iterator poisoned by an earlier panic"))
                .map(|mut lines| lines.next())
        })?;
        match next {
            None => Ok(None),
            Some(Ok(line)) => Ok(Some(line)),
            Some(Err(error)) => Err(value_error(error)),
        }
    }
}

/// A positioned view over one [`PyIOBase`], following Python's file protocol.
///
/// The cursor holds the handle itself, so its reads and writes land on the
/// same resource the handle addresses; the position is the cursor's own.
#[pyclass(name = "IOCursor", module = "yggdryl._native")]
pub(crate) struct PyIOCursor {
    handle: Py<PyIOBase>,
    position: std::sync::atomic::AtomicU64,
}

impl PyIOCursor {
    fn load(&self) -> u64 {
        self.position.load(std::sync::atomic::Ordering::Acquire)
    }

    fn store(&self, position: u64) {
        self.position
            .store(position, std::sync::atomic::Ordering::Release);
    }
}

#[pymethods]
impl PyIOCursor {
    /// The current position, in bytes from the start.
    fn tell(&self) -> u64 {
        self.load()
    }

    /// The same position as an attribute, settable.
    #[getter]
    fn position(&self) -> u64 {
        self.load()
    }

    #[setter]
    fn set_position(&self, position: u64) {
        self.store(position);
    }

    /// Move the position, as `io.IOBase.seek` moves one, returning it.
    #[pyo3(signature = (offset, whence = 0))]
    fn seek(&self, py: Python<'_>, offset: i64, whence: u8) -> PyResult<u64> {
        let origin = match whence {
            0 => 0,
            1 => self.load(),
            2 => self.handle.borrow(py).inner.size(),
            _ => {
                return Err(PyValueError::new_err(
                    "whence must be 0 (start), 1 (current), or 2 (end)",
                ));
            }
        };
        let target = origin
            .checked_add_signed(offset)
            .ok_or_else(|| PyValueError::new_err("a seek cannot land before the start"))?;
        self.store(target);
        Ok(target)
    }

    /// Read from the position, advancing it; `-1` reads to the end.
    #[pyo3(signature = (size = -1))]
    fn read<'py>(&self, py: Python<'py>, size: i64) -> PyResult<Bound<'py, PyBytes>> {
        let handle = self.handle.borrow(py);
        let position = self.load();
        let remaining = handle.inner.size().saturating_sub(position);
        let wanted = match u64::try_from(size) {
            Ok(size) => remaining.min(size),
            // A negative size reads to the end, as io.RawIOBase spells it.
            Err(_) => remaining,
        };
        let mut buffer = vec![0_u8; usize::try_from(wanted).map_err(value_error)?];
        let read = handle
            .inner
            .pread(position, &mut buffer)
            .map_err(value_error)?;
        buffer.truncate(read);
        self.store(position + read as u64);
        Ok(PyBytes::new(py, &buffer))
    }

    /// Write at the position, advancing it, returning the bytes written.
    fn write(&self, py: Python<'_>, data: &[u8]) -> PyResult<u64> {
        let mut handle = self.handle.borrow_mut(py);
        let position = self.load();
        let written = handle.inner.pwrite(position, data).map_err(value_error)?;
        self.store(position + written as u64);
        Ok(written as u64)
    }

    /// Flush the handle the cursor writes through.
    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        self.handle
            .borrow_mut(py)
            .inner
            .flush()
            .map_err(value_error)
    }
}
