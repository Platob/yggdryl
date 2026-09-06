//! The storage role a handle turned out to be, one class per core holder.
//!
//! Every class here is an `IOBase` subclass that adds no state of its own: the
//! core holder still does the work, and the class is how Python says which
//! implementation is doing it. `type(handle)` therefore answers the question a
//! caller actually has - is this mapped local storage, an in-memory buffer, or
//! a location on a foreign filesystem - without a predicate for each one.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyType;

use yggdryl::holder::Holder;
use yggdryl::holder::buffered::Buffered;
use yggdryl::holder::s3::S3Options;

use crate::iobase::PyIOBase;
use crate::value_error;

/// Declare one storage role that adds nothing but its name.
macro_rules! role {
    ($ident:ident, $name:literal, $doc:expr) => {
        #[doc = $doc]
        #[pyclass(name = $name, module = "yggdryl._native", extends = PyIOBase, skip_from_py_object)]
        pub(crate) struct $ident;
    };
}

role!(
    PyBuffer,
    "Buffer",
    "In-memory bytes: the handle `IOBase.from_bytes` answers with, and what a \
     nameless stream is captured into."
);
role!(
    PyFile,
    "File",
    "One memory-mapped local file. The mapping is created by the first \
     operation that needs it, not by naming the location."
);
role!(
    PyFolder,
    "Folder",
    "One local directory, listed and walked without being opened."
);
role!(
    PyPath,
    "Path",
    "One local location that resolves to `File` or `Folder` when an operation \
     needs to know which it is - the role a name that says nothing takes."
);
role!(
    PyFsFile,
    "FsFile",
    "One file on a foreign filesystem, read and written through its streams."
);
role!(
    PyFsFolder,
    "FsFolder",
    "One directory on a foreign filesystem."
);
role!(
    PyFsPath,
    "FsPath",
    "One location on a foreign filesystem that resolves when an operation \
     needs to know what is there."
);
role!(
    PyS3File,
    "S3File",
    "One Amazon S3 object, read by range and written whole. A ranged read \
     transfers the range rather than the object, and learns the object's \
     length from the answer."
);
role!(
    PyS3Folder,
    "S3Folder",
    "One S3 key prefix, or a whole bucket. A prefix is not stored: it exists \
     exactly while a key starts with it, so creating and deleting one cost \
     nothing and listing is the only question the store answers."
);
role!(
    PyS3Path,
    "S3Path",
    "One S3 location that resolves to `S3File` or `S3Folder` when an \
     operation needs to know which it is - one listing of a single key, or \
     none at all when a trailing slash already said it is a container."
);
role!(
    PyBuffered,
    "Buffered",
    "Any handle read through the core's bounded page cache. A cache stays the \
     outermost wrapper, so it never sits under a coding or a record encoding."
);

/// Build one located local role from whatever names the location.
fn local_holder(
    location: &Bound<'_, PyAny>,
    build: impl FnOnce(std::path::PathBuf) -> yggdryl::Result<Holder>,
) -> PyResult<PyClassInitializer<PyIOBase>> {
    let url = crate::uri::core_url_from_value(location)?;
    let path = url.into_path().map_err(value_error)?;
    Ok(PyClassInitializer::from(PyIOBase::from_core(
        build(path).map_err(value_error)?,
    )))
}

/// Bind one location on a `pyarrow.fs.FileSystem`, as the roles take it.
fn bound_location(
    filesystem: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    uri: Option<&Bound<'_, PyAny>>,
) -> PyResult<yggdryl::holder::fs::BoundLocation> {
    if !crate::holder::fs::is_arrow_filesystem(filesystem)? {
        return Err(PyValueError::new_err(format!(
            "expected a pyarrow.fs.FileSystem, got {}",
            filesystem.get_type().name()?,
        )));
    }
    let path = crate::uri::path_string_from_value(path)?;
    let uri = uri.map(crate::uri::path_string_from_value).transpose()?;
    let backend: std::sync::Arc<dyn yggdryl::holder::fs::FileSystem> =
        std::sync::Arc::new(crate::holder::fs::PyFileSystem::new(filesystem)?);
    yggdryl::holder::fs::BoundLocation::new(backend, path, uri)
        .map_err(crate::holder::fs::storage_error)
}

