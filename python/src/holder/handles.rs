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
    module.add_class::<PyBuffered>()?;
    Ok(())
}
