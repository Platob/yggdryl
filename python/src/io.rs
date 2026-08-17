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
    /// A handle owns backend state - a mapping, an open descriptor - so it is
    /// not copied; the location it describes is what gets rebuilt.
    fn rebuilt(&self) -> PyResult<Self> {
        let url = self.inner.url().ok_or_else(|| {
            PyValueError::new_err("an in-memory resource has no location to rebuild from")
        })?;
        Self::located(&url.to_path().map_err(value_error)?)
    }

    /// Build a container handle on the same location.
    ///
    /// A table is a folder, so a caller who names one that does not exist yet
    /// still gets a handle that can resolve children; [`Holder::local`] would
    /// have decided it was a file, because nothing is there to look at.
    pub(crate) fn folder_holder(&self) -> PyResult<Holder> {
        let url = self
            .inner
            .url()
            .ok_or_else(|| PyValueError::new_err("an in-memory resource is not a container"))?;
        Holder::folder(url.to_path().map_err(value_error)?).map_err(value_error)
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
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
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