/// Build one foreign-filesystem role from a bound location.
fn fs_holder(
    filesystem: &Bound<'_, PyAny>,
    path: &Bound<'_, PyAny>,
    uri: Option<&Bound<'_, PyAny>>,
    build: impl FnOnce(yggdryl::holder::fs::BoundLocation) -> Holder,
) -> PyResult<PyClassInitializer<PyIOBase>> {
    Ok(PyClassInitializer::from(PyIOBase::from_core(build(
        bound_location(filesystem, path, uri)?,
    ))))
}

#[pymethods]
impl PyPath {
    /// Describe a local location without deciding what it is.
    ///
    /// `IOBase(location)` answers with the coding and record implementation
    /// the name declares; this is the byte handle underneath that, for a
    /// caller who wants the stored bytes rather than the value they encode.
    #[new]
    fn new(location: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        Ok(local_holder(location, Holder::local)?.add_subclass(Self))
    }
}

#[pymethods]
impl PyFile {
    /// Describe a local file, whether or not it exists yet.
    #[new]
    fn new(location: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        Ok(local_holder(location, Holder::file)?.add_subclass(Self))
    }
}

#[pymethods]
impl PyFolder {
    /// Describe a local directory, whether or not it exists yet.
    ///
    /// Naming a location as a container is what tells a handle to resolve
    /// children rather than bytes; nothing is created until a write asks.
    #[new]
    fn new(location: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        Ok(local_holder(location, Holder::folder)?.add_subclass(Self))
    }

    /// The platform temporary directory, created by nothing.
    #[classmethod]
    fn temporary(_cls: &Bound<'_, PyType>, py: Python<'_>) -> PyResult<Py<Self>> {
        Self::root(py, yggdryl::holder::local::Folder::temporary())
    }

    /// The user's home directory, from `HOME` then `USERPROFILE`.
    #[classmethod]
    fn home(_cls: &Bound<'_, PyType>, py: Python<'_>) -> PyResult<Py<Self>> {
        Self::root(py, yggdryl::holder::local::Folder::home())
    }

    /// The user's configuration directory: `home` joined with `.config`.
    #[classmethod]
    fn config(_cls: &Bound<'_, PyType>, py: Python<'_>) -> PyResult<Py<Self>> {
        Self::root(py, yggdryl::holder::local::Folder::config())
    }
}

impl PyFolder {
    /// Answer one well-known root as this class.
    fn root(
        py: Python<'_>,
        folder: yggdryl::Result<yggdryl::holder::local::Folder>,
    ) -> PyResult<Py<Self>> {
        let holder = Holder::Folder(folder.map_err(value_error)?);
        Py::new(
            py,
            PyClassInitializer::from(PyIOBase::from_core(holder)).add_subclass(Self),
        )
    }
}

#[pymethods]
impl PyFsPath {
    /// Describe a location on a foreign filesystem without deciding what it is.
    ///
    /// `IOBase.from_fs` answers with the coding and record implementation the
    /// name declares; this is the byte handle underneath that, and the way to
    /// address the stored bytes of a coded name on a bucket.
    #[new]
    #[pyo3(signature = (filesystem, path, *, uri = None))]
    fn new(
        filesystem: &Bound<'_, PyAny>,
        path: &Bound<'_, PyAny>,
        uri: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(fs_holder(filesystem, path, uri, yggdryl::holder::fs::located)?.add_subclass(Self))
    }
}

#[pymethods]
impl PyFsFile {
    /// Describe a file on a foreign filesystem, whether or not it exists yet.
    #[new]
    #[pyo3(signature = (filesystem, path, *, uri = None))]
    fn new(
        filesystem: &Bound<'_, PyAny>,
        path: &Bound<'_, PyAny>,
        uri: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let build = |bound| Holder::FsFile(yggdryl::holder::fs::File::new(bound));
        Ok(fs_holder(filesystem, path, uri, build)?.add_subclass(Self))
    }
}

