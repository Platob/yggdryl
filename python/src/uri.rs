//! Native Python views of Yggdryl URI, URL, and URN values.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyString, PyTuple};
use yggdryl::{Uri as CoreUri, Url as CoreUrl, Urn as CoreUrn};

use crate::enums::{
    PyMediaType, PyMimeType, core_media_type_from_value, core_mime_type_from_value,
    strings_from_iterable,
};
use crate::{compare, normalize_index, value_error};

/// Adds an exact size hint to a cheaply cloned core iterator without collecting it.
struct ExactIterator<I> {
    inner: I,
    remaining: usize,
}

impl<I> ExactIterator<I>
where
    I: Iterator + Clone,
{
    fn new(inner: I) -> Self {
        let remaining = inner.clone().count();
        Self { inner, remaining }
    }
}

impl<I> Iterator for ExactIterator<I>
where
    I: Iterator,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.inner.next();
        if value.is_some() {
            self.remaining = self.remaining.saturating_sub(1);
        } else {
            self.remaining = 0;
        }
        value
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<I> ExactSizeIterator for ExactIterator<I> where I: Iterator {}

fn core_uri_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreUri> {
    if let Ok(value) = value.extract::<PyRef<'_, PyUri>>() {
        return Ok(value.inner.clone());
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUrl>>() {
        return Ok(value.inner.clone().into_uri());
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUrn>>() {
        return Ok(value.inner.clone().into_uri());
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreUri::from_str(value).map_err(value_error);
    }
    if value.hasattr("__fspath__")? {
        return CoreUri::from_path(path_string_from_value(value)?).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a yggdryl.Uri, yggdryl.Url, yggdryl.Urn, or URI string",
    ))
}

pub(crate) fn core_url_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreUrl> {
    if let Ok(value) = value.extract::<PyRef<'_, PyUrl>>() {
        return Ok(value.inner.clone());
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUri>>() {
        return CoreUrl::from_uri(value.inner.clone()).map_err(value_error);
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUrn>>() {
        return CoreUrl::from_uri(value.inner.clone().into_uri()).map_err(value_error);
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreUrl::from_str(value).map_err(value_error);
    }
    if value.hasattr("__fspath__")? {
        return CoreUrl::from_path(path_string_from_value(value)?).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a yggdryl.Url, yggdryl.Uri, yggdryl.Urn, URL string, or path-like value",
    ))
}

fn core_urn_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreUrn> {
    if let Ok(value) = value.extract::<PyRef<'_, PyUrn>>() {
        return Ok(value.inner.clone());
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreUrn::from_str(value).map_err(value_error);
    }
    if let Ok(value) = core_uri_from_value(value) {
        return CoreUrn::from_uri(value).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a yggdryl.Urn, yggdryl.Uri, yggdryl.Url, or URN string",
    ))
}

pub(crate) fn path_string_from_value(value: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(value) = value.extract::<String>() {
        return Ok(value);
    }
    if value.hasattr("__fspath__")? {
        return value
            .call_method0("__fspath__")?
            .extract()
            .map_err(|_| PyTypeError::new_err("path-like __fspath__ must return str, not bytes"));
    }
    Err(PyTypeError::new_err("expected str or os.PathLike[str]"))
}

fn path_string_from_core(value: std::path::PathBuf) -> PyResult<String> {
    value.into_os_string().into_string().map_err(|_| {
        PyValueError::new_err("file URI path cannot be represented as a Python string")
    })
}

/// A normalized URI with sequence access to its path segments.
///
/// The Python view stays mutable until it is first hashed. Hashing locks that
/// one wrapper so its canonical value remains stable as a mapping key.
#[pyclass(name = "Uri", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyUri {
    pub(crate) inner: CoreUri,
    hash_locked: bool,
}

impl PyUri {
    fn from_core(inner: CoreUri) -> Self {
        Self {
            inner,
            hash_locked: false,
        }
    }

    fn require_mutable(&self) -> PyResult<()> {
        if self.hash_locked {
            Err(PyTypeError::new_err(
                "a hashed Uri is frozen; copy it before mutation",
            ))
        } else {
            Ok(())
        }
    }
}

