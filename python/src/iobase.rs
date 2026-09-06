//! `IOBase`, exposed to Python with the method names `pathlib` already uses.
//!
//! The core trait is positional and fully random-access, so there are no modes
//! to open with and no cursor to keep: `read_bytes`, `write_bytes`, `iterdir`,
//! `glob`, `mkdir`, `touch`, and `unlink` all mean here what they mean on a
//! `pathlib.Path`, and each one is answered by the core implementation for the
//! backend the location names. Code written against `pathlib` therefore runs
//! against a local directory, and the same code will run against a bucket when
//! that backend lands, because only the handle changes.

use std::collections::BTreeMap;

use pyo3::buffer::PyBuffer;
use pyo3::exceptions::{PyIsADirectoryError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyString, PyTuple, PyType};

use yggdryl::holder::Holder;
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::media::{IORecordOptions as _, RecordOptions};
use yggdryl::{Codec, IOMode, Level};
use yggdryl::{IOBase as _, IOMedia as _};

use crate::iomedia::{
    Frames, PyRecordOptions, PyTextOptions, batch_reader_from_arrow_reader,
    batch_reader_from_arrow_table, batch_reader_from_records, batch_reader_to_pyarrow,
    core_record_options_from_value, core_root_field_from_value, frame_batch_reader,
    frame_from_reader, frames_batch_reader, frames_from_reader, record_batch_from_value,
};
use crate::text::codec::{decoded_as_py, decoded_into_py, with_python_bytes};
use crate::types::field::{PyField, core_field_from_value};
use crate::types::scalar::{PyScalar, from_py};
use crate::uri::{PyUrl, core_url_from_value};
use crate::value_error;

/// A random-access resource: a local file, a directory, or a memory buffer.
///
/// The base class of every storage role. A handle is never built as this class
/// directly: [`describe`] answers with the subclass the core holder turned out
/// to be, so `type(handle)` names the implementation doing the work.
#[pyclass(
    name = "IOBase",
    module = "yggdryl._native",
    subclass,
    skip_from_py_object
)]
pub(crate) struct PyIOBase {
    /// The held implementation, until a consuming conversion takes it.
    ///
    /// A wrapper - a page cache, a coding, a text configuration - owns the
    /// handle it wraps, so building one moves this holder into the new handle
    /// the conversion answers with. The emptied handle then raises rather than
    /// reading a resource it no longer addresses.
    held: Option<Holder>,
}

/// Report a handle whose value a conversion already moved elsewhere.
fn consumed() -> PyErr {
    PyValueError::new_err(
        "this handle was consumed by a conversion; use the handle it returned instead",
    )
}

/// Rebuild a foreign-filesystem handle, keeping the filesystem it stands on.
///
/// `None` for anything else, so the local rebuild stays the default path.
fn rebuilt_arrow_holder(inner: &Holder) -> Option<Holder> {
    inner
        .bound_location()
        .cloned()
        .map(yggdryl::holder::fs::located)
}

/// Hold the resource `url` names, on the store its scheme selects.
///
/// The scheme is what says which backend a location belongs to. An `s3` URL
/// reaches the native S3 backend; everything else stays local, exactly as it
/// did. Construction touches nothing on either.
pub(crate) fn located_holder(url: &yggdryl::Url) -> PyResult<Holder> {
    if url.scheme() == &yggdryl::Scheme::S3 {
        return yggdryl::holder::s3::located(&url.to_string())
            .map_err(crate::holder::fs::storage_error);
    }
    Holder::local(url.clone().into_path().map_err(value_error)?)
        .map_err(crate::holder::fs::storage_error)
}

/// Hold `url` as a container, on the store its scheme selects.
pub(crate) fn folder_holder_for(url: &yggdryl::Url) -> PyResult<Holder> {
    if url.scheme() == &yggdryl::Scheme::S3 {
        return yggdryl::holder::s3::folder(&url.to_string())
            .map(Holder::S3Folder)
            .map_err(crate::holder::fs::storage_error);
    }
    Holder::folder(url.clone().into_path().map_err(value_error)?).map_err(value_error)
}

/// Address a foreign-filesystem handle's location as a container.
pub(crate) fn fs_folder_holder(inner: &Holder) -> Option<Holder> {
    inner
        .bound_location()
        .cloned()
        .map(yggdryl::holder::fs::Folder::new)
        .map(Holder::FsFolder)
}

/// The role one core holder is, named once for both of its readers.
///
/// `describe` turns it into a class and `stack` into a word; deriving both
/// from one reading is what keeps `type(handle)` and `repr(handle)` from ever
/// disagreeing about what a handle is.
#[derive(Clone, Copy)]
pub(crate) enum Role {
    Buffer,
    Folder,
    Path,
    File,
    FsFolder,
    FsPath,
    FsFile,
    Buffered,
    Coded(Codec),
    Text,
    Media(crate::media::handles::Encoding),
    /// A holder this build has no class for.
    Held,
}

impl Role {
    /// Read the role off a holder, without consuming it.
    fn of(holder: &Holder) -> Self {
        match holder {
            Holder::Buffer(_) => Self::Buffer,
            Holder::Folder(_) => Self::Folder,
            Holder::Path(_) => Self::Path,
            Holder::File(_) => Self::File,
            Holder::FsFolder(_) => Self::FsFolder,
            Holder::FsPath(_) => Self::FsPath,
            Holder::FsFile(_) => Self::FsFile,
            Holder::Buffered(_) => Self::Buffered,
            Holder::Text(_) => Self::Text,
            Holder::Coded(coded) => Self::Coded(coded.codec()),
            Holder::Media(media) => Self::Media(crate::media::handles::Encoding::of(media)),
            // `Holder` is non-exhaustive, so a variant added to the core after
            // this build arrives here. Answering the base class is the honest
            // reading - it holds the whole contract - but it also means a new
            // variant will not remind anyone to name it here.
            _ => Self::Held,
        }
    }

    /// The class name this role reports.
    fn name(self) -> &'static str {
        use crate::media::handles::Encoding;
        match self {
            Self::Buffer => "Buffer",
            Self::Folder => "Folder",
            Self::Path => "Path",
            Self::File => "File",
            Self::FsFolder => "FsFolder",
            Self::FsPath => "FsPath",
            Self::FsFile => "FsFile",
            Self::Buffered => "Buffered",
            Self::Coded(Codec::Gzip) => "Gzip",
            Self::Coded(Codec::Zlib | Codec::Deflate) => "Zlib",
            Self::Coded(Codec::Zstd) => "Zstd",
            Self::Coded(Codec::Identity) => "Identity",
            Self::Coded(_) => "Coded",
            Self::Text | Self::Media(Encoding::Text) => "Text",
            Self::Media(Encoding::Ipc) => "Ipc",
            Self::Media(Encoding::Parquet) => "Parquet",
            Self::Media(Encoding::Avro) => "Avro",
            Self::Held => "IOBase",
        }
    }
}

/// Answer `holder` as the Python class its implementation names.
///
/// This is the one place a core holder becomes a Python object, so every
/// handle a caller ever receives - from the constructor, from `parent`, from a
/// listing, from a conversion - reports the implementation actually doing the
/// work.
pub(crate) fn describe(py: Python<'_>, holder: Holder) -> PyResult<Py<PyAny>> {
    use crate::coding::handles as codings;
    use crate::holder::handles as roles;
    use crate::media::handles as encodings;

    let role = Role::of(&holder);
    let base = PyClassInitializer::from(PyIOBase::from_core(holder));
    Ok(match role {
        Role::Buffer => Py::new(py, base.add_subclass(roles::PyBuffer))?.into_any(),
        Role::Folder => Py::new(py, base.add_subclass(roles::PyFolder))?.into_any(),
        Role::Path => Py::new(py, base.add_subclass(roles::PyPath))?.into_any(),
        Role::File => Py::new(py, base.add_subclass(roles::PyFile))?.into_any(),
        Role::FsFolder => Py::new(py, base.add_subclass(roles::PyFsFolder))?.into_any(),
        Role::FsPath => Py::new(py, base.add_subclass(roles::PyFsPath))?.into_any(),
        Role::FsFile => Py::new(py, base.add_subclass(roles::PyFsFile))?.into_any(),
        Role::Buffered => Py::new(py, base.add_subclass(roles::PyBuffered))?.into_any(),
        Role::Text => encodings::describe_text(py, base)?,
        Role::Coded(codec) => codings::describe(py, base, codec)?,
        Role::Media(encoding) => encodings::describe(py, base, encoding)?,
        Role::Held => Py::new(py, base)?.into_any(),
    })
}

/// The content coding the *stored* bytes carry.
///
/// A composed handle presents the decoded value, so its own media type
/// declares no coding - the coding is the layer doing the decoding. Reading
/// the composition first keeps one answer to "what are these bytes wrapped
/// in", whether or not the handle has been composed.
fn applied_codec(holder: &Holder) -> Codec {
    match holder {
        Holder::Buffered(buffered) => applied_codec(buffered.handle()),
        Holder::Coded(coded) => coded.codec(),
        Holder::Text(text) => applied_codec(text.handle()),
        Holder::Media(media) => applied_codec(media.handle()),
        other => other.codec(),
    }
}

/// Return whether a handle presents the decoded value rather than what is
/// stored.
fn presents_decoded(holder: &Holder) -> bool {
    match holder {
        Holder::Coded(_) => true,
        Holder::Buffered(buffered) => presents_decoded(buffered.handle()),
        Holder::Text(text) => presents_decoded(text.handle()),
        Holder::Media(media) => presents_decoded(media.handle()),
        _ => false,
    }
}

/// Refuse a decoded view where the stored bytes are what is meant.
///
/// The two coding transfers move *stored* bytes and report their sizes, so a
/// handle that already codes on the way through would apply the coding twice
/// and count the wrong value. Writing plain bytes through such a handle is
/// what stores the coded form, so `copy_into` is the call that means this.
fn require_stored(holder: &Holder, role: &str) -> PyResult<()> {
    if !presents_decoded(holder) {
        return Ok(());
    }
    let location = holder
        .url()
        .map_or_else(|| "an in-memory value".to_owned(), ToString::to_string);
    Err(PyValueError::new_err(format!(
        "expected {role} presenting its stored bytes, got a {} view of {location}; a coded handle \
         codes what passes through it, so copy_into already stores the coded form - or address \
         the stored bytes with the Path, File, FsPath, or FsFile role",
        applied_codec(holder).as_str(),
    )))
}

/// Take one composed layer off a wrapper, spending the handle that held it.
///
/// The inverse of the conversions that built the stack, and consuming for the
/// same reason: a wrapper owns the handle it wraps, so the value moves to the
/// handle this answers with. A coded layer publishes its pending writes on the
/// way out, which is why it can fail.
pub(crate) fn unwrapped(py: Python<'_>, base: &mut PyIOBase) -> PyResult<Py<PyAny>> {
    if !matches!(
        base.inner()?,
        Holder::Buffered(_) | Holder::Coded(_) | Holder::Text(_) | Holder::Media(_)
    ) {
        return Err(PyValueError::new_err(
            "this handle wraps no other handle to descend to",
        ));
    }
    let inner = match base.take()? {
        Holder::Buffered(buffered) => buffered.into_handle(),
        Holder::Coded(coded) => coded.into_handle().map_err(value_error)?,
        Holder::Text(text) => text.into_handle(),
        Holder::Media(media) => media.into_handle(),
        held => held,
    };
    describe(py, inner)
}

/// Render the composition a handle stands on, outermost role first.
///
/// `Text(Gzip(Path("file:///trades.txt.gz")))` says in one line what the class
/// alone only says about the top: which wrappers are in play, in which order,
/// and what they finally sit on.
fn stack(holder: &Holder, location: &str) -> String {
    let name = Role::of(holder).name();
    match holder {
        Holder::Buffered(buffered) => format!("{name}({})", stack(buffered.handle(), location)),
        Holder::Coded(coded) => format!("{name}({})", stack(coded.handle(), location)),
        Holder::Text(text) => format!("{name}({})", stack(text.handle(), location)),
        Holder::Media(media) => format!("{name}({})", stack(media.handle(), location)),
        _ => format!("{name}({location})"),
    }
}

/// Compose what a location's name declares, then answer its class.
///
/// The composition is the core's own and costs nothing: a name is read, no
/// store is touched, and a location that declares neither a coding nor a
/// record encoding is answered exactly as it arrived. It is applied wherever a
/// handle is described - construction, `parent`, `joinpath`, a listing entry -
/// so `IOBase("trades.txt.gz")` reads decoded text rows, and the same child of
/// a bucket does too.
pub(crate) fn declared(py: Python<'_>, holder: Holder) -> PyResult<Py<PyAny>> {
    describe(py, holder.into_declared_media())
}

impl PyIOBase {
    pub(crate) const fn from_core(inner: Holder) -> Self {
        Self { held: Some(inner) }
    }

    /// Borrow the held implementation.
    pub(crate) fn inner(&self) -> PyResult<&Holder> {
        self.held.as_ref().ok_or_else(consumed)
    }

    /// Borrow the held implementation mutably.
    pub(crate) fn inner_mut(&mut self) -> PyResult<&mut Holder> {
        self.held.as_mut().ok_or_else(consumed)
    }

    /// Take the held implementation, emptying this handle.
    pub(crate) fn take(&mut self) -> PyResult<Holder> {
        self.held.take().ok_or_else(consumed)
    }

    /// Describe a local location, as whichever role fits what is there.
    ///
    /// The specialized roles are what a record caller needs: a leaf reports the
    /// media type its name implies, which is where the encoding comes from,
    /// while the generic location reports only that it is a file. A handle for
    /// a location that does not exist yet is therefore a leaf.
    fn located(path: &std::path::Path) -> PyResult<Holder> {
        Holder::local(path).map_err(crate::holder::fs::storage_error)
    }