#[pymethods]
impl PyFsFolder {
    /// Describe a directory on a foreign filesystem, creating nothing.
    #[new]
    #[pyo3(signature = (filesystem, path, *, uri = None))]
    fn new(
        filesystem: &Bound<'_, PyAny>,
        path: &Bound<'_, PyAny>,
        uri: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let build = |bound| Holder::FsFolder(yggdryl::holder::fs::Folder::new(bound));
        Ok(fs_holder(filesystem, path, uri, build)?.add_subclass(Self))
    }
}

/// Build one S3 role from a bucket and a raw key, or from a location.
///
/// A caller who has a location passes one string; a caller who has the name a
/// store uses passes the bucket and the key, and encoding belongs here rather
/// than to them - `a b/c.txt` is an ordinary key and not a URL.
fn s3_holder(
    location: &Bound<'_, PyAny>,
    key: Option<&Bound<'_, PyAny>>,
    options: Option<&Bound<'_, pyo3::types::PyDict>>,
    from_url: impl FnOnce(&str, S3Options) -> yggdryl::Result<Holder>,
    from_key: impl FnOnce(&str, &str, S3Options) -> yggdryl::Result<Holder>,
) -> PyResult<PyClassInitializer<PyIOBase>> {
    let first = crate::uri::path_string_from_value(location)?;
    let options = s3_options(options)?;
    let holder = match key {
        Some(key) => from_key(&first, &crate::uri::path_string_from_value(key)?, options),
        None => from_url(&first, options),
    };
    // Construction touches no store, so every failure here is about the name
    // the caller gave rather than about the store: a `ValueError`, as it is
    // for a local location that will not form a URL.
    Ok(PyClassInitializer::from(PyIOBase::from_core(
        holder.map_err(value_error)?,
    )))
}

/// Read an options mapping in whichever vocabulary it is written in.
///
/// `PyIceberg`'s `s3.*` property names, `PyArrow`'s `S3FileSystem` arguments,
/// and the AWS environment's names are all read; anything else is ignored, so
/// a catalog's properties can be handed over whole. Values are taken as their
/// text, so `True` and `30` are as good as `"true"` and `"30"`.
fn s3_options(options: Option<&Bound<'_, pyo3::types::PyDict>>) -> PyResult<S3Options> {
    let Some(options) = options else {
        return Ok(S3Options::default());
    };
    let mut properties: Vec<(String, String)> = Vec::with_capacity(options.len());
    for (name, value) in options {
        if value.is_none() {
            continue;
        }
        properties.push((name.str()?.extract()?, value.str()?.extract()?));
    }
    // Nothing is contacted, so a value that will not parse is an argument
    // error rather than a store's refusal.
    S3Options::from_properties(properties).map_err(value_error)
}

#[pymethods]
impl PyS3Path {
    /// Describe an S3 location without deciding what it is.
    ///
    /// `S3Path("s3://trades/lake/part.parquet")` names a location;
    /// `S3Path("trades", "lake/a b/part.parquet")` names a bucket and the raw
    /// key a store uses. Neither contacts the store.
    ///
    /// `options` is a mapping of endpoint, credentials, encryption, and the
    /// rest, in `PyIceberg`'s names, `PyArrow`'s, or the AWS environment's.
    #[new]
    #[pyo3(signature = (location, key = None, *, options = None))]
    fn new(
        location: &Bound<'_, PyAny>,
        key: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, pyo3::types::PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(s3_holder(
            location,
            key,
            options,
            yggdryl::holder::s3::located_with,
            |bucket, key, options| {
                yggdryl::holder::s3::path_at_with(bucket, key, options).map(Holder::S3Path)
            },
        )?
        .add_subclass(Self))
    }
}