#[pymethods]
impl PyUri {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_uri_from_value(value).map(Self::from_core)
    }

    #[staticmethod]
    fn from_value(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::new(value)
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreUri::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_path(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let value = path_string_from_value(value)?;
        CoreUri::from_path(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreUri::from_json(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_url(&self) -> PyResult<PyUrl> {
        self.inner
            .clone()
            .into_url()
            .map(PyUrl::from_core)
            .map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_urn(&self) -> PyResult<PyUrn> {
        self.inner
            .clone()
            .into_urn()
            .map(PyUrn::from_core)
            .map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_path(&self) -> PyResult<String> {
        self.inner
            .clone()
            .into_path()
            .map_err(value_error)
            .and_then(path_string_from_core)
    }

    fn __fspath__(&self) -> PyResult<String> {
        self.into_path()
    }

    #[getter]
    fn scheme(&self) -> &str {
        self.inner.scheme().as_str()
    }

    #[getter]
    fn authority(&self) -> &str {
        self.inner.authority().as_str()
    }

    #[getter]
    fn user(&self) -> Option<&str> {
        self.inner.user()
    }

    #[getter]
    fn password(&self) -> Option<&str> {
        self.inner.password()
    }

    #[getter]
    fn hostname(&self) -> Option<&str> {
        self.inner.hostname()
    }

    #[getter]
    fn bucket(&self) -> Option<&str> {
        self.inner.bucket()
    }

    #[getter]
    fn region(&self) -> Option<&str> {
        self.inner.region()
    }

    #[getter]
    fn path(&self) -> &str {
        self.inner.path().as_str()
    }

    #[getter]
    fn query(&self) -> Option<&str> {
        self.inner.query()
    }

    #[getter]
    fn fragment(&self) -> Option<&str> {
        self.inner.fragment()
    }

    #[getter]
    fn path_segments<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, ExactIterator::new(self.inner.path_segments()))
    }

    #[getter]
    fn file_name(&self) -> Option<&str> {
        self.inner.file_name()
    }

    #[getter]
    fn stem(&self) -> Option<&str> {
        self.inner.stem()
    }

    #[getter]
    fn extension(&self) -> Option<&str> {
        self.inner.extension()
    }

    #[getter]
    fn extensions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, ExactIterator::new(self.inner.extensions()))
    }

    fn set_file_name(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_file_name(value).map_err(value_error)
    }

    fn set_stem(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_stem(value).map_err(value_error)
    }

    fn set_extension(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_extension(value).map_err(value_error)
    }

    fn set_extensions(&mut self, values: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_extensions(strings_from_iterable(values, "extensions")?)
            .map_err(value_error)
    }

    fn remove_extension(&mut self) -> PyResult<bool> {
        self.require_mutable()?;
        Ok(self.inner.remove_extension())
    }

    fn clear_extensions(&mut self) -> PyResult<bool> {
        self.require_mutable()?;
        Ok(self.inner.clear_extensions())
    }

    #[getter]
    fn mime_type(&self) -> PyMimeType {
        PyMimeType::from_core(self.inner.mime_type())
    }

    #[getter]
    fn media_type(&self) -> PyMediaType {
        PyMediaType::from_core(self.inner.media_type())
    }

    fn set_mime_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_mime_type(core_mime_type_from_value(value)?)
            .map_err(value_error)
    }

    fn set_media_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_media_type(core_media_type_from_value(value)?)
            .map_err(value_error)
    }

    /// Return this URI with path components joined by the core path resolver.
    ///
    /// Scheme, authority, query, and fragment are preserved. Relative values
    /// extend the path with `.` and `..` resolved; an absolute value replaces
    /// the path. The source is never mutated, including after it is hash-locked.
    #[pyo3(signature = (*others))]
    fn joinpath(&self, others: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut joined = self.inner.clone();
        for other in others {
            joined = joined
                .joinpath(&path_string_from_value(&other)?)
                .map_err(value_error)?;
        }
        Ok(Self::from_core(joined))
    }

    /// `uri / "child"`, using the same core join as `joinpath`.
    fn __truediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.inner
            .joinpath(&path_string_from_value(other)?)
            .map(Self::from_core)
            .map_err(value_error)
    }

    fn __len__(&self) -> usize {
        self.inner.path().segment_len()
    }

    fn __iter__(&self) -> PyUriPathIterator {
        let inner = self.inner.clone();
        PyUriPathIterator {
            remaining: inner.path().segment_len(),
            inner,
            cursor: 0,
        }
    }

    fn __getitem__(&self, index: isize) -> PyResult<&str> {
        let normalized = if index >= 0 {
            usize::try_from(index).ok()
        } else {
            normalize_index(index, self.inner.path().segment_len())
        };
        normalized
            .and_then(|index| self.inner.path().get_segment(index))
            .ok_or_else(|| PyIndexError::new_err(index))
    }

    fn __contains__(&self, segment: &Bound<'_, PyAny>) -> bool {
        segment
            .extract::<&str>()
            .is_ok_and(|segment| self.inner.path().contains_segment(segment))
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Uri.from_str({:?})", self.inner.to_string())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&mut self) -> isize {
        self.hash_locked = true;
        crate::python_hash(self.inner.stable_hash())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let callable = py.get_type::<Self>().getattr("from_str")?.unbind();
        Ok((callable, (self.inner.to_string(),)))
    }

    fn __copy__(&self) -> Self {
        Self::from_core(self.inner.clone())
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        Self::from_core(self.inner.clone())
    }
}