    /// Describe the resource `url` names, on the store its scheme selects.
    ///
    /// A location is what says which backend it belongs to, so this is the one
    /// place that decides. Nothing is opened or contacted here either way.
    fn located_url(url: &yggdryl::Url) -> PyResult<Holder> {
        located_holder(url)
    }

    /// Build a second holder on the same location.
    ///
    /// A handle owns backend state - such as a mapping or an open descriptor -
    /// so it is not copied; the location it describes is what gets
    /// rebuilt. A handle on a foreign Arrow filesystem rebuilds onto that
    /// same filesystem, because its location alone would not say where it
    /// lives.
    fn rebuilt(&self) -> PyResult<Holder> {
        if let Some(holder) = rebuilt_arrow_holder(self.inner()?) {
            return Ok(holder);
        }
        let url = self.inner()?.url().ok_or_else(|| {
            PyValueError::new_err("an in-memory resource has no location to rebuild from")
        })?;
        Self::located_url(url)
    }

    /// Build a container handle on the same location.
    ///
    /// A table is a folder, so a caller who names one that does not exist yet
    /// still gets a handle that can resolve children; [`Holder::local`] would
    /// have decided it was a file, because nothing is there to look at. A
    /// foreign filesystem's handle becomes a container on that filesystem, so
    /// a table reached this way never learns which backend it stands on.
    pub(crate) fn folder_holder(&self) -> PyResult<Holder> {
        if let Some(holder) = fs_folder_holder(self.inner()?) {
            return Ok(holder);
        }
        let url = self
            .inner()?
            .url()
            .ok_or_else(|| PyValueError::new_err("an in-memory resource is not a container"))?;
        folder_holder_for(url)
    }

    /// Build a handle on `path` over a held `pyarrow.fs.FileSystem`.
    fn over_fs(
        filesystem: &Bound<'_, PyAny>,
        path: &Bound<'_, PyAny>,
        uri: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Holder> {
        let path = crate::uri::path_string_from_value(path)?;
        let uri = uri.map(crate::uri::path_string_from_value).transpose()?;
        Self::over_fs_parts(filesystem, path, uri)
    }

    fn over_fs_parts(
        filesystem: &Bound<'_, PyAny>,
        path: String,
        uri: Option<String>,
    ) -> PyResult<Holder> {
        let backend: std::sync::Arc<dyn yggdryl::holder::fs::FileSystem> =
            std::sync::Arc::new(crate::holder::fs::PyFileSystem::new(filesystem)?);
        let bound = yggdryl::holder::fs::BoundLocation::new(backend, path, uri)
            .map_err(crate::holder::fs::storage_error)?;
        Ok(yggdryl::holder::fs::located(bound))
    }

    fn arrow_binding(&self, py: Python<'_>) -> Option<(Py<PyAny>, String)> {
        let bound = self.held.as_ref()?.bound_location()?;
        let filesystem = bound
            .filesystem()
            .as_any()
            .downcast_ref::<crate::holder::fs::PyFileSystem>()?;
        Some((filesystem.original(py), bound.path().to_owned()))
    }

    fn native_cursor(slf: &Bound<'_, Self>, position: u64) -> PyIOCursor {
        PyIOCursor {
            handle: slf.clone().unbind(),
            position: std::sync::atomic::AtomicU64::new(position),
            closed: std::sync::atomic::AtomicBool::new(false),
            reader: std::sync::Mutex::new(None),
            close_failure: std::sync::Mutex::new(None),
        }
    }

    fn native_python_file<'py>(
        slf: &Bound<'py, Self>,
        position: u64,
        mode: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let cursor = Py::new(py, Self::native_cursor(slf, position))?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("mode", mode)?;
        py.import("pyarrow")?
            .getattr("PythonFile")?
            .call((cursor,), Some(&kwargs))
    }

    fn bound(&self) -> PyResult<&yggdryl::holder::fs::BoundLocation> {
        self.inner()?
            .bound_location()
            .ok_or_else(|| PyValueError::new_err("this handle has no bound filesystem location"))
    }

    /// Resolve the options a record call runs under.
    ///
    /// Omitting them is the normal case: the encoding then comes from the
    /// handle's own media type, so it is never guessed.
    fn resolve_options(&self, options: Option<&Bound<'_, PyAny>>) -> PyResult<RecordOptions> {
        match options {
            Some(options) => core_record_options_from_value(options),
            None => self.inner()?.record_options().map_err(value_error),
        }
    }

    /// Resolve and validate one explicit mode before touching an input value.
    fn write_options(
        &mut self,
        mode: IOMode,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Option<RecordOptions>> {
        let options = self.resolve_options(options)?;
        options.require_write_mode(mode).map_err(value_error)?;
        options.require_commit_row_size().map_err(value_error)?;
        options.require_write_limits().map_err(value_error)?;
        if options.write_limit_is_zero() {
            if mode == IOMode::Overwrite {
                // A zero limit admits no input row. The explicit field supplies
                // the output shape, so overwrite can publish a typed empty
                // value without asking a one-shot Python object for its schema.
                let field = options.require_field().map_err(value_error)?;
                let schema = field.clone().into_arrow_schema().map_err(value_error)?;
                self.write_reader(yggdryl::arrow::batch_reader(schema, []), mode, &options)?;
            }
            // Append is a true no-op. Merge with a limit is rejected by the
            // shared preflight before this branch.
            return Ok(None);
        }
        Ok(Some(options))
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
        let media_type = self.inner()?.media_type();
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
        let Some(url) = self.inner()?.url() else {
            return Ok(None);
        };
        let Ok(path) = url.clone().into_path() else {
            return Ok(None);
        };
        let path = path.to_string_lossy().into_owned();
        self.inner_mut()?
            .close()
            .map_err(crate::holder::fs::storage_error)?;
        Ok(Some((path, base)))
    }

    /// Read this resource with options that are already resolved.
    fn read_reader<'py>(
        &self,
        py: Python<'py>,
        options: &RecordOptions,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = self
            .inner()?
            .read_arrow_reader(options)
            .map_err(crate::holder::fs::storage_error)?;
        batch_reader_to_pyarrow(py, reader)
    }

    /// Write one core reader with resolved options and explicit intent.
    fn write_reader(
        &mut self,
        batches: yggdryl::arrow::BatchReader,
        mode: IOMode,
        options: &RecordOptions,
    ) -> PyResult<()> {
        self.inner_mut()?
            .write_arrow_reader(batches, mode, options)
            .map_err(crate::holder::fs::storage_error)
    }
}

fn filesystem_uri_options(
    options: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<BTreeMap<String, String>>> {
    let Some(options) = options else {
        return Ok(None);
    };
    let mut values = BTreeMap::new();
    for item in options.call_method0("items")?.try_iter()? {
        let item = item?;
        let pair = item.cast::<PyTuple>()?;
        if pair.len() != 2 {
            return Err(PyTypeError::new_err("filesystem options must be a mapping"));
        }
        let key = pair.get_item(0)?.extract::<String>()?;
        let value = pair.get_item(1)?;
        let value = if value.is_instance_of::<PyBool>() {
            if value.extract::<bool>()? {
                "true".to_owned()
            } else {
                "false".to_owned()
            }
        } else {
            value.extract::<String>().map_err(|_| {
                PyTypeError::new_err(format!(
                    "filesystem option {key:?} must be a string or boolean"
                ))
            })?
        };
        values.insert(key, value);
    }
    Ok(Some(values))
}

fn resolved_arrow_filesystem<'py>(
    py: Python<'py>,
    filesystem: &yggdryl::holder::fs::ResolvedFileSystem,
) -> PyResult<Bound<'py, PyAny>> {
    let module = py.import("pyarrow.fs")?;
    match filesystem {
        yggdryl::holder::fs::ResolvedFileSystem::Local => {
            module.getattr("LocalFileSystem")?.call0()
        }
        yggdryl::holder::fs::ResolvedFileSystem::S3(options) => {
            let kwargs = PyDict::new(py);
            if let Some(value) = options.access_key() {
                kwargs.set_item("access_key", value)?;
            }
            if let Some(value) = options.secret_key() {
                kwargs.set_item("secret_key", value)?;
            }
            if let Some(value) = options.session_token() {
                kwargs.set_item("session_token", value)?;
            }
            if let Some(value) = options.endpoint_override() {
                kwargs.set_item("endpoint_override", value)?;
            }
            if let Some(value) = options.region() {
                kwargs.set_item("region", value)?;
            }
            kwargs.set_item("scheme", options.transport())?;
            kwargs.set_item("anonymous", options.anonymous())?;
            match options.addressing_style() {
                yggdryl::holder::fs::S3AddressingStyle::Automatic => {}
                yggdryl::holder::fs::S3AddressingStyle::Path => {
                    kwargs.set_item("force_virtual_addressing", false)?;
                }
                yggdryl::holder::fs::S3AddressingStyle::Virtual => {
                    kwargs.set_item("force_virtual_addressing", true)?;
                }
            }
            module.getattr("S3FileSystem")?.call((), Some(&kwargs))
        }
    }
}

#[pymethods]
impl PyIOBase {
    // A handle observes mutable external state and has no canonical value
    // identity. Never expose Python's inherited object-identity hash for it.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

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
    /// filesystem `PyArrow` ships is accepted, as is a custom one wrapped in
    /// `PyFileSystem(FileSystemHandler)`, and what comes back is this same
    /// class - nothing filesystem-specific leaks into the surface.
    #[new]
    #[pyo3(signature = (value, path = None))]
    #[allow(clippy::new_ret_no_self)] // Construction answers the role subclass, never this base class.
    fn new(
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        path: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        if crate::holder::fs::is_arrow_filesystem(value)? {
            let path = path.ok_or_else(|| {
                PyValueError::new_err(
                    "expected a path on the filesystem as the second argument, got none",
                )
            })?;
            return declared(py, Self::over_fs(value, path, None)?);
        }
        if let Some(path) = path {
            return Err(PyValueError::new_err(format!(
                "expected a pyarrow.fs.FileSystem to resolve {} against, got {}",
                path.repr()
                    .map_or_else(|_| "a second argument".to_owned(), |text| text.to_string()),
                value.get_type().name()?,
            )));
        }
        if let Ok(handle) = value.extract::<PyRef<'_, Self>>() {
            if handle.inner()?.kind() != yggdryl::IOKind::Memory {
                return declared(py, handle.rebuilt()?);
            }
            // No location to rebuild from, so the content is what is taken.
            let bytes = handle
                .inner()?
                .read_all_bytes()
                .map_err(crate::holder::fs::storage_error)?;
            let mut buffer = Holder::Buffer(yggdryl::holder::Buffer::from_bytes(bytes));
            buffer.set_media_type(handle.inner()?.media_type().clone());
            return declared(py, buffer);
        }
        if value.hasattr("read")? && !value.is_instance_of::<PyString>() {
            // An open file knows where it lives; `name` is an `int` for a
            // descriptor-opened one and absent on a plain stream, and neither
            // of those names a place.
            if let Ok(name) = value.getattr("name")
                && name.is_instance_of::<PyString>()
            {
                return declared(py, Self::located(&name.extract::<std::path::PathBuf>()?)?);
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
            return declared(
                py,
                Holder::Buffer(yggdryl::holder::Buffer::from_bytes(bytes)),
            );
        }
        let url = core_url_from_value(value)?;
        declared(py, Self::located_url(&url)?)
    }

    /// Describe a resource on any `pyarrow.fs.FileSystem`.
    ///
    /// This is the explicit spelling of what the constructor infers, and it
    /// accepts every Arrow filesystem: `S3FileSystem`,
    /// `GcsFileSystem`, `AzureFileSystem`, `LocalFileSystem`,
    /// `SubTreeFileSystem`, and a custom filesystem wrapped in
    /// `PyFileSystem(FileSystemHandler)` - which is also how `fsspec` arrives
    /// - all reach the same complete filesystem and stream contract.
    ///
    /// ```python
    /// handle = IOBase.from_fs(S3FileSystem(region="eu-west-1"), "bucket/key.parquet")
    /// reader = handle.read_arrow_reader()
    /// ```
    ///
    /// The result is an ordinary handle: `iterdir`, `glob`, `/`, and `parent`
    /// return handles that still carry the filesystem, and the three record
    /// methods work exactly as they do on a local file. Per the laziness
    /// contract nothing is opened, created, or read here.
    ///
    /// The four explicit `open_*` methods return `PyArrow` native files and
    /// forward writes as they arrive; close each returned stream to flush the
    /// backend exactly once.
    #[classmethod]
    #[pyo3(signature = (filesystem, path, *, uri = None))]
    fn from_fs(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        filesystem: &Bound<'_, PyAny>,
        path: &Bound<'_, PyAny>,
        uri: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        if !crate::holder::fs::is_arrow_filesystem(filesystem)? {
            return Err(PyValueError::new_err(format!(
                "expected a pyarrow.fs.FileSystem, got {}",
                filesystem.get_type().name()?,
            )));
        }
        declared(py, Self::over_fs(filesystem, path, uri)?)
    }

    /// Resolve one `file`, `s3`, `s3a`, or `s3n` URI through the core parser.
    #[classmethod]
    #[pyo3(signature = (uri, *, options = None))]
    fn from_uri(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        uri: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let uri = crate::uri::path_string_from_value(uri)?;
        let options = filesystem_uri_options(options)?;
        let resolved =
            yggdryl::holder::fs::ResolvedFileSystemUri::from_uri(uri.clone(), options.as_ref())
                .map_err(crate::holder::fs::storage_error)?;
        let filesystem = resolved_arrow_filesystem(py, resolved.filesystem())?;
        let holder = Self::over_fs_parts(&filesystem, resolved.path().to_owned(), Some(uri))?;
        declared(py, holder)
    }