#[pymethods]
impl PyS3File {
    /// Describe an S3 object, whether or not it exists yet.
    ///
    /// `options` is read as it is by [`S3Path`](PyS3Path).
    #[new]
    #[pyo3(signature = (location, key = None, *, options = None))]
    fn new(
        location: &Bound<'_, PyAny>,
        key: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, pyo3::types::PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(s3_holder(
            location,
            key,
            options,
            |url, options| yggdryl::holder::s3::file_with(url, options).map(Holder::S3File),
            |bucket, key, options| {
                yggdryl::holder::s3::file_at_with(bucket, key, options).map(Holder::S3File)
            },
        )?
        .add_subclass(Self))
    }
}

#[pymethods]
impl PyS3Folder {
    /// Describe an S3 prefix or bucket, creating nothing.
    ///
    /// `options` is read as it is by [`S3Path`](PyS3Path).
    #[new]
    #[pyo3(signature = (location, key = None, *, options = None))]
    fn new(
        location: &Bound<'_, PyAny>,
        key: Option<&Bound<'_, PyAny>>,
        options: Option<&Bound<'_, pyo3::types::PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        Ok(s3_holder(
            location,
            key,
            options,
            |url, options| yggdryl::holder::s3::folder_with(url, options).map(Holder::S3Folder),
            |bucket, key, options| {
                yggdryl::holder::s3::folder_at_with(bucket, key, options).map(Holder::S3Folder)
            },
        )?
        .add_subclass(Self))
    }
}

/// Borrow the page cache behind a `Buffered` handle.
fn cache<'borrow>(slf: &'borrow PyRef<'_, PyBuffered>) -> PyResult<&'borrow Buffered<Holder>> {
    match slf.as_super().inner()? {
        Holder::Buffered(buffered) => Ok(buffered),
        _ => Err(PyValueError::new_err(
            "this handle no longer holds a page cache",
        )),
    }
}

#[pymethods]
impl PyBuffered {
    /// The number of bytes the cache currently holds.
    #[getter]
    fn cached_bytes(slf: &Bound<'_, Self>) -> PyResult<u64> {
        Ok(cache(&slf.borrow())?.cached_bytes())
    }

    /// The number of pages the cache currently holds.
    #[getter]
    fn cached_pages(slf: &Bound<'_, Self>) -> PyResult<usize> {
        Ok(cache(&slf.borrow())?.cached_pages())
    }

    /// Return whether the page at `index` is resident.
    ///
    /// Pages are indexed by `offset // page_size`, so this answers what a
    /// benchmark or a diagnostic asks: did that read come from the cache.
    fn has_cached_page(slf: &Bound<'_, Self>, index: u64) -> PyResult<bool> {
        Ok(cache(&slf.borrow())?.has_cached_page(index))
    }

    /// The handle underneath the cache, as its own role.
    ///
    /// This handle is spent: a cache owns the handle it caches, so descending
    /// moves the value to the one this answers with.
    fn into_handle(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::iobase::unwrapped(py, slf.as_super())
    }

    /// Drop every cached page, keeping the cache and its options.
    fn clear_cache(mut slf: PyRefMut<'_, Self>) -> PyResult<()> {
        match slf.as_super().inner_mut()? {
            Holder::Buffered(buffered) => {
                buffered.clear_cache();
                Ok(())
            }
            _ => Err(PyValueError::new_err(
                "this handle no longer holds a page cache",
            )),
        }
    }
}

/// Register every storage role.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyBuffer>()?;
    module.add_class::<PyFile>()?;
    module.add_class::<PyFolder>()?;
    module.add_class::<PyPath>()?;
    module.add_class::<PyFsFile>()?;
    module.add_class::<PyFsFolder>()?;
    module.add_class::<PyFsPath>()?;
    module.add_class::<PyS3File>()?;
    module.add_class::<PyS3Folder>()?;
    module.add_class::<PyS3Path>()?;
    module.add_class::<PyBuffered>()?;
    Ok(())
}
