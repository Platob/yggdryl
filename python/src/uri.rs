//! Native Python views of Yggdryl URI, URL, and URN values.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyString, PyTuple};
use yggdryl::{Parameters as CoreParameters, Uri as CoreUri, Url as CoreUrl, Urn as CoreUrn};

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

    /// Return query text without `?`, decoding its escapes when asked.
    ///
    /// `decode` chooses which text: the component's own bytes, or the text its
    /// percent escapes stand for. Decoding reads the component as text -
    /// `%26` becomes a literal `&`, not a new pair - so `parameters` is what
    /// reads a query as its pairs.
    #[pyo3(signature = (decode = false))]
    fn query(&self, decode: bool) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .query(decode)
            .map_err(value_error)?
            .map(std::borrow::Cow::into_owned))
    }

    /// Return fragment text without `#`, decoding its escapes when asked.
    #[pyo3(signature = (decode = false))]
    fn fragment(&self, decode: bool) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .fragment(decode)
            .map_err(value_error)?
            .map(std::borrow::Cow::into_owned))
    }

    /// Address the query as the `key=value` pairs it spells.
    ///
    /// The view is live: it reads and writes this value's query rather than a
    /// copy of it, so `parameters()["symbol"] = "MSFT"` changes this URI.
    #[pyo3(signature = (decode = false))]
    fn parameters(slf: &Bound<'_, Self>, decode: bool) -> PyParameters {
        PyParameters::over_uri(slf.clone().unbind(), decode)
    }

    /// Return the path as text, decoding its escapes when asked.
    ///
    /// A decoded path is text, not structure: `%2F` becomes a literal `/`
    /// inside the segment that carried it, so `path_segments` stays the way to
    /// walk structure.
    #[pyo3(signature = (decode = false))]
    fn path_text(&self, decode: bool) -> PyResult<String> {
        Ok(self
            .inner
            .path_text(decode)
            .map_err(value_error)?
            .into_owned())
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

    /// Replace the query text, or clear it with `None`.
    ///
    /// The value is the component itself, without `?`. An error leaves the
    /// value unchanged.
    #[pyo3(signature = (query, /))]
    fn set_query(&mut self, query: Option<&str>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_query(query).map_err(value_error)
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

    /// Return query text without `?`, decoding its escapes when asked.
    ///
    /// `decode` chooses which text: the component's own bytes, or the text its
    /// percent escapes stand for. Decoding reads the component as text -
    /// `%26` becomes a literal `&`, not a new pair - so `parameters` is what
    /// reads a query as its pairs.
    #[pyo3(signature = (decode = false))]
    fn query(&self, decode: bool) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .query(decode)
            .map_err(value_error)?
            .map(std::borrow::Cow::into_owned))
    }

    /// Return fragment text without `#`, decoding its escapes when asked.
    #[pyo3(signature = (decode = false))]
    fn fragment(&self, decode: bool) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .fragment(decode)
            .map_err(value_error)?
            .map(std::borrow::Cow::into_owned))
    }

    /// Address the query as the `key=value` pairs it spells.
    ///
    /// The view is live: it reads and writes this value's query rather than a
    /// copy of it, so `parameters()["symbol"] = "MSFT"` changes this URL.
    #[pyo3(signature = (decode = false))]
    fn parameters(slf: &Bound<'_, Self>, decode: bool) -> PyParameters {
        PyParameters::over_url(slf.clone().unbind(), decode)
    }

    /// Return the path as text, decoding its escapes when asked.
    ///
    /// A decoded path is text, not structure: `%2F` becomes a literal `/`
    /// inside the segment that carried it, so `path_segments` stays the way to
    /// walk structure.
    #[pyo3(signature = (decode = false))]
    fn path_text(&self, decode: bool) -> PyResult<String> {
        Ok(self
            .inner
            .path_text(decode)
            .map_err(value_error)?
            .into_owned())
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

    /// Replace the query text, or clear it with `None`.
    ///
    /// The value is the component itself, without `?`. An error leaves the
    /// value unchanged.
    #[pyo3(signature = (query, /))]
    fn set_query(&mut self, query: Option<&str>) -> PyResult<()> {
        self.require_mutable()?;
        self.inner.set_query(query).map_err(value_error)
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

    /// Return query text without `?`, decoding its escapes when asked.
    ///
    /// `decode` chooses which text: the component's own bytes, or the text its
    /// percent escapes stand for. Decoding reads the component as text -
    /// `%26` becomes a literal `&`, not a new pair - so `parameters` is what
    /// reads a query as its pairs.
    #[pyo3(signature = (decode = false))]
    fn query(&self, decode: bool) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .query(decode)
            .map_err(value_error)?
            .map(std::borrow::Cow::into_owned))
    }

    /// Return fragment text without `#`, decoding its escapes when asked.
    #[pyo3(signature = (decode = false))]
    fn fragment(&self, decode: bool) -> PyResult<Option<String>> {
        Ok(self
            .inner
            .fragment(decode)
            .map_err(value_error)?
            .map(std::borrow::Cow::into_owned))
    }

    /// Return the path as text, decoding its escapes when asked.
    ///
    /// A decoded path is text, not structure: `%2F` becomes a literal `/`
    /// inside the segment that carried it, so `path_segments` stays the way to
    /// walk structure.
    #[pyo3(signature = (decode = false))]
    fn path_text(&self, decode: bool) -> PyResult<String> {
        Ok(self
            .inner
            .path_text(decode)
            .map_err(value_error)?
            .into_owned())
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

/// Which value a [`PyParameters`] view reads and writes through.
///
/// The view holds the Python object, not a copy of its query, so a write is
/// visible on the URL a caller already has and a read never goes stale.
enum ParametersOwner {
    Uri(Py<PyUri>),
    Url(Py<PyUrl>),
}

/// A URL query, addressed as the `key=value` pairs it spells.
///
/// This is a live view of the value it was taken from - `url.parameters()` -
/// so every read goes back to that URL's query and every write replaces it.
/// Item syntax means a key: `parameters["symbol"]`, `del parameters["venue"]`,
/// and the mapping methods `keys`, `values`, `items`, `get`, `pop`,
/// `setdefault`, `update` and `clear` behave as a `dict`'s do.
///
/// A query may name one key more than once, which a `dict` cannot: `[]` and
/// `get` answer with the first, `get_all` with every one, `[]=` replaces the
/// first and drops the rest, and `append` adds another.
///
/// `decode` chooses the text the view speaks. A decoding view answers with the
/// text the escapes stand for and encodes what it is given; a raw view answers
/// with the query's own bytes and refuses text the query syntax cannot carry.
#[pyclass(name = "Parameters", module = "yggdryl._native")]
pub(crate) struct PyParameters {
    owner: ParametersOwner,
    decode: bool,
}

impl PyParameters {
    pub(crate) const fn over_uri(owner: Py<PyUri>, decode: bool) -> Self {
        Self {
            owner: ParametersOwner::Uri(owner),
            decode,
        }
    }

    pub(crate) const fn over_url(owner: Py<PyUrl>, decode: bool) -> Self {
        Self {
            owner: ParametersOwner::Url(owner),
            decode,
        }
    }

    /// Read the pairs the owner's query holds right now.
    ///
    /// The snapshot owns its pairs: the borrow on the Python object ends with
    /// this call, which is what lets an edited snapshot be written straight
    /// back to the same object.
    fn pairs(&self, py: Python<'_>) -> PyResult<CoreParameters<'static>> {
        match &self.owner {
            ParametersOwner::Uri(owner) => Ok(owner
                .bind(py)
                .try_borrow()?
                .inner
                .parameters(self.decode)
                .map_err(value_error)?
                .into_owned()),
            ParametersOwner::Url(owner) => Ok(owner
                .bind(py)
                .try_borrow()?
                .inner
                .parameters(self.decode)
                .map_err(value_error)?
                .into_owned()),
        }
    }

    /// Replace the owner's query with these pairs, refusing a frozen value.
    fn write(&self, py: Python<'_>, pairs: &CoreParameters<'_>) -> PyResult<()> {
        match &self.owner {
            ParametersOwner::Uri(owner) => {
                let mut owner = owner.bind(py).try_borrow_mut()?;
                owner.require_mutable()?;
                owner.inner.set_parameters(pairs).map_err(value_error)
            }
            ParametersOwner::Url(owner) => {
                let mut owner = owner.bind(py).try_borrow_mut()?;
                owner.require_mutable()?;
                owner.inner.set_parameters(pairs).map_err(value_error)
            }
        }
    }

    /// Read, edit, and write back in one step.
    fn edit<R>(
        &self,
        py: Python<'_>,
        edit: impl FnOnce(&mut CoreParameters<'static>) -> PyResult<R>,
    ) -> PyResult<R> {
        let mut pairs = self.pairs(py)?;
        let answer = edit(&mut pairs)?;
        self.write(py, &pairs)?;
        Ok(answer)
    }
}

#[pymethods]
impl PyParameters {
    // A live view of a mutable query cannot promise a stable hash.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// Return whether this view speaks decoded text.
    #[getter]
    const fn decode(&self) -> bool {
        self.decode
    }

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        Ok(self.pairs(py)?.len())
    }

    fn __bool__(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(!self.pairs(py)?.is_empty())
    }

    fn __contains__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        let Ok(key) = key.extract::<&str>() else {
            return Ok(false);
        };
        Ok(self.pairs(py)?.contains_key(key))
    }

    fn __getitem__(&self, py: Python<'_>, key: &str) -> PyResult<String> {
        self.pairs(py)?
            .get(key)
            .map(str::to_owned)
            .ok_or_else(|| PyKeyError::new_err(key.to_owned()))
    }

    fn __setitem__(&self, py: Python<'_>, key: &str, value: &str) -> PyResult<()> {
        self.edit(py, |pairs| {
            pairs.insert(key, value).map_err(value_error)?;
            Ok(())
        })
    }

    fn __delitem__(&self, py: Python<'_>, key: &str) -> PyResult<()> {
        self.edit(py, |pairs| {
            pairs
                .remove(key)
                .map(|_| ())
                .ok_or_else(|| PyKeyError::new_err(key.to_owned()))
        })
    }

    fn __iter__(&self, py: Python<'_>) -> PyResult<PyParameterIterator> {
        self.keys(py)
    }

    /// Return every key, in the order the query spells them.
    fn keys(&self, py: Python<'_>) -> PyResult<PyParameterIterator> {
        Ok(PyParameterIterator::new(
            self.pairs(py)?
                .keys()
                .map(str::to_owned)
                .map(ParameterEntry::One)
                .collect(),
        ))
    }

    /// Return every value, in order, repeated keys included.
    fn values(&self, py: Python<'_>) -> PyResult<PyParameterIterator> {
        Ok(PyParameterIterator::new(
            self.pairs(py)?
                .values()
                .map(str::to_owned)
                .map(ParameterEntry::One)
                .collect(),
        ))
    }

    /// Return every `(key, value)` pair, in order.
    fn items(&self, py: Python<'_>) -> PyResult<PyParameterIterator> {
        Ok(PyParameterIterator::new(
            self.pairs(py)?
                .iter()
                .map(|(key, value)| ParameterEntry::Pair(key.to_owned(), value.to_owned()))
                .collect(),
        ))
    }

    /// Return the first value named `key`, or `default` when there is none.
    #[pyo3(signature = (key, default=None, /))]
    fn get(&self, py: Python<'_>, key: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        Ok(self.pairs(py)?.get(key).map_or_else(
            || default.unwrap_or_else(|| py.None()),
            |value| PyString::new(py, value).into_any().unbind(),
        ))
    }

    /// Return every value named `key`, in order.
    fn get_all<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Bound<'py, PyTuple>> {
        let values: Vec<String> = self.pairs(py)?.get_all(key).map(str::to_owned).collect();
        PyTuple::new(py, values)
    }

    /// Add another pair named `key`, keeping the pairs already there.
    fn append(&self, py: Python<'_>, key: &str, value: &str) -> PyResult<()> {
        self.edit(py, |pairs| pairs.append(key, value).map_err(value_error))
    }

    /// Remove every pair named `key`, returning the first value removed.
    ///
    /// A key the query does not hold raises unless a default is passed, which
    /// is what `dict.pop` does - and why the default is variadic: passing
    /// `None` as the default has to mean `None`, not "no default".
    #[pyo3(signature = (key, /, *default))]
    fn pop(&self, py: Python<'_>, key: &str, default: &Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> {
        if default.len() > 1 {
            return Err(PyTypeError::new_err(format!(
                "pop expected at most 2 arguments, got {}",
                default.len() + 1
            )));
        }
        let removed = self.edit(py, |pairs| Ok(pairs.remove(key)))?;
        if let Some(value) = removed {
            return Ok(PyString::new(py, &value).into_any().unbind());
        }
        default
            .get_item(0)
            .map(Bound::unbind)
            .map_err(|_| PyKeyError::new_err(key.to_owned()))
    }

    /// Return the value named `key`, setting it to `default` when absent.
    #[pyo3(signature = (key, default="", /))]
    fn setdefault(&self, py: Python<'_>, key: &str, default: &str) -> PyResult<String> {
        self.edit(py, |pairs| {
            if let Some(value) = pairs.get(key) {
                return Ok(value.to_owned());
            }
            pairs.append(key, default).map_err(value_error)?;
            Ok(default.to_owned())
        })
    }

    /// Set every pair `values` and the keyword arguments name.
    #[pyo3(signature = (values=None, /, **kwargs))]
    fn update(
        &self,
        py: Python<'_>,
        values: Option<&Bound<'_, PyAny>>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let mut entries: Vec<(String, String)> = Vec::new();
        if let Some(values) = values {
            if let Ok(mapping) = values.cast::<PyDict>() {
                for (key, value) in mapping.iter() {
                    entries.push((key.extract()?, value.extract()?));
                }
            } else {
                for entry in values.try_iter()? {
                    let entry = entry?;
                    let pair = entry.extract::<(String, String)>().map_err(|_| {
                        PyTypeError::new_err(
                            "expected a mapping or an iterable of (key, value) pairs",
                        )
                    })?;
                    entries.push(pair);
                }
            }
        }
        if let Some(kwargs) = kwargs {
            for (key, value) in kwargs.iter() {
                entries.push((key.extract()?, value.extract()?));
            }
        }
        self.edit(py, |pairs| {
            for (key, value) in &entries {
                pairs.insert(key, value).map_err(value_error)?;
            }
            Ok(())
        })
    }

    /// Drop every pair, clearing the query.
    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        self.edit(py, |pairs| {
            pairs.clear();
            Ok(())
        })
    }

    /// Return the pairs as a `dict`, keeping the first value of a repeated key.
    #[allow(clippy::wrong_self_convention)]
    fn into_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        for (key, value) in self.pairs(py)?.iter() {
            if !dict.contains(key)? {
                dict.set_item(key, value)?;
            }
        }
        Ok(dict)
    }

    /// Return the query component these pairs spell, or `None` when empty.
    #[allow(clippy::wrong_self_convention)]
    fn into_query(&self, py: Python<'_>) -> PyResult<Option<String>> {
        Ok(self.pairs(py)?.into_query().map(Into::into))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let pairs = self.pairs(py)?;
        let entries: Vec<String> = pairs
            .iter()
            .map(|(key, value)| format!("{key:?}: {value:?}"))
            .collect();
        Ok(format!("Parameters({{{}}})", entries.join(", ")))
    }

    /// Compare the pairs, independently of the values holding them.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let left = self.pairs(py)?;
        if let Ok(other) = other.extract::<PyRef<'_, Self>>() {
            let right = other.pairs(py)?;
            let equal = left.iter().eq(right.iter());
            return Ok(PyBool::new(py, equal).to_owned().into_any().unbind());
        }
        let Ok(mapping) = other.cast::<PyDict>() else {
            return Ok(py.NotImplemented());
        };
        let equal = left.len() == mapping.len()
            && left.iter().try_fold(true, |equal, (key, value)| {
                Ok::<bool, PyErr>(
                    equal
                        && mapping.get_item(key)?.is_some_and(|held| {
                            held.extract::<&str>().is_ok_and(|held| held == value)
                        }),
                )
            })?;
        Ok(PyBool::new(py, equal).to_owned().into_any().unbind())
    }
}

/// One entry a [`PyParameterIterator`] yields.
enum ParameterEntry {
    One(String),
    Pair(String, String),
}

/// Iterator over a query's keys, values, or pairs.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyParameterIterator {
    entries: std::vec::IntoIter<ParameterEntry>,
}

impl PyParameterIterator {
    fn new(entries: Vec<ParameterEntry>) -> Self {
        Self {
            entries: entries.into_iter(),
        }
    }
}

#[pymethods]
impl PyParameterIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> Option<Py<PyAny>> {
        match self.entries.next()? {
            ParameterEntry::One(value) => Some(PyString::new(py, &value).into_any().unbind()),
            ParameterEntry::Pair(key, value) => {
                Some(PyTuple::new(py, [key, value]).ok()?.into_any().unbind())
            }
        }
    }

    fn __length_hint__(&self) -> usize {
        self.entries.len()
    }
}