    /// Describe an in-memory resource holding `data`.
    ///
    /// `capacity` reserves the allocation up front, so a caller who knows the
    /// final length pays for one allocation rather than a run of growths. It
    /// changes what the buffer can hold without changing what it holds.
    #[classmethod]
    #[pyo3(signature = (data = None, *, capacity = None))]
    fn from_bytes(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        data: Option<Vec<u8>>,
        capacity: Option<usize>,
    ) -> PyResult<Py<PyAny>> {
        let buffer = match (data, capacity) {
            (None, Some(capacity)) => yggdryl::holder::Buffer::with_capacity(capacity),
            (data, Some(capacity)) => {
                let mut buffer = yggdryl::holder::Buffer::with_capacity(capacity);
                buffer
                    .write_all_bytes(&data.unwrap_or_default())
                    .map_err(crate::holder::fs::storage_error)?;
                buffer
            }
            (data, None) => yggdryl::holder::Buffer::from_bytes(data.unwrap_or_default()),
        };
        describe(py, Holder::Buffer(buffer))
    }

    /// The location this handle addresses.
    #[getter]
    fn url(&self) -> PyResult<Option<PyUrl>> {
        Ok(self.inner()?.url().cloned().map(PyUrl::from_core))
    }

    /// The exact `pyarrow.fs.FileSystem` supplied at construction.
    #[getter]
    fn filesystem(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let Some(bound) = self.inner()?.bound_location() else {
            return Ok(None);
        };
        Ok(bound
            .filesystem()
            .as_any()
            .downcast_ref::<crate::holder::fs::PyFileSystem>()
            .map(|filesystem| filesystem.original(py)))
    }

    /// The exact opaque path passed to the bound filesystem.
    #[getter]
    fn path(&self) -> PyResult<Option<String>> {
        Ok(self
            .inner()?
            .bound_location()
            .map(|bound| bound.path().to_owned()))
    }

    /// The caller's exact optional URI spelling. It may contain credentials.
    #[getter]
    fn uri(&self) -> PyResult<Option<String>> {
        Ok(self
            .inner()?
            .bound_location()
            .and_then(|bound| bound.uri().map(str::to_owned)))
    }

    /// A credential-free URI for diagnostics and logs.
    #[getter]
    fn masked_uri(&self) -> PyResult<Option<String>> {
        Ok(self
            .inner()?
            .bound_location()
            .and_then(|bound| bound.masked_uri().map(str::to_owned)))
    }