/// A URL view validated and normalized by the core URI model.
///
/// The Python view stays mutable until it is first hashed. Hashing locks that
/// one wrapper so its canonical value remains stable as a mapping key.
#[pyclass(name = "Url", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyUrl {
    pub(crate) inner: CoreUrl,
    hash_locked: bool,
}

impl PyUrl {
    pub(crate) fn from_core(inner: CoreUrl) -> Self {
        Self {
            inner,
            hash_locked: false,
        }
    }

    fn require_mutable(&self) -> PyResult<()> {
        if self.hash_locked {
            Err(PyTypeError::new_err(
                "a hashed Url is frozen; copy it before mutation",
            ))
        } else {
            Ok(())
        }
    }
}

#[pymethods]
impl PyUrl {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_url_from_value(value).map(Self::from_core)
    }

    #[staticmethod]
    fn from_value(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::new(value)
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreUrl::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_path(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        let value = path_string_from_value(value)?;
        CoreUrl::from_path(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_uri(value: PyRef<'_, PyUri>) -> PyResult<Self> {
        let inner = value.inner.clone();
        drop(value);
        CoreUrl::from_uri(inner)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreUrl::from_json(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_uri(&self) -> PyUri {
        PyUri::from_core(self.inner.clone().into_uri())
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_path(&self) -> PyResult<String> {
        self.inner
            .clone()
            .into_path()
            .map_err(value_error)
            .and_then(path_string_from_core)
    }

    fn __fspath__(&self) -> PyResult<String> {
        self.into_path()
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    #[getter]
    fn scheme(&self) -> &str {
        self.inner.scheme().as_str()
    }

    #[getter]
    fn authority(&self) -> &str {
        self.inner.authority().as_str()
    }

    #[getter]
    fn user(&self) -> Option<&str> {
        self.inner.user()
    }

    #[getter]
    fn password(&self) -> Option<&str> {
        self.inner.password()
    }

    #[getter]
    fn hostname(&self) -> Option<&str> {
        self.inner.hostname()
    }

    #[getter]
    fn bucket(&self) -> Option<&str> {
        self.inner.bucket()
    }

    #[getter]
    fn region(&self) -> Option<&str> {
        self.inner.region()
    }

    #[getter]
    fn path(&self) -> &str {
        self.inner.path().as_str()
    }

    #[getter]
    fn query(&self) -> Option<&str> {
        self.inner.query()
    }

    #[getter]
    fn fragment(&self) -> Option<&str> {
        self.inner.fragment()
    }

    #[getter]
    fn path_segments<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, ExactIterator::new(self.inner.path_segments()))
    }

    #[getter]
    fn file_name(&self) -> Option<&str> {
        self.inner.file_name()
    }

    #[getter]
    fn extension(&self) -> Option<&str> {
        self.inner.extension()
    }

    #[getter]
    fn extensions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, ExactIterator::new(self.inner.extensions()))
    }

    #[getter]
    fn stem(&self) -> Option<&str> {
        self.inner.stem()
    }

    fn set_file_name(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_file_name(value).map_err(value_error)
    }

    fn set_stem(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_stem(value).map_err(value_error)
    }

    fn set_extension(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_extension(value).map_err(value_error)
    }

    fn set_extensions(&mut self, values: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_extensions(strings_from_iterable(values, "extensions")?)
            .map_err(value_error)
    }

    fn remove_extension(&mut self) -> PyResult<bool> {
        self.require_mutable()?;
        Ok(self.inner.remove_extension())
    }

    fn clear_extensions(&mut self) -> PyResult<bool> {
        self.require_mutable()?;
        Ok(self.inner.clear_extensions())
    }

    #[getter]
    fn mime_type(&self) -> PyMimeType {
        PyMimeType::from_core(self.inner.mime_type())
    }

    #[getter]
    fn media_type(&self) -> PyMediaType {
        PyMediaType::from_core(self.inner.media_type())
    }

    fn set_mime_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_mime_type(core_mime_type_from_value(value)?)
            .map_err(value_error)
    }

    fn set_media_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_media_type(core_media_type_from_value(value)?)
            .map_err(value_error)
    }

    // ---------------------------------------------------------------------
    // `pathlib.Path` compatibility.
    //
    // A URL is a path with a scheme, so it answers the same questions a
    // `Path` does under the same names. Code written against `pathlib` runs
    // against a location in any backend, and every answer comes from the core
    // implementation rather than from a second one written in Python.
    // ---------------------------------------------------------------------

    /// The final path component, as `pathlib.PurePath.name`.
    #[getter]
    fn name(&self) -> &str {
        self.inner.file_name().unwrap_or_default()
    }

    /// The final extension with its leading dot, as `PurePath.suffix`.
    #[getter]
    fn suffix(&self) -> String {
        self.inner
            .extension()
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default()
    }

    /// Every extension with leading dots, as `PurePath.suffixes`.
    #[getter]
    fn suffixes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let suffixes: Vec<String> = self
            .inner
            .extensions()
            .map(|extension| format!(".{extension}"))
            .collect();
        PyTuple::new(py, suffixes)
    }

    /// The path components, as `PurePath.parts`.
    #[getter]
    fn parts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        self.path_segments(py)
    }

    /// The containing location, as `PurePath.parent`.
    ///
    /// A location at the root is its own parent, which is what `pathlib` does.
    #[getter]
    fn parent(&self) -> Self {
        Self::from_core(self.inner.parent().unwrap_or_else(|| self.inner.clone()))
    }

    /// Every containing location, closest first, as `PurePath.parents`.
    #[getter]
    fn parents<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let parents: Vec<Self> = self.inner.parents().map(Self::from_core).collect();
        PyTuple::new(py, parents)
    }

    /// Join path components onto this location, as `PurePath.joinpath`.
    #[pyo3(signature = (*others))]
    fn joinpath(&self, others: &Bound<'_, PyTuple>) -> PyResult<Self> {
        let mut joined = self.inner.clone();
        for other in others {
            joined = joined
                .joinpath(&path_string_from_value(&other)?)
                .map_err(value_error)?;
        }
        Ok(Self::from_core(joined))
    }

    /// `url / "child"`, as `PurePath.__truediv__`.
    fn __truediv__(&self, other: &Bound<'_, PyAny>) -> PyResult<Self> {
        self.inner
            .joinpath(&path_string_from_value(other)?)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// This location with a different final component, as `with_name`.
    fn with_name(&self, value: &str) -> PyResult<Self> {
        let mut renamed = self.inner.clone();
        renamed.set_file_name(value).map_err(value_error)?;
        Ok(Self::from_core(renamed))
    }

    /// This location with a different stem, as `with_stem`.
    fn with_stem(&self, value: &str) -> PyResult<Self> {
        let mut renamed = self.inner.clone();
        renamed.set_stem(value).map_err(value_error)?;
        Ok(Self::from_core(renamed))
    }

    /// This location with a different final extension, as `with_suffix`.
    ///
    /// The leading dot is optional, and an empty suffix removes the extension.
    fn with_suffix(&self, value: &str) -> PyResult<Self> {
        let mut renamed = self.inner.clone();
        let suffix = value.strip_prefix('.').unwrap_or(value);
        if suffix.is_empty() {
            renamed.remove_extension();
        } else {
            renamed.set_extension(suffix).map_err(value_error)?;
        }
        Ok(Self::from_core(renamed))
    }

    /// A URL path is always absolute, as `PurePath.is_absolute`.
    #[allow(clippy::unused_self)]
    fn is_absolute(&self) -> bool {
        true
    }

    /// The path in POSIX form, as `PurePath.as_posix`.
    fn as_posix(&self) -> &str {
        self.inner.path().as_str()
    }

    /// The whole location as text, as `PurePath.as_uri`.
    fn as_uri(&self) -> String {
        self.inner.to_string()
    }

    /// Return whether this location matches `pattern`, as `PurePath.match`.
    ///
    /// A pattern with no separator matches the name at any depth; one with a
    /// separator is anchored at the path root.
    #[pyo3(name = "match")]
    fn matches(&self, pattern: &str) -> bool {
        self.inner.matches_glob(pattern)
    }

    /// Return whether the whole path matches, as `PurePath.full_match`.
    fn full_match(&self, pattern: &str) -> bool {
        self.inner.matches_glob(pattern)
    }

    /// Return whether this location is a glob pattern rather than one name.
    fn is_glob(&self) -> bool {
        self.inner.is_glob()
    }

    /// Return this location relative to `other`, as `PurePath.relative_to`.
    ///
    /// Raises `ValueError` when this location is not below `other`, which is
    /// what `pathlib` does.
    fn relative_to(&self, other: &Bound<'_, PyAny>) -> PyResult<String> {
        let root = core_url_from_value(other)?;
        self.inner
            .segments_under(&root)
            .map(|segments| segments.join("/"))
            .ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "{} is not in the subpath of {root}",
                    self.inner
                ))
            })
    }

    /// Return whether this location is below `other`, as `is_relative_to`.
    fn is_relative_to(&self, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self
            .inner
            .segments_under(&core_url_from_value(other)?)
            .is_some())
    }

    /// Return whether something exists here now, as `Path.exists`.
    fn exists(&self) -> bool {
        self.inner.exists()
    }

    /// Return whether this location is a directory, as `Path.is_dir`.
    fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Return whether this location is a regular file, as `Path.is_file`.
    fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Return whether the name begins with a dot, so a listing may skip it.
    fn is_private(&self) -> bool {
        self.inner.is_private()
    }

    /// The Hive partition pairs this location's path spells out.
    #[getter]
    fn partitions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.inner.hive_partitions())
    }

    /// Return the value of one Hive partition column, when the path has it.
    fn partition(&self, column: &str) -> Option<String> {
        self.inner.hive_partition(column)
    }

    fn __len__(&self) -> usize {
        self.inner.path().segment_len()
    }

    fn __iter__(&self) -> PyUriPathIterator {
        let inner = self.inner.clone().into_uri();
        PyUriPathIterator {
            remaining: inner.path().segment_len(),
            inner,
            cursor: 0,
        }
    }

    fn __getitem__(&self, index: isize) -> PyResult<&str> {
        let normalized = if index >= 0 {
            usize::try_from(index).ok()
        } else {
            normalize_index(index, self.inner.path().segment_len())
        };
        normalized
            .and_then(|index| self.inner.path().get_segment(index))
            .ok_or_else(|| PyIndexError::new_err(index))
    }

    fn __contains__(&self, segment: &Bound<'_, PyAny>) -> bool {
        segment
            .extract::<&str>()
            .is_ok_and(|segment| self.inner.path().contains_segment(segment))
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Url.from_str({:?})", self.inner.to_string())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&mut self) -> isize {
        self.hash_locked = true;
        crate::python_hash(self.inner.stable_hash())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let callable = py.get_type::<Self>().getattr("from_str")?.unbind();
        Ok((callable, (self.inner.to_string(),)))
    }

    fn __copy__(&self) -> Self {
        Self::from_core(self.inner.clone())
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        Self::from_core(self.inner.clone())
    }
}