    /// Inspect this exact bound path as a `pyarrow.fs.FileInfo`.
    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let bound = self.bound()?;
        let info = bound
            .filesystem()
            .file_info(bound.path())
            .map_err(crate::holder::fs::storage_error)?;
        let module = py.import("pyarrow.fs")?;
        let file_type = module.getattr("FileType")?;
        let kind = match info.kind {
            yggdryl::IOKind::File => file_type.getattr("File")?,
            yggdryl::IOKind::Directory => file_type.getattr("Directory")?,
            yggdryl::IOKind::Unknown => file_type.getattr("NotFound")?,
            other => {
                return Err(PyValueError::new_err(format!(
                    "a filesystem cannot report the storage kind {}",
                    other.as_str()
                )));
            }
        };
        let kwargs = PyDict::new(py);
        kwargs.set_item("type", kind)?;
        if let Some(size) = info.size {
            kwargs.set_item("size", size)?;
        }
        if let Some(mtime_ns) = info.mtime_ns {
            kwargs.set_item("mtime_ns", mtime_ns)?;
        }
        module
            .getattr("FileInfo")?
            .call((&info.path,), Some(&kwargs))
    }

    /// Return whether both handles bind the same filesystem and raw path.
    fn same_location(&self, other: &Self) -> PyResult<bool> {
        let (Some(left), Some(right)) = (
            self.inner()?.bound_location(),
            other.inner()?.bound_location(),
        ) else {
            return Ok(false);
        };
        if left.path() != right.path() {
            return Ok(false);
        }
        left.try_same_location(right)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Ask the bound filesystem to normalize a path explicitly.
    fn normalize_path(&self, path: &Bound<'_, PyAny>) -> PyResult<String> {
        let path = crate::uri::path_string_from_value(path)?;
        self.bound()?
            .filesystem()
            .normalize_path(&path)
            .map_err(crate::holder::fs::storage_error)
    }

    /// The final path component, as `pathlib.PurePath.name`.
    #[getter]
    fn name(&self) -> PyResult<String> {
        let inner = self.inner()?;
        if let Some(bound) = inner.bound_location() {
            return Ok(bound
                .path()
                .strip_suffix('/')
                .unwrap_or(bound.path())
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned());
        }
        Ok(inner
            .url()
            .and_then(|url| url.file_name())
            .unwrap_or_default()
            .to_owned())
    }

    /// The media type of the bytes here.
    #[getter]
    fn media_type(&self) -> PyResult<crate::enums::PyMediaType> {
        Ok(crate::enums::PyMediaType::from_core(
            self.inner()?.media_type().clone(),
        ))
    }

    /// Declare what the bytes here are, as a media type, MIME type, or string.
    ///
    /// A located resource infers this from its name, so the setter is what an
    /// in-memory buffer uses to say which record encoding it holds.
    #[setter]
    fn set_media_type(&mut self, media_type: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner_mut()?
            .set_media_type(crate::enums::core_media_type_from_value(media_type)?);
        Ok(())
    }

    /// The content coding the stored bytes carry, or `None` for none.
    ///
    /// This is what a name says the bytes are wrapped in - `data.json.gz`
    /// reads as `"gzip"` - and it is what [`decompress_into`][Self::decompress_into]
    /// decodes with when the caller names no coding. Identity is spelled
    /// `None` rather than `"identity"`, because "these bytes carry no coding"
    /// is the question a caller is actually asking.
    ///
    /// A composed handle answers the same way. `IOBase("data.json.gz")` is a
    /// `Gzip` presenting decoded JSON, so its *media type* declares no coding;
    /// the coding is the layer, and that is what this reports.
    #[getter]
    fn codec(&self) -> PyResult<Option<&'static str>> {
        Ok(match applied_codec(self.inner()?) {
            Codec::Identity => None,
            codec => Some(codec.as_str()),
        })
    }

    /// The number of bytes here, as `Path.stat().st_size`.
    #[getter]
    fn size(&self) -> PyResult<u64> {
        match self.inner()?.bound_location() {
            Some(bound) => bound
                .filesystem()
                .file_info(bound.path())
                .map(|info| info.size.unwrap_or(0))
                .map_err(crate::holder::fs::storage_error),
            None => Ok(self.inner()?.size()),
        }
    }

    /// The exact core storage role: memory, file, directory, table,
    /// namespace, catalog, or unknown.
    #[getter]
    fn kind(&self) -> PyResult<&'static str> {
        match self.inner()?.bound_location() {
            Some(bound) => bound
                .filesystem()
                .file_info(bound.path())
                .map(|info| info.kind.as_str())
                .map_err(crate::holder::fs::storage_error),
            None => Ok(self.inner()?.kind().as_str()),
        }
    }

    /// The number of logical rows in this media value.
    ///
    /// Schema-bearing encodings answer from metadata, and an opened handle
    /// retains that answer until close. Text counts its extractor stream
    /// without materializing Arrow batches.
    #[getter]
    fn row_size(&self) -> PyResult<u64> {
        self.inner()?.row_size().map_err(value_error)
    }

    /// The number of columns in this media value's canonical struct field.
    #[getter]
    fn column_size(&self) -> PyResult<usize> {
        self.inner()?.column_size().map_err(value_error)
    }

    /// The containing resource, as `PurePath.parent`.
    #[getter]
    fn parent(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        self.inner()?
            .parent()
            .map(|holder| declared(py, holder))
            .transpose()
    }

    /// Resolve a child of this resource, as `PurePath.joinpath`.
    #[pyo3(signature = (*others))]
    fn joinpath(&self, py: Python<'_>, others: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        let mut resolved: Option<Holder> = None;
        for other in others {
            let name = crate::uri::path_string_from_value(&other)?;
            let base = resolved.as_ref().map_or_else(|| self.inner(), Ok)?;
            resolved = Some(base.child_by_path(&name).map_err(value_error)?);
        }
        match resolved {
            Some(handle) => declared(py, handle),
            // `joinpath()` with nothing to join is the same location.
            None => declared(py, self.rebuilt()?),
        }
    }

    /// `handle / "child"`, as `PurePath.__truediv__`.
    fn __truediv__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let child = self
            .inner()?
            .child_by_path(&crate::uri::path_string_from_value(other)?)
            .map_err(value_error)?;
        declared(py, child)
    }

    /// Return whether anything is here now, as `Path.exists`.
    fn exists(&self) -> PyResult<bool> {
        match self.inner()?.bound_location() {
            Some(bound) => bound
                .filesystem()
                .file_info(bound.path())
                .map(|info| info.kind != yggdryl::IOKind::Unknown)
                .map_err(crate::holder::fs::storage_error),
            None => Ok(self.inner()?.kind() != yggdryl::IOKind::Unknown),
        }
    }

    /// Return whether this resource contains others, as `Path.is_dir`.
    fn is_dir(&self) -> PyResult<bool> {
        match self.inner()?.bound_location() {
            Some(bound) => bound
                .filesystem()
                .file_info(bound.path())
                .map(|info| info.kind == yggdryl::IOKind::Directory)
                .map_err(crate::holder::fs::storage_error),
            None => Ok(self.inner()?.is_container()),
        }
    }

    /// Return whether this resource holds bytes, as `Path.is_file`.
    fn is_file(&self) -> PyResult<bool> {
        match self.inner()?.bound_location() {
            Some(bound) => bound
                .filesystem()
                .file_info(bound.path())
                .map(|info| info.kind == yggdryl::IOKind::File)
                .map_err(crate::holder::fs::storage_error),
            None => Ok(self.inner()?.kind() == yggdryl::IOKind::File),
        }
    }

    /// Return whether this handle exposes its byte or record surface.
    fn is_io(&self) -> PyResult<bool> {
        if let Some(bound) = self.inner()?.bound_location() {
            bound
                .filesystem()
                .file_info(bound.path())
                .map_err(crate::holder::fs::storage_error)?;
        }
        Ok(self.inner()?.is_io())
    }

    /// Return whether this resource is one whole byte value.
    ///
    /// The byte surface - `read_bytes` and `write_bytes` - is for an atomic
    /// resource; `is_tabular` names the record surface instead. A container
    /// holding neither answers `False` to both.
    fn is_atomic(&self) -> PyResult<bool> {
        if let Some(bound) = self.inner()?.bound_location() {
            bound
                .filesystem()
                .file_info(bound.path())
                .map_err(crate::holder::fs::storage_error)?;
        }
        Ok(self.inner()?.is_atomic())
    }

    /// Return whether this resource holds rows and columns.
    ///
    /// The record surface - `read_arrow_reader` and its explicit write
    /// triplets - is for a tabular resource: a leaf whose media type names a
    /// record encoding, a folder that reads as the table beneath it, or a
    /// table format's own folder.
    fn is_tabular(&self) -> PyResult<bool> {
        if let Some(bound) = self.inner()?.bound_location() {
            bound
                .filesystem()
                .file_info(bound.path())
                .map_err(crate::holder::fs::storage_error)?;
        }
        Ok(self.inner()?.is_tabular())
    }

    /// Iterate the immediate children, as `Path.iterdir`.
    ///
    /// Lazy exactly as `pathlib` is: the walk runs as the iterator is drained,
    /// so taking three entries from a folder of a hundred thousand costs three.
    /// Private entries - names beginning with a dot - are skipped unless
    /// `include_private` asks for them.
    #[pyo3(signature = (include_private = false))]
    fn iterdir(&self, include_private: bool) -> PyResult<PyIOBaseIterator> {
        Ok(PyIOBaseIterator {
            entries: self.inner()?.ls(false, include_private),
        })
    }

    /// List the children, optionally descending, as the core `ls`.
    ///
    /// Lazy: see `iterdir`.
    #[pyo3(signature = (recursive = false, include_private = false))]
    fn ls(&self, recursive: bool, include_private: bool) -> PyResult<PyIOBaseIterator> {
        Ok(PyIOBaseIterator {
            entries: self.inner()?.ls(recursive, include_private),
        })
    }

    /// Expand a glob against this resource, as `Path.glob`.
    ///
    /// Lazy: a pattern whose fixed prefix names nothing touches nothing
    /// beneath it, because the walk only starts on the first `next`.
    #[pyo3(signature = (pattern, include_private = false))]
    fn glob(&self, pattern: &str, include_private: bool) -> PyResult<PyIOBaseIterator> {
        Ok(PyIOBaseIterator {
            entries: self
                .inner()?
                .glob(pattern, include_private)
                .map_err(crate::holder::fs::storage_error)?,
        })
    }

    /// Expand a glob at any depth, as `Path.rglob`.
    #[pyo3(signature = (pattern, include_private = false))]
    fn rglob(&self, pattern: &str, include_private: bool) -> PyResult<PyIOBaseIterator> {
        self.glob(&format!("**/{pattern}"), include_private)
    }

    /// The Hive partition pairs this resource's location spells out.
    #[getter]
    fn partitions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.inner()?.partitions())
    }

    /// Iterate the entries beneath this one a predicate does not rule out.
    ///
    /// The predicate is asked of the holder, not of the rows: `&holder.name`,
    /// `&holder.partition['year']`, `&holder.size`. A conjunct that reads a
    /// row column cannot be answered by a listing, so it is dropped rather
    /// than guessed at - this may keep a file the rows later discard and can
    /// never discard one they would have kept.
    ///
    /// `filter` is an `Expression` or the text of one, which parses.
    #[pyo3(signature = (filter, include_private = false))]
    fn children_matching(
        &self,
        filter: &Bound<'_, PyAny>,
        include_private: bool,
    ) -> PyResult<PyIOBaseIterator> {
        let filter = crate::expression::expression_from_value(filter)?;
        Ok(PyIOBaseIterator {
            entries: self
                .inner()?
                .children_matching(&filter, include_private)
                .map_err(crate::holder::fs::storage_error)?,
        })
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
    ) -> PyResult<PyIOBaseIterator> {
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
        Ok(PyIOBaseIterator {
            entries: self
                .inner()?
                .children_where(&borrowed, include_private)
                .map_err(crate::holder::fs::storage_error)?,
        })
    }

    /// Open a random-access native Arrow input file.
    fn open_input_file<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        if let Some((filesystem, path)) = slf.borrow().arrow_binding(py) {
            let filesystem = filesystem.bind(py);
            return filesystem
                .call_method1("open_input_file", (&path,))
                .map_err(|error| {
                    crate::holder::fs::direct_file_error(py, filesystem, &error, &path)
                });
        }
        Self::native_python_file(slf, 0, "r")
    }

    /// Open a sequential native Arrow input stream.
    #[pyo3(signature = (compression = Some("detect"), buffer_size = None))]
    fn open_input_stream<'py>(
        slf: &Bound<'py, Self>,
        compression: Option<&str>,
        buffer_size: Option<i64>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let kwargs = PyDict::new(py);
        match compression {
            Some(compression) => kwargs.set_item("compression", compression)?,
            None => kwargs.set_item("compression", py.None())?,
        }
        match buffer_size {
            Some(size) => kwargs.set_item("buffer_size", size)?,
            None => kwargs.set_item("buffer_size", py.None())?,
        }
        if let Some((filesystem, path)) = slf.borrow().arrow_binding(py) {
            let filesystem = filesystem.bind(py);
            return filesystem
                .call_method("open_input_stream", (&path,), Some(&kwargs))
                .map_err(|error| {
                    crate::holder::fs::direct_file_error(py, filesystem, &error, &path)
                });
        }
        let stream = Self::native_python_file(slf, 0, "r")?;
        py.import("pyarrow")?
            .getattr("input_stream")?
            .call((stream,), Some(&kwargs))
    }

    /// Open a truncating native Arrow output stream.
    #[pyo3(signature = (compression = Some("detect"), buffer_size = None, metadata = None))]
    fn open_output_stream<'py>(
        slf: &Bound<'py, Self>,
        compression: Option<&str>,
        buffer_size: Option<i64>,
        metadata: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let kwargs = PyDict::new(py);
        match compression {
            Some(compression) => kwargs.set_item("compression", compression)?,
            None => kwargs.set_item("compression", py.None())?,
        }
        match buffer_size {
            Some(size) => kwargs.set_item("buffer_size", size)?,
            None => kwargs.set_item("buffer_size", py.None())?,
        }
        match metadata {
            Some(metadata) => kwargs.set_item("metadata", metadata)?,
            None => kwargs.set_item("metadata", py.None())?,
        }
        if let Some((filesystem, path)) = slf.borrow().arrow_binding(py) {
            let filesystem = filesystem.bind(py);
            return filesystem
                .call_method("open_output_stream", (&path,), Some(&kwargs))
                .map_err(|error| {
                    crate::holder::fs::direct_file_error(py, filesystem, &error, &path)
                });
        }
        if metadata.is_some() {
            return Err(crate::holder::fs::storage_error(
                yggdryl::Error::unsupported("output metadata", "native IOBase"),
            ));
        }
        kwargs.del_item("metadata")?;
        let stream = Self::native_python_file(slf, 0, "w")?;
        let output = py
            .import("pyarrow")?
            .getattr("output_stream")?
            .call((stream,), Some(&kwargs))?;
        if let Err(error) = slf
            .borrow_mut()
            .inner_mut()?
            .truncate(0)
            .map_err(crate::holder::fs::storage_error)
        {
            let _ = output.call_method0("close");
            return Err(error);
        }
        Ok(output)
    }

    /// Open a native Arrow append stream.
    #[pyo3(signature = (compression = Some("detect"), buffer_size = None, metadata = None))]
    fn open_append_stream<'py>(
        slf: &Bound<'py, Self>,
        compression: Option<&str>,
        buffer_size: Option<i64>,
        metadata: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let kwargs = PyDict::new(py);
        match compression {
            Some(compression) => kwargs.set_item("compression", compression)?,
            None => kwargs.set_item("compression", py.None())?,
        }
        match buffer_size {
            Some(size) => kwargs.set_item("buffer_size", size)?,
            None => kwargs.set_item("buffer_size", py.None())?,
        }
        match metadata {
            Some(metadata) => kwargs.set_item("metadata", metadata)?,
            None => kwargs.set_item("metadata", py.None())?,
        }
        if let Some((filesystem, path)) = slf.borrow().arrow_binding(py) {
            let filesystem = filesystem.bind(py);
            return filesystem
                .call_method("open_append_stream", (&path,), Some(&kwargs))
                .map_err(|error| {
                    crate::holder::fs::direct_file_error(py, filesystem, &error, &path)
                });
        }
        if metadata.is_some() {
            return Err(crate::holder::fs::storage_error(
                yggdryl::Error::unsupported("append metadata", "native IOBase"),
            ));
        }
        kwargs.del_item("metadata")?;
        let position = {
            let handle = slf.borrow();
            match handle.inner()?.bound_location() {
                Some(bound) => bound
                    .filesystem()
                    .file_info(bound.path())
                    .map_err(crate::holder::fs::storage_error)?
                    .size
                    .unwrap_or(0),
                None => handle.inner()?.size(),
            }
        };
        let stream = Self::native_python_file(slf, position, "w")?;
        py.import("pyarrow")?
            .getattr("output_stream")?
            .call((stream,), Some(&kwargs))
    }

    /// Read every byte here, as `Path.read_bytes`.
    ///
    /// A resource that does not exist reads as empty rather than raising, per
    /// the laziness contract.
    fn read_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self
            .inner()?
            .read_all_bytes()
            .map_err(crate::holder::fs::storage_error)?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Digest every byte here, without holding the value in memory.
    ///
    /// The read streams in bounded chunks, so a multi-gigabyte object costs
    /// one window rather than a copy. A resource that does not exist digests
    /// as empty, per the laziness contract; a container raises.
    #[pyo3(signature = (algorithm = "xxh3-64"))]
    fn read_digest(&self, py: Python<'_>, algorithm: &str) -> PyResult<crate::xxhash::PyDigest> {
        let algorithm = crate::xxhash::algorithm_from_str(algorithm)?;
        let inner = self.inner()?;
        let digest = py
            .detach(|| inner.read_digest(algorithm))
            .map_err(crate::holder::fs::storage_error)?;
        Ok(crate::xxhash::PyDigest::from_core(digest))
    }

    /// Digest `length` bytes from `offset`, streaming the window.
    #[pyo3(signature = (offset, length, algorithm = "xxh3-64"))]
    fn read_range_digest(
        &self,
        py: Python<'_>,
        offset: u64,
        length: usize,
        algorithm: &str,
    ) -> PyResult<crate::xxhash::PyDigest> {
        let algorithm = crate::xxhash::algorithm_from_str(algorithm)?;
        let inner = self.inner()?;
        let digest = py
            .detach(|| inner.read_range_digest(offset, length, algorithm))
            .map_err(crate::holder::fs::storage_error)?;
        Ok(crate::xxhash::PyDigest::from_core(digest))
    }

    /// Read every byte here as text, as `Path.read_text`.
    fn read_text(&self) -> PyResult<String> {
        let bytes = self
            .inner()?
            .read_all_bytes()
            .map_err(crate::holder::fs::storage_error)?;
        String::from_utf8(bytes).map_err(|error| PyValueError::new_err(error.to_string()))
    }

    /// Decode inferred JSON, YAML, or TOML into natural Python or exact `Scalar`.
    #[pyo3(signature = (field = None, *, cls = None))]
    fn read_scalar(
        &self,
        py: Python<'_>,
        field: Option<&Bound<'_, PyAny>>,
        cls: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let native_scalar = match cls {
            None => false,
            Some(cls) if cls.is(py.get_type::<PyScalar>()) => true,
            Some(_) => return Err(PyTypeError::new_err("cls must be Scalar or None")),
        };
        let field = field.map(core_field_from_value).transpose()?;
        let value = self
            .inner()?
            .read_scalar(field.as_ref())
            .map_err(crate::holder::fs::storage_error)?;
        decoded_into_py(py, value, field.as_ref(), native_scalar)
    }

    /// Replace what is here with `data`, as `Path.write_bytes`.
    fn write_bytes(&mut self, data: &[u8]) -> PyResult<usize> {
        self.inner_mut()?
            .write_all_bytes(data)
            .map_err(crate::holder::fs::storage_error)?;
        Ok(data.len())
    }

    /// Replace what is here with `text`, as `Path.write_text`.
    fn write_text(&mut self, text: &str) -> PyResult<usize> {
        self.write_bytes(text.as_bytes())
    }

    /// Encode one Python value as inferred JSON, YAML, or TOML.
    fn write_scalar(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner_mut()?
            .write_scalar(&from_py(value)?)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Read `length` bytes from `offset`, which `pathlib` cannot do.
    ///
    /// The core's `read_range_bytes` under its own name: the ranged half of
    /// the pair `read_bytes` reads whole.
    fn read_range_bytes<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self
            .inner()?
            .read_range_bytes(offset, length)
            .map_err(crate::holder::fs::storage_error)?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Read `length` bytes from `offset` as `cls`.
    ///
    /// The inferring entry point over the one core range read: `bytes` - the
    /// default - is `read_range_bytes` itself, and `str` decodes that same
    /// range as UTF-8, exactly as `read_text` decodes what `read_bytes` reads.
    #[pyo3(signature = (offset, length, *, cls = None))]
    fn read_range<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
        cls: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        // Settled before the read, so a rejected `cls` costs no fetch and the
        // caller's real mistake is what gets raised.
        let text = match cls {
            None => false,
            Some(cls) if cls.is(py.get_type::<PyBytes>()) => false,
            Some(cls) if cls.is(py.get_type::<PyString>()) => true,
            Some(_) => return Err(PyTypeError::new_err("cls must be bytes, str, or None")),
        };
        // The core answer is read once and becomes exactly one Python object:
        // the text path never materializes the `bytes` it would discard.
        let bytes = self
            .inner()?
            .read_range_bytes(offset, length)
            .map_err(crate::holder::fs::storage_error)?;
        if !text {
            return Ok(PyBytes::new(py, &bytes).into_any());
        }
        let decoded =
            String::from_utf8(bytes).map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(PyString::new(py, &decoded).into_any())
    }

    /// Stream byte arrays from an explicit position without retaining pages.
    ///
    /// The iterator is lazy and fused after its first read failure. Arrays are
    /// `batch_size` bytes except for the final short one; no empty array is
    /// yielded. The iterator keeps this handle alive for as long as it reads.
    #[pyo3(signature = (position = 0, batch_size = 65536))]
    fn pstream_bytes(
        slf: &Bound<'_, Self>,
        position: u64,
        batch_size: usize,
    ) -> PyResult<PyByteIterator> {
        if batch_size == 0 {
            return Err(PyValueError::new_err(
                "batch_size must be greater than zero",
            ));
        }
        if let Some(bound) = slf.borrow().inner()?.bound_location() {
            let mut reader = match bound.filesystem().open_input_file(bound.path()) {
                Ok(reader) => reader,
                Err(error) if error.is_absent() => {
                    return Ok(PyByteIterator {
                        source: PyByteSource::Empty,
                        batch_size,
                        done: false,
                    });
                }
                Err(error) => return Err(crate::holder::fs::storage_error(error)),
            };
            reader
                .seek(std::io::SeekFrom::Start(position))
                .map_err(crate::holder::fs::storage_error)?;
            return Ok(PyByteIterator {
                source: PyByteSource::Reader {
                    reader,
                    cursor: None,
                },
                batch_size,
                done: false,
            });
        }
        // Validate through the core without touching the source.
        drop(
            slf.borrow()
                .inner()?
                .pstream_bytes(position, batch_size)
                .map_err(crate::holder::fs::storage_error)?,
        );
        Ok(PyByteIterator {
            source: PyByteSource::Position {
                handle: slf.clone().unbind(),
                position,
            },
            batch_size,
            done: false,
        })
    }

    /// Write `data` at `offset`, growing and zero-filling as needed.
    ///
    /// Filesystem-backed writes use the bound filesystem's stream capability;
    /// the Python binding does not retain or assemble the object in memory.
    fn pwrite(&mut self, offset: u64, data: &[u8]) -> PyResult<usize> {
        self.inner_mut()?
            .pwrite(offset, data)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Append `data` after the last byte, returning the offset it landed at.
    ///
    /// The core's `append_bytes` under its own name; `append` is the inferring
    /// entry point that also takes text and the other buffer types.
    fn append_bytes(&mut self, data: &[u8]) -> PyResult<u64> {
        self.inner_mut()?
            .append_bytes(data)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Append bytes or UTF-8 text after the last byte, returning its offset.
    ///
    /// `bytes`, `bytearray`, `memoryview`, and `str` all name the same core
    /// append: the buffer is read once here and redirected to `append_bytes`.
    fn append(&mut self, data: &Bound<'_, PyAny>) -> PyResult<u64> {
        if let Ok(text) = data.cast::<PyString>() {
            let text = text.to_cow()?;
            return self.append_bytes(text.as_bytes());
        }
        with_python_bytes(
            data,
            "appended data must be bytes, bytearray, memoryview, or str",
            |bytes| self.append_bytes(bytes),
        )
    }

    /// Create this bound directory with Arrow's explicit recursive policy.
    ///
    /// The container it created is what comes back, because a role is what a
    /// handle's class says it is: this handle still addresses the location it
    /// always did, and the returned `FsFolder` is the one that reads it as a
    /// directory.
    #[pyo3(signature = (recursive = false))]
    fn create_dir(&mut self, py: Python<'_>, recursive: bool) -> PyResult<Py<PyAny>> {
        let bound = self.bound()?.clone();
        bound
            .filesystem()
            .create_dir(bound.path(), recursive)
            .map_err(crate::holder::fs::storage_error)?;
        describe(
            py,
            Holder::FsFolder(yggdryl::holder::fs::Folder::new(bound)),
        )
    }

    /// Delete this empty directory itself.
    fn delete_dir(&mut self) -> PyResult<()> {
        let bound = self.bound()?.clone();
        bound
            .filesystem()
            .delete_dir(bound.path())
            .map_err(crate::holder::fs::storage_error)
    }

    /// Delete descendants while retaining this directory.
    #[pyo3(signature = (missing_dir_ok = false))]
    fn delete_dir_contents(&mut self, missing_dir_ok: bool) -> PyResult<()> {
        let bound = self.bound()?.clone();
        bound
            .filesystem()
            .delete_dir_contents(bound.path(), missing_dir_ok)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Delete all filesystem-root children while retaining its root.
    fn delete_root_dir_contents(&mut self) -> PyResult<()> {
        yggdryl::holder::fs::Folder::new(self.bound()?.clone())
            .delete_root_dir_contents()
            .map_err(crate::holder::fs::storage_error)
    }

    /// Delete this file. Directories are rejected by the backend.
    fn delete_file(&mut self) -> PyResult<()> {
        let bound = self.bound()?.clone();
        bound
            .filesystem()
            .delete_file(bound.path())
            .map_err(crate::holder::fs::storage_error)
    }

    /// Create this resource as a container, as `Path.mkdir`.
    ///
    /// Parents are created too, and an existing container is left alone, which
    /// is `mkdir(parents=True, exist_ok=True)`. The container is what comes
    /// back: a plain byte write here would have made this location a file, so
    /// the handle that reads it as a directory is a different role, and a role
    /// is what a handle's class says it is.
    fn mkdir(&mut self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        // A handle on a foreign filesystem becomes a container on that
        // filesystem. Rebuilding from the location alone would silently move
        // the handle to the local disk, because a location does not say which
        // backend it belongs to.
        let mut folder = if let Some(holder) = fs_folder_holder(self.inner()?) {
            holder
        } else {
            let url = self.inner()?.url().ok_or_else(|| {
                PyValueError::new_err("an in-memory resource cannot become a directory")
            })?;
            folder_holder_for(url)?
        };
        if let Some(bound) = folder.bound_location() {
            bound
                .filesystem()
                .create_dir(bound.path(), true)
                .map_err(crate::holder::fs::storage_error)?;
        } else {
            folder.truncate(0).map_err(value_error)?;
        }
        describe(py, folder)
    }

    /// Create this resource as an empty leaf, as `Path.touch`.
    ///
    /// An existing leaf keeps its bytes, as `touch` does.
    fn touch(&mut self) -> PyResult<()> {
        // An empty positional write is the non-truncating act: it creates a
        // missing leaf, preserves an existing value, and lets a directory
        // reject the write itself. No existence or kind probe races the act.
        self.inner_mut()?
            .pwrite(0, b"")
            .map_err(|error| match error {
                yggdryl::Error::Io(error) if error.kind() == std::io::ErrorKind::IsADirectory => {
                    PyIsADirectoryError::new_err(error.to_string())
                }
                error => crate::holder::fs::storage_error(error),
            })?;
        self.inner_mut()?
            .flush()
            .map_err(crate::holder::fs::storage_error)
    }

    /// Delete the resource here, as `Path.unlink` on a leaf.
    ///
    /// A thin spelling of `remove(recursive=False)` under the name `pathlib`
    /// uses; unlike `pathlib`'s, a resource that is not there is not an error,
    /// because absence is a no-op success everywhere on this handle.
    fn unlink(&mut self) -> PyResult<()> {
        if self.inner()?.bound_location().is_some() {
            self.delete_file()
        } else {
            self.inner_mut()?
                .remove(false)
                .map_err(crate::holder::fs::storage_error)
        }
    }

    /// Empty the contents, keeping the resource.
    ///
    /// A leaf keeps existing with `size` 0; a directory keeps existing and is
    /// emptied of every child, recursively; a resource that is not there is
    /// left alone. Nothing is created.
    fn clear(&mut self) -> PyResult<()> {
        self.inner_mut()?
            .clear()
            .map_err(crate::holder::fs::storage_error)
    }

    /// Delete the resource completely.
    ///
    /// After this returns nothing of what the handle addressed remains - the
    /// bytes, the tree below a directory, and any cached schema or footer. A
    /// leaf ignores `recursive`. A directory needs `recursive=True` to delete
    /// anything below it; without it, a directory that still has children
    /// raises rather than silently succeeding or silently recursing. A
    /// resource that is not there succeeds, having done nothing - absence and
    /// successful removal are indistinguishable by design.
    ///
    /// The handle stays usable afterwards: writing through it recreates the
    /// resource exactly as a fresh handle would.
    #[pyo3(signature = (recursive = false))]
    fn remove(&mut self, recursive: bool) -> PyResult<()> {
        self.inner_mut()?
            .remove(recursive)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Cut this resource to `size` bytes.
    fn truncate(&mut self, size: u64) -> PyResult<()> {
        self.inner_mut()?
            .truncate(size)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Read exactly `length` bytes from `offset`, or refuse naming the shortfall.
    ///
    /// `read_range_bytes` answers what exists; this is the form that treats a
    /// short resource as an error, which is what a fixed-width header or a
    /// footer of known length wants.
    fn pread_exact<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut buffer = vec![0_u8; length];
        self.inner()?
            .pread_exact(offset, &mut buffer)
            .map_err(crate::holder::fs::storage_error)?;
        Ok(PyBytes::new(py, &buffer))
    }

    /// Write every byte at `offset`, or refuse naming the shortfall.
    ///
    /// `pwrite` answers how many bytes landed; this loops until they all do,
    /// which is what a caller placing a whole record at a known offset means.
    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> PyResult<()> {
        self.inner_mut()?
            .pwrite_all(offset, data)
            .map_err(crate::holder::fs::storage_error)
    }

    /// The allocation behind this resource, never less than `size`.
    ///
    /// A memory-mapped local file grows its mapping geometrically, so its
    /// capacity outruns the size a flush publishes; an in-memory buffer
    /// answers what it has room for before it grows again.
    #[getter]
    fn capacity(&self) -> PyResult<u64> {
        Ok(self.inner()?.capacity())
    }

    /// Grow the allocation so at least `capacity` bytes fit without a growth.
    ///
    /// Reserving never shrinks and never changes `size`, so it is what a
    /// caller with a known final length asks before writing. Like every other
    /// write it creates the resource if naming it did not.
    fn reserve(&mut self, capacity: u64) -> PyResult<()> {
        self.inner_mut()?
            .reserve(capacity)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Flush anything buffered, as `IOBase.flush`.
    fn flush(&mut self) -> PyResult<()> {
        self.inner_mut()?
            .flush()
            .map_err(crate::holder::fs::storage_error)
    }

    /// Put this handle behind the core's bounded page cache.
    ///
    /// The cached handle is what comes back, as a `Buffered`, and this handle
    /// is spent: a wrapper owns the handle it wraps, so the value moves into
    /// the one the call answers with. Repeating the call replaces the cache
    /// options around the same held resource; it never stacks a second cache.
    #[pyo3(signature = (*, page_size = None, max_bytes = None, ttl = None))]
    fn buffered(
        &mut self,
        py: Python<'_>,
        page_size: Option<usize>,
        max_bytes: Option<u64>,
        ttl: Option<f64>,
    ) -> PyResult<Py<PyAny>> {
        let mut options = BufferedOptions::default();
        if let Some(page_size) = page_size {
            options = options.with_page_size(page_size);
        }
        if let Some(max_bytes) = max_bytes {
            options = options.with_max_bytes(max_bytes);
        }
        if let Some(ttl) = ttl {
            if !ttl.is_finite() || ttl < 0.0 {
                return Err(PyValueError::new_err(
                    "ttl must be a finite non-negative number of seconds",
                ));
            }
            options = options.with_ttl(std::time::Duration::from_secs_f64(ttl));
        }

        // Options are validated before the value is taken, so a rejected
        // argument never leaves this handle spent.
        describe(py, self.take()?.buffered(options))
    }

    /// Retain this handle as plain-text record media.
    ///
    /// The text handle is what comes back, and this handle is spent, as it is
    /// for every wrapper. Repeating the call replaces explicit options without
    /// stacking another text wrapper.
    #[pyo3(signature = (options = None))]
    #[allow(clippy::wrong_self_convention)] // A pyclass method cannot consume its receiver.
    fn into_text(
        &mut self,
        py: Python<'_>,
        options: Option<PyRef<'_, PyTextOptions>>,
    ) -> PyResult<Py<PyAny>> {
        let held = self.take()?;
        describe(
            py,
            match options {
                Some(options) => held.into_text_with(options.inner.clone()),
                None => held.into_text(),
            },
        )
    }

    /// Retain this handle behind the content coding its name declares.
    ///
    /// The handle then presents *decoded* bytes:
    /// `IOBase("app.log.gz").into_coded()` reads as the log it holds, reports
    /// `text/plain`, and reads its records without anything naming gzip. The
    /// conversion is lazy and every read streams in bounded windows, so a
    /// multi-gigabyte archive costs one window rather than its decoded size.
    ///
    /// `codec` overrides what the name declares - `"gzip"`, `"zlib"`,
    /// `"deflate"`, `"zstd"`, `"identity"` - and `level` is the shared 0-9
    /// scale writes encode at. A name declaring no coding passes its bytes
    /// through unchanged.
    ///
    /// The coded handle is what comes back - a `Gzip`, `Zlib`, `Zstd`, or
    /// `Identity` - and this handle is spent, as it is for every wrapper. A
    /// handle already presenting decoded bytes keeps the coding it has, so
    /// repeating the call never decodes twice.
    #[pyo3(signature = (codec = None, level = None))]
    #[allow(clippy::wrong_self_convention)] // A pyclass method cannot consume its receiver.
    fn into_coded(
        &mut self,
        py: Python<'_>,
        codec: Option<&str>,
        level: Option<u8>,
    ) -> PyResult<Py<PyAny>> {
        let codec = match codec {
            Some(name) => name.parse::<Codec>().map_err(value_error)?,
            None => self.inner()?.codec(),
        };
        let level = level.map_or(Level::DEFAULT, Level::new);
        // The codec is parsed before the value is taken, so a rejected
        // argument never leaves this handle spent.
        describe(py, self.take()?.into_coded_with(codec, level))
    }

    /// Retain the record encoding this resource turns out to hold.
    ///
    /// `into_text` and `into_coded` name what the wrapper is; this asks the
    /// resource. An unresolved location is asked of the store, so a name that
    /// says nothing still answers the encoding its bytes are in - and a
    /// resource that is not a record encoding answers itself.
    ///
    /// The handle is spent, the way every other wrapper conversion spends it.
    #[allow(clippy::wrong_self_convention)] // A pyclass method cannot consume its receiver.
    fn into_media(&mut self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        describe(py, self.take()?.into_media())
    }

    /// Materialize the resource and cache what repeated calls would re-derive.
    ///
    /// A handle works without this - every operation materializes what it needs
    /// - so calling it moves that cost to a known point. Opening a resource
    /// that does not exist yet succeeds without creating it.
    fn open(&mut self) -> PyResult<()> {
        // The trait method, deliberately: the inherent `Holder::open` promotes
        // the holder into its record media first, and a handle whose variant
        // changed under it would report a class it no longer is. Construction
        // already composed what the name declares, so there is nothing left to
        // promote.
        yggdryl::IOBase::open(self.inner_mut()?).map_err(crate::holder::fs::storage_error)
    }

    /// Return whether cached state is currently held.
    ///
    /// A property rather than a method, because `io.IOBase.closed` is one and
    /// this module mirrors that vocabulary.
    #[getter]
    fn opened(&self) -> PyResult<bool> {
        Ok(self.inner()?.opened())
    }

    /// Return whether no cached state is currently held, as `io.IOBase.closed`.
    #[getter]
    fn closed(&self) -> PyResult<bool> {
        Ok(self.inner()?.closed())
    }

    /// Publish and release everything `open` cached.
    ///
    /// The handle stays usable afterwards; a later operation re-materializes.
    /// This is what publishes a written file at its exact length, which is why
    /// a `with` block is how a file meant for another reader is written.
    fn close(&mut self) -> PyResult<()> {
        self.inner_mut()?
            .close()
            .map_err(crate::holder::fs::storage_error)
    }

    /// Enter a scope, as `IOBase.open`.
    fn __enter__(mut slf: PyRefMut<'_, Self>) -> PyResult<PyRefMut<'_, Self>> {
        // The trait method, for the reason `open` gives.
        yggdryl::IOBase::open(slf.inner_mut()?).map_err(crate::holder::fs::storage_error)?;
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
        self.inner()?
            .copy_into(target.inner_mut()?)
            .map_err(crate::holder::fs::storage_error)
    }

    /// Move this file into `target`, using the backend's native move when equal.
    fn move_into(&mut self, py: Python<'_>, target: &mut Self) -> PyResult<Py<PyAny>> {
        let returned = target.rebuilt()?;
        self.inner_mut()?
            .move_into(target.inner_mut()?)
            .map_err(crate::holder::fs::storage_error)?;
        declared(py, returned)
    }

    /// Encode every byte here into `target`, returning the bytes written.
    ///
    /// `codec` names the coding - `"gzip"`, `"zlib"`, `"deflate"`, `"zstd"`,
    /// `"identity"` - and defaults to the one `target`'s own name declares, so
    /// `handle.compress_into(root / "rows.json.gz")` needs no second spelling
    /// of "gzip". A target that declares none is refused rather than silently
    /// copied, because a coding nobody named is a coding nobody can decode by
    /// name later. `level` is the shared 0-9 scale.
    ///
    /// The target's media type records the added coding, which is what lets
    /// [`decompress_into`][Self::decompress_into] undo this with no argument.
    #[pyo3(signature = (target, codec = None, level = None))]
    fn compress_into(
        &self,
        target: &mut Self,
        codec: Option<&str>,
        level: Option<u8>,
    ) -> PyResult<u64> {
        require_stored(target.inner()?, "a target")?;
        let codec = match codec {
            Some(name) => name.parse::<Codec>().map_err(value_error)?,
            None => match target.inner()?.codec() {
                Codec::Identity => {
                    return Err(PyValueError::new_err(format!(
                        "expected a target declaring a content coding, got {}; pass codec= to say \
                         which coding to write",
                        target.inner()?.media_type(),
                    )));
                }
                codec => codec,
            },
        };
        let level = level.map_or(Level::DEFAULT, Level::new);
        self.inner()?
            .compress_into_with_level(target.inner_mut()?, codec, level)
            .map_err(value_error)
    }

    /// Decode every byte here into `target`, returning the bytes written.
    ///
    /// `codec` defaults to [`self.codec`][Self::codec] - the coding this
    /// handle's own name declares - so a `.gz` file decodes into a plain one
    /// without the caller repeating what the name already said; naming a
    /// coding overrides that, for bytes whose name does not admit what they
    /// are. The target's media type loses the coding this removed.
    #[pyo3(signature = (target, codec = None))]
    fn decompress_into(&self, target: &mut Self, codec: Option<&str>) -> PyResult<u64> {
        require_stored(self.inner()?, "a source")?;
        require_stored(target.inner()?, "a target")?;
        match codec {
            Some(name) => {
                let codec = name.parse::<Codec>().map_err(value_error)?;
                self.inner()?
                    .decompress_into_with(target.inner_mut()?, codec)
                    .map_err(value_error)
            }
            None => self
                .inner()?
                .decompress_into(target.inner_mut()?)
                .map_err(value_error),
        }
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
            closed: std::sync::atomic::AtomicBool::new(false),
            reader: std::sync::Mutex::new(None),
            close_failure: std::sync::Mutex::new(None),
        }
    }

    /// The record settings for the encoding this handle's media type names.
    ///
    /// Every record method below defaults to exactly this, so the encoding is
    /// never guessed and never passed as a format argument.
    fn record_options(&self) -> PyResult<PyRecordOptions> {
        self.inner()?
            .record_options()
            .map(PyRecordOptions::from_core)
            .map_err(value_error)
    }

    /// Read one Parquet leaf's footer statistics without decoding rows.
    ///
    /// The core validates the handle's inferred media type before parsing its
    /// footer; this boundary only projects the shared `Scalar` into Python.
    fn read_parquet_statistics(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let statistics = self
            .inner()?
            .read_parquet_statistics()
            .map_err(value_error)?;
        decoded_as_py(py, &yggdryl::Scalar::from(statistics), None)
    }

    /// The row-group split offsets a Parquet footer records.
    ///
    /// This is what a table format writes as a data file's split points, so
    /// a reader can divide the file without decoding it.
    fn read_parquet_split_offsets(&self) -> PyResult<Vec<i64>> {
        Ok(self
            .inner()?
            .read_parquet_statistics()
            .map_err(value_error)?
            .split_offsets())
    }

    /// The null count one Parquet leaf column records across every row group.
    ///
    /// `None` when no row group recorded the statistic, which is not the same
    /// answer as zero: a missing statistic prunes nothing.
    fn read_parquet_null_count(&self, path: &str) -> PyResult<Option<u64>> {
        Ok(self
            .inner()?
            .read_parquet_statistics()
            .map_err(value_error)?
            .null_count(path))
    }

    /// Recompute one Parquet geospatial column's bounds and geometry types.
    ///
    /// This is the deliberate scan counterpart to `read_parquet_statistics`:
    /// the core projects and decodes only the named WKB column.
    fn read_parquet_geospatial_statistics(
        &self,
        py: Python<'_>,
        column: &str,
    ) -> PyResult<Py<PyAny>> {
        let statistics = self
            .inner()?
            .read_parquet_geospatial_statistics(column)
            .map_err(value_error)?;
        decoded_as_py(py, &yggdryl::Scalar::from(statistics), None)
    }

    /// Read the canonical non-null struct root `Field` of this resource.
    #[pyo3(signature = (*, options = None))]
    fn read_arrow_field(&self, options: Option<&Bound<'_, PyAny>>) -> PyResult<PyField> {
        let options = self.resolve_options(options)?;
        self.inner()?
            .read_arrow_field(&options)
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// Read this resource as a `pyarrow.RecordBatchReader`.
    ///
    /// A field on the options selects and casts during the read: the columns
    /// it names become the encoding's own projection, so the rest are skipped
    /// rather than read and discarded, and what comes back is the shape it
    /// declares. A handle addressing a folder reads across the partitions
    /// beneath it, so a caller never has to know which they addressed.
    #[pyo3(signature = (*, options = None))]
    fn read_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let options = self.resolve_options(options)?;
        self.read_reader(py, &options)
    }

    /// Replace this resource with the batches `reader` yields.
    ///
    /// This typed entry point accepts a `pyarrow.RecordBatchReader` or another
    /// Arrow C stream reader. Tables, held record batches, and row records use
    /// their dedicated adapters. The explicit method name is authoritative;
    /// `merge_by_names` never changes overwrite into merge.
    #[pyo3(signature = (reader, *, options = None))]
    fn overwrite_arrow_reader(
        &mut self,
        reader: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Overwrite, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_arrow_reader(reader)?;
        self.write_reader(batches, IOMode::Overwrite, &options)
    }

    /// Append the batches `reader` yields after this resource's stored rows.
    #[pyo3(signature = (reader, *, options = None))]
    fn append_arrow_reader(
        &mut self,
        reader: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Append, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_arrow_reader(reader)?;
        self.write_reader(batches, IOMode::Append, &options)
    }

    /// Merge the batches `reader` yields by the non-empty match key.
    #[pyo3(signature = (reader, *, options = None))]
    fn merge_arrow_reader(
        &mut self,
        reader: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Merge, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_arrow_reader(reader)?;
        self.write_reader(batches, IOMode::Merge, &options)
    }

    /// Write the batches `reader` yields using an explicit mode.
    ///
    /// The canonical argument order is input, mode, then optional settings.
    #[pyo3(signature = (reader, mode, *, options = None))]
    fn write_arrow_reader(
        &mut self,
        reader: &Bound<'_, PyAny>,
        mode: &str,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mode = IOMode::from_str(mode).map_err(value_error)?;
        let Some(options) = self.write_options(mode, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_arrow_reader(reader)?;
        self.write_reader(batches, mode, &options)
    }

    /// Replace this resource from exactly one `pyarrow.Table`.
    #[pyo3(signature = (table, *, options = None))]
    fn overwrite_arrow_table(
        &mut self,
        table: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Overwrite, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_arrow_table(table)?;
        self.write_reader(batches, IOMode::Overwrite, &options)
    }

    /// Append exactly one `pyarrow.Table` after this resource's rows.
    #[pyo3(signature = (table, *, options = None))]
    fn append_arrow_table(
        &mut self,
        table: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Append, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_arrow_table(table)?;
        self.write_reader(batches, IOMode::Append, &options)
    }

    /// Merge exactly one `pyarrow.Table` by `merge_by_names`.
    #[pyo3(signature = (table, *, options = None))]
    fn merge_arrow_table(
        &mut self,
        table: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Merge, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_arrow_table(table)?;
        self.write_reader(batches, IOMode::Merge, &options)
    }

    /// Write exactly one `pyarrow.Table` using an explicit mode.
    #[pyo3(signature = (table, mode, *, options = None))]
    fn write_arrow_table(
        &mut self,
        table: &Bound<'_, PyAny>,
        mode: &str,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mode = IOMode::from_str(mode).map_err(value_error)?;
        let Some(options) = self.write_options(mode, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_arrow_table(table)?;
        self.write_reader(batches, mode, &options)
    }

    /// Replace this resource from one held `pyarrow.RecordBatch`.
    #[pyo3(signature = (batch, *, options = None))]
    fn overwrite_arrow_batch(
        &mut self,
        batch: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Overwrite, options)? else {
            return Ok(());
        };
        let batch = record_batch_from_value(batch)?;
        self.inner_mut()?
            .write_arrow_batch(batch, IOMode::Overwrite, &options)
            .map_err(value_error)
    }

    /// Append one held `pyarrow.RecordBatch` after this resource's rows.
    #[pyo3(signature = (batch, *, options = None))]
    fn append_arrow_batch(
        &mut self,
        batch: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Append, options)? else {
            return Ok(());
        };
        let batch = record_batch_from_value(batch)?;
        self.inner_mut()?
            .write_arrow_batch(batch, IOMode::Append, &options)
            .map_err(value_error)
    }

    /// Merge one held `pyarrow.RecordBatch` by `merge_by_names`.
    #[pyo3(signature = (batch, *, options = None))]
    fn merge_arrow_batch(
        &mut self,
        batch: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Merge, options)? else {
            return Ok(());
        };
        let batch = record_batch_from_value(batch)?;
        self.inner_mut()?
            .write_arrow_batch(batch, IOMode::Merge, &options)
            .map_err(value_error)
    }

    /// Write exactly one `pyarrow.RecordBatch` using an explicit mode.
    #[pyo3(signature = (batch, mode, *, options = None))]
    fn write_arrow_batch(
        &mut self,
        batch: &Bound<'_, PyAny>,
        mode: &str,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mode = IOMode::from_str(mode).map_err(value_error)?;
        let Some(options) = self.write_options(mode, options)? else {
            return Ok(());
        };
        let batch = record_batch_from_value(batch)?;
        self.inner_mut()?
            .write_arrow_batch(batch, mode, &options)
            .map_err(value_error)
    }

    /// Lazily yield this resource as plain mappings or dataclass instances.
    ///
    /// A requested stdlib or `@scalar` dataclass is instantiated one row at a
    /// time. When that class is decorated and `options.field` is absent, its
    /// cached `field()` drives projection and casting on the read.
    #[pyo3(signature = (cls = None, *, options = None))]
    fn read_records<'py>(
        &self,
        py: Python<'py>,
        cls: Option<&Bound<'py, PyAny>>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut options = self.resolve_options(options)?;
        if let Some(cls) = cls {
            let is_dataclass = cls.is_instance_of::<PyType>()
                && py
                    .import("dataclasses")?
                    .getattr("is_dataclass")?
                    .call1((cls,))?
                    .extract::<bool>()?;
            if !is_dataclass {
                return Err(PyTypeError::new_err(format!(
                    "expected a dataclass type or None, got {}",
                    cls.get_type().fully_qualified_name()?
                )));
            }
            if options.field().is_none() {
                let field = core_root_field_from_value(cls, options.name())?;
                options.set_field(field);
            }
        }
        let reader = self
            .inner()?
            .read_arrow_reader(&options)
            .map_err(value_error)?;
        let field =
            yggdryl::Field::from_arrow_schema("row", &reader.schema()).map_err(value_error)?;
        let from_dict = cls
            .map(|cls| {
                let from_dict = py.import("yggdryl.types._classes")?.getattr("from_dict")?;
                Ok::<_, PyErr>((from_dict.unbind(), cls.clone().unbind()))
            })
            .transpose()?;
        Py::new(
            py,
            PyRecordIterator {
                reader,
                field,
                from_dict,
                rows: yggdryl::Scalar::from_sequence([]),
                next: 0,
            },
        )
        .map(|iterator| iterator.into_bound(py).into_any())
    }

    /// Replace this resource from an iterable of Python row records.
    ///
    /// A decorated dataclass instance infers its class's cached
    /// `field()` when no field was declared. Empty input has no
    /// class to inspect and therefore requires `options.field`.
    #[pyo3(signature = (records, *, options = None))]
    fn overwrite_records(
        &mut self,
        records: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(mut options) = self.write_options(IOMode::Overwrite, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_records(records, &mut options)?;
        self.write_reader(batches, IOMode::Overwrite, &options)
    }

    /// Append an iterable of Python row records after this resource's rows.
    #[pyo3(signature = (records, *, options = None))]
    fn append_records(
        &mut self,
        records: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(mut options) = self.write_options(IOMode::Append, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_records(records, &mut options)?;
        self.write_reader(batches, IOMode::Append, &options)
    }

    /// Merge an iterable of Python row records by `merge_by_names`.
    #[pyo3(signature = (records, *, options = None))]
    fn merge_records(
        &mut self,
        records: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(mut options) = self.write_options(IOMode::Merge, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_records(records, &mut options)?;
        self.write_reader(batches, IOMode::Merge, &options)
    }

    /// Write an iterable of Python row records using an explicit mode.
    #[pyo3(signature = (records, mode, *, options = None))]
    fn write_records(
        &mut self,
        records: &Bound<'_, PyAny>,
        mode: &str,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mode = IOMode::from_str(mode).map_err(value_error)?;
        let Some(mut options) = self.write_options(mode, options)? else {
            return Ok(());
        };
        let batches = batch_reader_from_records(records, &mut options)?;
        self.write_reader(batches, mode, &options)
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
            .inner()?
            .read_arrow_reader(&options)
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
            .inner()?
            .read_arrow_reader(&options)
            .map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Pandas)
    }

    /// Replace this resource with a stream of `pandas` frames.
    ///
    /// `frames` is one frame or any iterable of them, and an iterable is
    /// consumed one frame at a time. Anything that is not a `pandas` frame is
    /// refused by name; Arrow readers, tables, batches, and row records use
    /// their correspondingly named adapters.
    #[pyo3(signature = (frames, *, options = None))]
    fn overwrite_pandas(
        &mut self,
        frames: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Overwrite, options)? else {
            return Ok(());
        };
        let batches = frames_batch_reader(frames, Frames::Pandas, &options)?;
        self.write_reader(batches, IOMode::Overwrite, &options)
    }

    /// Append a stream of `pandas` frames after this resource's rows.
    #[pyo3(signature = (frames, *, options = None))]
    fn append_pandas(
        &mut self,
        frames: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Append, options)? else {
            return Ok(());
        };
        let batches = frames_batch_reader(frames, Frames::Pandas, &options)?;
        self.write_reader(batches, IOMode::Append, &options)
    }

    /// Merge a stream of `pandas` frames by `merge_by_names`.
    #[pyo3(signature = (frames, *, options = None))]
    fn merge_pandas(
        &mut self,
        frames: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Merge, options)? else {
            return Ok(());
        };
        let batches = frames_batch_reader(frames, Frames::Pandas, &options)?;
        self.write_reader(batches, IOMode::Merge, &options)
    }

    /// Write a stream of pandas frames using an explicit mode.
    #[pyo3(signature = (frames, mode, *, options = None))]
    fn write_pandas(
        &mut self,
        frames: &Bound<'_, PyAny>,
        mode: &str,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mode = IOMode::from_str(mode).map_err(value_error)?;
        let Some(options) = self.write_options(mode, options)? else {
            return Ok(());
        };
        let batches = frames_batch_reader(frames, Frames::Pandas, &options)?;
        self.write_reader(batches, mode, &options)
    }

    /// Replace this resource with exactly one `pandas` frame.
    #[pyo3(signature = (frame, *, options = None))]
    fn overwrite_pandas_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Overwrite, options)? else {
            return Ok(());
        };
        let batches = frame_batch_reader(frame, Frames::Pandas)?;
        self.write_reader(batches, IOMode::Overwrite, &options)
    }

    /// Append exactly one `pandas` frame after this resource's rows.
    #[pyo3(signature = (frame, *, options = None))]
    fn append_pandas_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Append, options)? else {
            return Ok(());
        };
        let batches = frame_batch_reader(frame, Frames::Pandas)?;
        self.write_reader(batches, IOMode::Append, &options)
    }

    /// Merge exactly one `pandas` frame by `merge_by_names`.
    #[pyo3(signature = (frame, *, options = None))]
    fn merge_pandas_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Merge, options)? else {
            return Ok(());
        };
        let batches = frame_batch_reader(frame, Frames::Pandas)?;
        self.write_reader(batches, IOMode::Merge, &options)
    }

    /// Write exactly one pandas frame using an explicit mode.
    #[pyo3(signature = (frame, mode, *, options = None))]
    fn write_pandas_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        mode: &str,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mode = IOMode::from_str(mode).map_err(value_error)?;
        let Some(options) = self.write_options(mode, options)? else {
            return Ok(());
        };
        let batches = frame_batch_reader(frame, Frames::Pandas)?;
        self.write_reader(batches, mode, &options)
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
            .inner()?
            .read_arrow_reader(&options)
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
            .inner()?
            .read_arrow_reader(&options)
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
        let asked = options.is_some();
        let options = self.resolve_options(options)?;
        let polars = py.import("polars")?;
        // The fast path hands the file to polars, which knows nothing about
        // what this call was asked for - so it is only the same answer when
        // the caller asked for nothing. Anything else reads through the
        // native reader, which honours every field; a projection that arrives
        // as a lazy scan of the whole file is not a projection.
        if !asked && let Some((path, _)) = self.published_scan_target()? {
            return polars.call_method1("scan_parquet", (path,));
        }
        let reader = self
            .inner()?
            .read_arrow_reader(&options)
            .map_err(value_error)?;
        frame_from_reader(py, reader, Frames::Polars)?.call_method0("lazy")
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
        let asked = options.is_some();
        let options = self.resolve_options(options)?;
        let dataset = py.import("pyarrow.dataset")?;
        // Same rule as `scan_polars`: the dataset scanner is handed the file
        // and nothing else, so it can only stand in for this call when the
        // call carried no options of its own.
        if !asked && let Some((path, _)) = self.published_scan_target()? {
            let arguments = pyo3::types::PyDict::new(py);
            arguments.set_item("format", "parquet")?;
            let opened = dataset.call_method("dataset", (path,), Some(&arguments))?;
            return opened.call_method0("scanner");
        }
        // A RecordBatchReader carries its own schema, and the scanner takes
        // it as it stands.
        let reader = self.read_reader(py, &options)?;
        dataset
            .getattr("Scanner")?
            .call_method1("from_batches", (reader,))
    }

    /// Replace this resource with a stream of `polars` frames.
    ///
    /// A `polars.LazyFrame` is accepted and collected, because polars offers no
    /// way to hand its rows over a batch at a time.
    #[pyo3(signature = (frames, *, options = None))]
    fn overwrite_polars(
        &mut self,
        frames: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Overwrite, options)? else {
            return Ok(());
        };
        let batches = frames_batch_reader(frames, Frames::Polars, &options)?;
        self.write_reader(batches, IOMode::Overwrite, &options)
    }

    /// Append a stream of `polars` frames after this resource's rows.
    #[pyo3(signature = (frames, *, options = None))]
    fn append_polars(
        &mut self,
        frames: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Append, options)? else {
            return Ok(());
        };
        let batches = frames_batch_reader(frames, Frames::Polars, &options)?;
        self.write_reader(batches, IOMode::Append, &options)
    }

    /// Merge a stream of `polars` frames by `merge_by_names`.
    #[pyo3(signature = (frames, *, options = None))]
    fn merge_polars(
        &mut self,
        frames: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Merge, options)? else {
            return Ok(());
        };
        let batches = frames_batch_reader(frames, Frames::Polars, &options)?;
        self.write_reader(batches, IOMode::Merge, &options)
    }

    /// Write a stream of polars frames using an explicit mode.
    #[pyo3(signature = (frames, mode, *, options = None))]
    fn write_polars(
        &mut self,
        frames: &Bound<'_, PyAny>,
        mode: &str,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mode = IOMode::from_str(mode).map_err(value_error)?;
        let Some(options) = self.write_options(mode, options)? else {
            return Ok(());
        };
        let batches = frames_batch_reader(frames, Frames::Polars, &options)?;
        self.write_reader(batches, mode, &options)
    }

    /// Replace this resource with exactly one `polars` frame.
    #[pyo3(signature = (frame, *, options = None))]
    fn overwrite_polars_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Overwrite, options)? else {
            return Ok(());
        };
        let batches = frame_batch_reader(frame, Frames::Polars)?;
        self.write_reader(batches, IOMode::Overwrite, &options)
    }

    /// Append exactly one `polars` frame after this resource's rows.
    #[pyo3(signature = (frame, *, options = None))]
    fn append_polars_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Append, options)? else {
            return Ok(());
        };
        let batches = frame_batch_reader(frame, Frames::Polars)?;
        self.write_reader(batches, IOMode::Append, &options)
    }

    /// Merge exactly one `polars` frame by `merge_by_names`.
    #[pyo3(signature = (frame, *, options = None))]
    fn merge_polars_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let Some(options) = self.write_options(IOMode::Merge, options)? else {
            return Ok(());
        };
        let batches = frame_batch_reader(frame, Frames::Polars)?;
        self.write_reader(batches, IOMode::Merge, &options)
    }

    /// Write exactly one polars frame using an explicit mode.
    #[pyo3(signature = (frame, mode, *, options = None))]
    fn write_polars_frame(
        &mut self,
        frame: &Bound<'_, PyAny>,
        mode: &str,
        options: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let mode = IOMode::from_str(mode).map_err(value_error)?;
        let Some(options) = self.write_options(mode, options)? else {
            return Ok(());
        };
        let batches = frame_batch_reader(frame, Frames::Polars)?;
        self.write_reader(batches, mode, &options)
    }

    /// The location as text, so `str(handle)` names it.
    fn __fspath__(&self) -> PyResult<String> {
        if let Some(bound) = self.inner()?.bound_location() {
            return Ok(bound.path().to_owned());
        }
        self.inner()?
            .url()
            .ok_or_else(|| PyValueError::new_err("this resource has no file system path"))?
            .clone()
            .into_path()
            .map_err(value_error)
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn __len__(&self) -> PyResult<usize> {
        Ok(usize::try_from(self.inner()?.size()).unwrap_or(usize::MAX))
    }

    fn __iter__(&self) -> PyResult<PyIOBaseIterator> {
        self.iterdir(false)
    }

    fn __str__(&self) -> PyResult<String> {
        let inner = self.inner()?;
        if let Some(bound) = inner.bound_location() {
            return Ok(bound.to_string());
        }
        Ok(inner
            .url()
            .map_or_else(|| "<memory>".to_owned(), ToString::to_string))
    }

    fn __repr__(&self) -> PyResult<String> {
        let location = format!("{:?}", self.__str__()?);
        Ok(stack(self.inner()?, &location))
    }
}

/// The iterator `for entry in handle`, `iterdir`, `ls`, and `glob` all walk.
///
/// It wraps the core listing directly, so nothing is collected on the way
/// across the boundary and a failure raises at the entry it happened on, after
/// which the iterator is exhausted.
#[pyclass(name = "Listing", module = "yggdryl._native")]
pub(crate) struct PyIOBaseIterator {
    entries: yggdryl::Listing,
}

#[pymethods]
impl PyIOBaseIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        let entry = self
            .entries
            .next()
            .transpose()
            .map_err(crate::holder::fs::storage_error)?;
        entry.map(|holder| declared(py, holder)).transpose()
    }
}

/// Lazy native iterator over a resource's rows as mappings or dataclasses.
///
/// One batch is lowered at a time through the core value boundary, so every
/// value crosses under its datatype - an ASCII width reads back trimmed, a
/// nested struct crosses as a mapping - and nothing binding-side reinterprets
/// storage. A requested dataclass is built from that mapping by
/// `yggdryl.types._classes.from_dict`, one row at a time.
#[pyclass(name = "RecordIterator", module = "yggdryl._native", unsendable)]
pub(crate) struct PyRecordIterator {
    reader: yggdryl::arrow::BatchReader,
    field: yggdryl::Field,
    from_dict: Option<(Py<PyAny>, Py<PyAny>)>,
    // The current batch's rows and the next one to hand out.
    rows: yggdryl::Scalar,
    next: usize,
}