/// A URN view with namespace-specific accessors.
///
/// The Python view stays mutable until it is first hashed. Hashing locks that
/// one wrapper so its canonical value remains stable as a mapping key.
#[pyclass(name = "Urn", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyUrn {
    pub(crate) inner: CoreUrn,
    hash_locked: bool,
}

impl PyUrn {
    fn from_core(inner: CoreUrn) -> Self {
        Self {
            inner,
            hash_locked: false,
        }
    }

    fn require_mutable(&self) -> PyResult<()> {
        if self.hash_locked {
            Err(PyTypeError::new_err(
                "a hashed Urn is frozen; copy it before mutation",
            ))
        } else {
            Ok(())
        }
    }
}

#[pymethods]
impl PyUrn {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        core_urn_from_value(value).map(Self::from_core)
    }

    #[staticmethod]
    fn from_value(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        Self::new(value)
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        CoreUrn::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_uri(value: PyRef<'_, PyUri>) -> PyResult<Self> {
        let inner = value.inner.clone();
        drop(value);
        CoreUrn::from_uri(inner)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreUrn::from_json(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_uri(&self) -> PyUri {
        PyUri::from_core(self.inner.clone().into_uri())
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    #[getter]
    fn scheme(&self) -> &str {
        self.inner.scheme().as_str()
    }

    #[getter]
    fn authority(&self) -> &str {
        self.inner.authority().as_str()
    }

    #[getter]
    fn path(&self) -> &str {
        self.inner.path().as_str()
    }

    #[getter]
    fn query(&self) -> Option<&str> {
        self.inner.query()
    }

    #[getter]
    fn fragment(&self) -> Option<&str> {
        self.inner.fragment()
    }

    #[getter]
    fn namespace(&self) -> &str {
        self.inner.namespace()
    }

    #[getter]
    fn namespace_specific(&self) -> &str {
        self.inner.namespace_specific()
    }

    #[getter]
    fn stem(&self) -> Option<&str> {
        self.inner.stem()
    }

    fn set_file_name(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_file_name(value).map_err(value_error)
    }

    fn set_stem(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_stem(value).map_err(value_error)
    }

    fn set_extension(&mut self, value: &str) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_extension(value).map_err(value_error)
    }

    fn set_extensions(&mut self, values: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_extensions(strings_from_iterable(values, "extensions")?)
            .map_err(value_error)
    }

    fn remove_extension(&mut self) -> PyResult<bool> {
        self.require_mutable()?;
        Ok(self.inner.remove_extension())
    }

    fn clear_extensions(&mut self) -> PyResult<bool> {
        self.require_mutable()?;
        Ok(self.inner.clear_extensions())
    }

    #[getter]
    fn mime_type(&self) -> PyMimeType {
        PyMimeType::from_core(self.inner.mime_type())
    }

    #[getter]
    fn media_type(&self) -> PyMediaType {
        PyMediaType::from_core(self.inner.media_type())
    }

    fn set_mime_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_mime_type(core_mime_type_from_value(value)?)
            .map_err(value_error)
    }

    fn set_media_type(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner
            .set_media_type(core_media_type_from_value(value)?)
            .map_err(value_error)
    }

    #[getter]
    fn path_segments<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, ExactIterator::new(self.inner.path_segments()))
    }

    #[getter]
    fn file_name(&self) -> Option<&str> {
        self.inner.file_name()
    }

    #[getter]
    fn extension(&self) -> Option<&str> {
        self.inner.extension()
    }

    #[getter]
    fn extensions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, ExactIterator::new(self.inner.extensions()))
    }

    fn __len__(&self) -> usize {
        self.inner.path().segment_len()
    }

    fn __iter__(&self) -> PyUriPathIterator {
        let inner = self.inner.clone().into_uri();
        PyUriPathIterator {
            remaining: inner.path().segment_len(),
            inner,
            cursor: 0,
        }
    }

    fn __getitem__(&self, index: isize) -> PyResult<&str> {
        let normalized = if index >= 0 {
            usize::try_from(index).ok()
        } else {
            normalize_index(index, self.inner.path().segment_len())
        };
        normalized
            .and_then(|index| self.inner.path().get_segment(index))
            .ok_or_else(|| PyIndexError::new_err(index))
    }

    fn __contains__(&self, segment: &Bound<'_, PyAny>) -> bool {
        segment
            .extract::<&str>()
            .is_ok_and(|segment| self.inner.path().contains_segment(segment))
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Urn.from_str({:?})", self.inner.to_string())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(other.py().NotImplemented());
        };
        Ok(compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(other.py())?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __hash__(&mut self) -> isize {
        self.hash_locked = true;
        crate::python_hash(self.inner.stable_hash())
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        let callable = py.get_type::<Self>().getattr("from_str")?.unbind();
        Ok((callable, (self.inner.to_string(),)))
    }

    fn __copy__(&self) -> Self {
        Self::from_core(self.inner.clone())
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        Self::from_core(self.inner.clone())
    }
}

/// Iterator over a URI value's normalized path segments.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyUriPathIterator {
    inner: CoreUri,
    cursor: usize,
    remaining: usize,
}

#[pymethods]
impl PyUriPathIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> Option<Py<PyString>> {
        let (cursor, segment) = self.inner.next_path_segment(self.cursor)?;
        self.cursor = cursor;
        self.remaining = self.remaining.saturating_sub(1);
        Some(PyString::new(py, segment).unbind())
    }

    fn __length_hint__(&self) -> usize {
        self.remaining
    }
}