#[pymethods]
impl PyRecordIterator {
    // Consumption changes reader state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        loop {
            if let Some(row) = self.rows.as_sequence().and_then(|rows| rows.get(self.next)) {
                self.next += 1;
                let record = crate::types::scalar::as_py_with_field(py, row, &self.field)?;
                return match &self.from_dict {
                    Some((from_dict, cls)) => from_dict.call1(py, (cls, record)).map(Some),
                    None => Ok(Some(record)),
                };
            }
            // The read and the lowering run without the GIL.
            let Some(rows) = py.detach(|| {
                self.reader.next().map(|batch| {
                    yggdryl::arrow::batch_to_value(&batch.map_err(value_error)?)
                        .map_err(value_error)
                })
            }) else {
                return Ok(None);
            };
            self.rows = rows?;
            self.next = 0;
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
    closed: std::sync::atomic::AtomicBool,
    reader: std::sync::Mutex<Option<Box<dyn yggdryl::holder::fs::RandomAccessReader>>>,
    close_failure: std::sync::Mutex<Option<crate::holder::fs::StickyFailure>>,
}

impl PyIOCursor {
    fn load(&self) -> u64 {
        self.position.load(std::sync::atomic::Ordering::Acquire)
    }

    fn store(&self, position: u64) {
        self.position
            .store(position, std::sync::atomic::Ordering::Release);
    }

    fn require_open(&self) -> PyResult<()> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            Err(PyValueError::new_err("I/O operation on closed file"))
        } else {
            Ok(())
        }
    }

    fn reader(
        &self,
    ) -> PyResult<std::sync::MutexGuard<'_, Option<Box<dyn yggdryl::holder::fs::RandomAccessReader>>>>
    {
        self.reader
            .lock()
            .map_err(|_| PyValueError::new_err("cursor reader lock is poisoned"))
    }

    fn close_failure(
        &self,
    ) -> PyResult<std::sync::MutexGuard<'_, Option<crate::holder::fs::StickyFailure>>> {
        self.close_failure
            .lock()
            .map_err(|_| PyValueError::new_err("cursor close lock is poisoned"))
    }

    fn read_buffer(&self, py: Python<'_>, wanted: usize) -> PyResult<Vec<u8>> {
        if wanted == 0 {
            return Ok(Vec::new());
        }
        let position = self.load();
        let bound = {
            let handle = self.handle.borrow(py);
            handle.inner()?.bound_location().cloned()
        };
        let mut buffer = Vec::new();
        buffer.try_reserve_exact(wanted).map_err(|error| {
            PyValueError::new_err(format!(
                "cannot allocate a {wanted}-byte cursor read: {error}"
            ))
        })?;
        buffer.resize(wanted, 0);
        let read = if let Some(bound) = bound {
            let mut slot = self.reader()?;
            if slot.is_none() {
                let mut reader = bound
                    .filesystem()
                    .open_input_file(bound.path())
                    .map_err(crate::holder::fs::storage_error)?;
                if let Err(error) = reader.seek(std::io::SeekFrom::Start(position)) {
                    let _ = reader.close();
                    return Err(crate::holder::fs::storage_error(error));
                }
                *slot = Some(reader);
            }
            let reader = slot
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("cursor reader was not initialized"))?;
            reader
                .read(&mut buffer)
                .map_err(crate::holder::fs::storage_error)?
        } else {
            self.handle
                .borrow(py)
                .inner()?
                .pread(position, &mut buffer)
                .map_err(crate::holder::fs::storage_error)?
        };
        if read > buffer.len() {
            return Err(PyValueError::new_err(format!(
                "backend reported reading {read} bytes into a {}-byte buffer",
                buffer.len()
            )));
        }
        buffer.truncate(read);
        let next = position
            .checked_add(read as u64)
            .ok_or_else(|| PyValueError::new_err("read position exceeds u64::MAX"))?;
        self.store(next);
        Ok(buffer)
    }

    fn read_to_end_buffer(&self, py: Python<'_>) -> PyResult<Vec<u8>> {
        const CHUNK: usize = 64 * 1024;
        let mut bytes = Vec::new();
        loop {
            let chunk = self.read_buffer(py, CHUNK)?;
            if chunk.is_empty() {
                return Ok(bytes);
            }
            bytes.try_reserve(chunk.len()).map_err(|error| {
                PyValueError::new_err(format!("cannot grow cursor read result: {error}"))
            })?;
            bytes.extend_from_slice(&chunk);
        }
    }

    fn write_slice(&self, py: Python<'_>, bytes: &[u8]) -> PyResult<u64> {
        if let Some(mut reader) = self.reader()?.take() {
            reader.close().map_err(crate::holder::fs::storage_error)?;
        }
        let mut handle = self.handle.borrow_mut(py);
        let position = self.load();
        let offered = u64::try_from(bytes.len())
            .map_err(|_| PyValueError::new_err("write length exceeds u64::MAX"))?;
        position
            .checked_add(offered)
            .ok_or_else(|| PyValueError::new_err("write position exceeds u64::MAX"))?;
        let written = handle
            .inner_mut()?
            .pwrite(position, bytes)
            .map_err(crate::holder::fs::storage_error)?;
        if written > bytes.len() {
            return Err(PyValueError::new_err(
                "backend reported writing beyond the supplied buffer",
            ));
        }
        let next = position
            .checked_add(written as u64)
            .ok_or_else(|| PyValueError::new_err("write position exceeds u64::MAX"))?;
        self.store(next);
        Ok(written as u64)
    }
}

#[pymethods]
impl PyIOCursor {
    // Position and the addressed resource both change over time.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The resource this cursor rides.
    ///
    /// The cursor owns only its position, so this is the way back to every
    /// question the whole resource answers - its media type, its listing, its
    /// records - without leaving the position behind.
    #[getter]
    fn handle(&self, py: Python<'_>) -> Py<PyIOBase> {
        self.handle.clone_ref(py)
    }

    /// The current position, in bytes from the start.
    fn tell(&self) -> PyResult<u64> {
        self.require_open()?;
        Ok(self.load())
    }

    /// The same position as an attribute, settable.
    #[getter]
    fn position(&self) -> u64 {
        self.load()
    }

    #[setter]
    fn set_position(&self, position: u64) -> PyResult<()> {
        self.require_open()?;
        if let Some(reader) = self.reader()?.as_mut() {
            reader
                .seek(std::io::SeekFrom::Start(position))
                .map_err(crate::holder::fs::storage_error)?;
        }
        self.store(position);
        Ok(())
    }

    /// Whether this file object has been closed.
    #[getter]
    fn closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Acquire)
    }

    fn readable(&self) -> PyResult<bool> {
        self.require_open()?;
        Ok(true)
    }

    fn writable(&self) -> PyResult<bool> {
        self.require_open()?;
        Ok(true)
    }

    fn seekable(&self) -> PyResult<bool> {
        self.require_open()?;
        Ok(true)
    }

    /// Move the position, as `io.IOBase.seek` moves one, returning it.
    #[pyo3(signature = (offset, whence = 0))]
    fn seek(&self, py: Python<'_>, offset: i64, whence: u8) -> PyResult<u64> {
        self.require_open()?;
        let from = match whence {
            0 => u64::try_from(offset)
                .map(std::io::SeekFrom::Start)
                .map_err(|_| PyValueError::new_err("a seek cannot land before the start"))?,
            1 => std::io::SeekFrom::Current(offset),
            2 => std::io::SeekFrom::End(offset),
            _ => {
                return Err(PyValueError::new_err(
                    "whence must be 0 (start), 1 (current), or 2 (end)",
                ));
            }
        };
        let bound = {
            let handle = self.handle.borrow(py);
            handle.inner()?.bound_location().cloned()
        };
        if let Some(bound) = bound {
            let mut slot = self.reader()?;
            if slot.is_none() {
                let mut reader = bound
                    .filesystem()
                    .open_input_file(bound.path())
                    .map_err(crate::holder::fs::storage_error)?;
                if let Err(error) = reader.seek(std::io::SeekFrom::Start(self.load())) {
                    let _ = reader.close();
                    return Err(crate::holder::fs::storage_error(error));
                }
                *slot = Some(reader);
            }
            let reader = slot
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("cursor reader was not initialized"))?;
            let target = reader
                .seek(from)
                .map_err(crate::holder::fs::storage_error)?;
            self.store(target);
            return Ok(target);
        }
        let origin = match whence {
            0 => 0,
            1 => self.load(),
            2 => self.handle.borrow(py).inner()?.size(),
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
        self.require_open()?;
        let buffer = match usize::try_from(size) {
            Ok(wanted) => self.read_buffer(py, wanted)?,
            Err(_) => self.read_to_end_buffer(py)?,
        };
        Ok(PyBytes::new(py, &buffer))
    }

    /// Read bytes into any writable Python buffer.
    fn readinto(&self, py: Python<'_>, buffer: &Bound<'_, PyAny>) -> PyResult<usize> {
        self.require_open()?;
        let view = PyBuffer::<u8>::get(buffer).map_err(|_| {
            PyTypeError::new_err("readinto() argument must be a writable contiguous byte buffer")
        })?;
        let capacity = view
            .as_mut_slice(py)
            .ok_or_else(|| {
                PyTypeError::new_err(
                    "readinto() argument must be a writable contiguous byte buffer",
                )
            })?
            .len();
        let bytes = self.read_buffer(py, capacity)?;
        let count = bytes.len();
        let target = view.as_mut_slice(py).ok_or_else(|| {
            PyTypeError::new_err("readinto() argument must be a writable contiguous byte buffer")
        })?;
        for (slot, byte) in target.iter().zip(bytes) {
            slot.set(byte);
        }
        Ok(count)
    }

    /// Stream byte arrays from the current position, advancing as consumed.
    #[pyo3(signature = (batch_size = 65536))]
    fn stream_bytes(slf: &Bound<'_, Self>, batch_size: usize) -> PyResult<PyByteIterator> {
        let cursor = slf.borrow();
        cursor.require_open()?;
        if batch_size == 0 {
            return Err(PyValueError::new_err(
                "batch_size must be greater than zero",
            ));
        }
        drop(cursor);
        Ok(PyByteIterator {
            source: PyByteSource::Cursor(slf.clone().unbind()),
            batch_size,
            done: false,
        })
    }

    /// Write at the position, advancing it, returning the bytes written.
    fn write(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<u64> {
        const WINDOW: usize = 64 * 1024;
        self.require_open()?;
        if let Ok(bytes) = data.cast::<PyBytes>() {
            return self.write_slice(py, bytes.as_bytes());
        }

        let byte_view = py
            .import("builtins")?
            .getattr("memoryview")?
            .call1((data,))
            .and_then(|view| view.call_method1("cast", ("B",)))
            .map_err(|_| {
                PyTypeError::new_err(
                    "write() argument must support the contiguous byte buffer protocol",
                )
            })?;
        let view = PyBuffer::<u8>::get(&byte_view).map_err(|_| {
            PyTypeError::new_err(
                "write() argument must support the contiguous byte buffer protocol",
            )
        })?;
        let cells = view.as_slice(py).ok_or_else(|| {
            PyTypeError::new_err("write() argument must be a contiguous byte buffer")
        })?;
        let length = u64::try_from(cells.len())
            .map_err(|_| PyValueError::new_err("write length exceeds u64::MAX"))?;
        self.load()
            .checked_add(length)
            .ok_or_else(|| PyValueError::new_err("write position exceeds u64::MAX"))?;
        let mut window = vec![0_u8; cells.len().min(WINDOW)];
        let mut total = 0_u64;
        for chunk in cells.chunks(WINDOW) {
            for (slot, cell) in window.iter_mut().zip(chunk) {
                *slot = cell.get();
            }
            let written = self.write_slice(py, &window[..chunk.len()])?;
            total += written;
            if written < chunk.len() as u64 {
                break;
            }
        }
        Ok(total)
    }

    /// Flush the handle the cursor writes through.
    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        self.require_open()?;
        self.handle
            .borrow_mut(py)
            .inner_mut()?
            .flush()
            .map_err(crate::holder::fs::storage_error)
    }

    /// Flush and close exactly once.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let mut close_failure = self.close_failure()?;
        if self.closed.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return close_failure.as_ref().map_or(Ok(()), |failure| {
                Err(crate::holder::fs::storage_error(failure.error()))
            });
        }
        let reader_close = match self.reader()?.take() {
            Some(mut reader) => reader.close(),
            None => Ok(()),
        };
        let flush = self.handle.borrow_mut(py).inner_mut()?.flush();
        let error = match (reader_close, flush) {
            (Err(error), _) | (_, Err(error)) => Some(error),
            (Ok(()), Ok(())) => None,
        };
        let Some(error) = error else {
            return Ok(());
        };
        let failure = crate::holder::fs::StickyFailure::new(error);
        let returned = crate::holder::fs::storage_error(failure.error());
        *close_failure = Some(failure);
        Err(returned)
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        slf.require_open()?;
        Ok(slf)
    }

    #[pyo3(signature = (exception_type = None, exception = None, traceback = None))]
    fn __exit__(
        &self,
        py: Python<'_>,
        exception_type: Option<&Bound<'_, PyAny>>,
        exception: Option<&Bound<'_, PyAny>>,
        traceback: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<bool> {
        let _ = (exception_type, exception, traceback);
        self.close(py)?;
        Ok(false)
    }
}

/// Python's lazy view of the core byte stream.
///
/// A Python object cannot hold a Rust borrow into another Python object, so it
/// retains the originating handle (or cursor) and asks the core stream for one
/// bounded chunk per `__next__`. No chunks are collected across the boundary.
enum PyByteSource {
    Position {
        handle: Py<PyIOBase>,
        position: u64,
    },
    Cursor(Py<PyIOCursor>),
    Reader {
        reader: Box<dyn yggdryl::holder::fs::RandomAccessReader>,
        cursor: Option<Py<PyIOCursor>>,
    },
    Empty,
}

#[pyclass(name = "ByteIterator", module = "yggdryl._native", unsendable)]
pub(crate) struct PyByteIterator {
    source: PyByteSource,
    batch_size: usize,
    done: bool,
}

#[pymethods]
impl PyByteIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyBytes>>> {
        if self.done {
            return Ok(None);
        }
        let next = match &mut self.source {
            PyByteSource::Position { handle, position } => {
                let handle = handle.borrow(py);
                let mut stream = handle
                    .inner()?
                    .pstream_bytes(*position, self.batch_size)
                    .map_err(value_error)?;
                let next = stream.next().transpose();
                drop(stream);
                if let Ok(Some(bytes)) = &next {
                    *position = position
                        .checked_add(bytes.len() as u64)
                        .ok_or_else(|| value_error("byte stream position exceeds u64::MAX"))?;
                }
                next
            }
            PyByteSource::Cursor(cursor) => {
                let cursor = cursor.borrow(py);
                match cursor.read_buffer(py, self.batch_size) {
                    Ok(bytes) if bytes.is_empty() => {
                        self.done = true;
                        return Ok(None);
                    }
                    Ok(bytes) => return Ok(Some(PyBytes::new(py, &bytes))),
                    Err(error) => {
                        self.done = true;
                        return Err(error);
                    }
                }
            }
            PyByteSource::Reader { reader, cursor } => {
                let mut bytes = vec![0_u8; self.batch_size];
                match reader.read(&mut bytes) {
                    Ok(0) => {
                        self.done = true;
                        reader.close().map_err(crate::holder::fs::storage_error)?;
                        return Ok(None);
                    }
                    Ok(count) => {
                        bytes.truncate(count);
                        if let Some(cursor) = cursor {
                            let cursor = cursor.borrow(py);
                            cursor.store(cursor.load().saturating_add(count as u64));
                        }
                        Ok(Some(bytes))
                    }
                    Err(error) => {
                        self.done = true;
                        let _ = reader.close();
                        Err(error)
                    }
                }
            }
            PyByteSource::Empty => {
                self.done = true;
                return Ok(None);
            }
        };
        let next = match next {
            Ok(next) => next,
            Err(error) => {
                self.done = true;
                return Err(crate::holder::fs::storage_error(error));
            }
        };
        if let Some(bytes) = next {
            Ok(Some(PyBytes::new(py, &bytes)))
        } else {
            self.done = true;
            Ok(None)
        }
    }
}
