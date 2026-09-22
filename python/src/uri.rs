//! Native Python views of Yggdryl URI, URL, URN, and ARN values.
//!
//! `Uri` is the class every identifier is, and `Url`, `Urn`, and `Arn` are the
//! three narrowings of it the scheme decides. They are Python subclasses of
//! `Uri` because that is what they are: one canonical value, held once by the
//! base, read by whichever class the scheme named. The shared vocabulary -
//! components, path, suffixes, comparison, hashing, pickling - is therefore
//! written once, and a narrowed class carries only what is its own.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyString, PyTuple};
use yggdryl::{
    Arn as CoreArn, Authority as CoreAuthority, Parameters as CoreParameters, Scheme as CoreScheme,
    Uri as CoreUri, UriPath as CoreUriPath, Url as CoreUrl, Urn as CoreUrn,
};

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

// ---------------------------------------------------------------------------
// Intake: every spelling of an identifier a caller may hand across.
// ---------------------------------------------------------------------------

/// Read any identifier a caller named as the canonical URI it is.
///
/// Every narrowed class is a `Uri`, so one borrow reads all four.
pub(crate) fn core_uri_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreUri> {
    if let Ok(value) = value.extract::<PyRef<'_, PyUri>>() {
        return Ok(value.inner.clone());
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreUri::from_str(value).map_err(value_error);
    }
    if value.hasattr("__fspath__")? {
        return CoreUri::from_path(path_string_from_value(value)?).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a yggdryl.Uri, yggdryl.Url, yggdryl.Urn, yggdryl.Arn, or URI string",
    ))
}

/// Read one location a caller named: a URL, a name, or a path to root.
///
/// An identifier crosses through `locator`, so a name opens as well as a
/// location does: a URN resolves to the path it spells and an Amazon S3 ARN to
/// the `s3:` URL it addresses. Text carrying a scheme is a URL and text
/// carrying none is a path rooted at the working directory.
pub(crate) fn core_url_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreUrl> {
    if let Ok(value) = value.extract::<PyRef<'_, PyUri>>() {
        return value.inner.locator().map_err(value_error);
    }
    if let Ok(value) = value.extract::<&str>() {
        return CoreUrl::from_location(value).map_err(value_error);
    }
    if value.hasattr("__fspath__")? {
        return CoreUrl::from_path(path_string_from_value(value)?).map_err(value_error);
    }
    Err(PyTypeError::new_err(
        "expected a yggdryl.Url, yggdryl.Uri, yggdryl.Urn, yggdryl.Arn, URL string, or path-like value",
    ))
}

/// Read any identifier a caller named as the URN it is.
fn core_urn_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreUrn> {
    if let Ok(value) = value.extract::<&str>() {
        return CoreUrn::from_str(value).map_err(value_error);
    }
    CoreUrn::from_uri(core_uri_from_value(value)?).map_err(value_error)
}

/// Read any identifier a caller named as the ARN it is.
fn core_arn_from_value(value: &Bound<'_, PyAny>) -> PyResult<CoreArn> {
    if let Ok(value) = value.extract::<&str>() {
        return CoreArn::from_str(value).map_err(value_error);
    }
    CoreArn::from_uri(core_uri_from_value(value)?).map_err(value_error)
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

/// Extend `url` with one component, choosing the core join the value names.
///
/// A `str` is one URL path component and joins as written. An `os.PathLike`
/// is an operating-system path, so it joins through `Url::join_path`, which
/// reads its own separators and its drive prefix rather than treating the
/// whole path as one segment.
fn join_url_component(url: &CoreUrl, value: &Bound<'_, PyAny>) -> PyResult<CoreUrl> {
    if let Ok(text) = value.extract::<&str>() {
        return url.joinpath(text).map_err(value_error);
    }
    url.join_path(path_string_from_value(value)?)
        .map_err(value_error)
}

fn path_string_from_core(value: std::path::PathBuf) -> PyResult<String> {
    value.into_os_string().into_string().map_err(|_| {
        PyValueError::new_err("file URI path cannot be represented as a Python string")
    })
}

// ---------------------------------------------------------------------------
// Answering: the class an identifier turns out to be.
// ---------------------------------------------------------------------------

/// Answer `value` as the Python class its scheme names.
///
/// This is the one place a parsed identifier becomes a Python object, so a
/// caller who wrote `Uri(...)` receives the narrowing that value is - a `Url`,
/// a `Urn`, or an `Arn` - and a value that is none of them stays a `Uri`. Each
/// narrowing is asked by its own core validator rather than by a reading this
/// binding invents, and the value stored is the one that validator
/// canonicalized.
pub(crate) fn describe(py: Python<'_>, value: CoreUri) -> PyResult<Py<PyAny>> {
    if let Ok(urn) = CoreUrn::from_uri(value.clone()) {
        return Ok(urn_object(py, urn)?.into_any());
    }
    if let Ok(arn) = CoreArn::from_uri(value.clone()) {
        return Ok(arn_object(py, arn)?.into_any());
    }
    if let Ok(url) = CoreUrl::from_uri(value.clone()) {
        return Ok(url_object(py, url)?.into_any());
    }
    Ok(Py::new(py, PyUri::from_core(value))?.into_any())
}

/// Build the base every narrowed class carries.
fn narrowed(value: CoreUri) -> PyClassInitializer<PyUri> {
    PyClassInitializer::from(PyUri::from_core(value))
}

pub(crate) fn url_object(py: Python<'_>, value: CoreUrl) -> PyResult<Py<PyUrl>> {
    Py::new(py, narrowed(value.into_uri()).add_subclass(PyUrl))
}

fn urn_object(py: Python<'_>, value: CoreUrn) -> PyResult<Py<PyUrn>> {
    Py::new(py, narrowed(value.into_uri()).add_subclass(PyUrn))
}

fn arn_object(py: Python<'_>, value: CoreArn) -> PyResult<Py<PyArn>> {
    Py::new(py, narrowed(value.into_uri()).add_subclass(PyArn))
}

/// Store `candidate` on `slf`, refusing what its class would no longer hold.
///
/// A narrowed class is a view of one canonical identifier, so an edit made
/// through the vocabulary every identifier shares has to leave the value still
/// being what the class says it is. This is the one place that is decided, and
/// each narrowing is asked by its own core validator.
fn hold_narrowed(slf: &Bound<'_, PyUri>, candidate: CoreUri) -> PyResult<()> {
    let object = slf.as_any();
    if object.is_instance_of::<PyUrn>() {
        CoreUrn::from_uri(candidate.clone()).map_err(value_error)?;
    } else if object.is_instance_of::<PyArn>() {
        CoreArn::from_uri(candidate.clone()).map_err(value_error)?;
    } else if object.is_instance_of::<PyUrl>() {
        CoreUrl::from_uri(candidate.clone()).map_err(value_error)?;
    }
    slf.borrow_mut().inner = candidate;
    Ok(())
}

/// Apply one core edit to the identifier `slf` holds, atomically.
///
/// The edit runs on a candidate, so a refusal - the core's own, or the
/// narrowing's - leaves the value exactly as it was.
fn edit_identifier(
    slf: &Bound<'_, PyUri>,
    edit: impl FnOnce(&mut CoreUri) -> yggdryl::Result<()>,
) -> PyResult<()> {
    let mut candidate = {
        let held = slf.borrow();
        held.require_mutable()?;
        held.inner.clone()
    };
    edit(&mut candidate).map_err(value_error)?;
    hold_narrowed(slf, candidate)
}

/// Apply one core edit that reports whether it changed anything.
fn edit_identifier_if(
    slf: &Bound<'_, PyUri>,
    edit: impl FnOnce(&mut CoreUri) -> bool,
) -> PyResult<bool> {
    let mut candidate = {
        let held = slf.borrow();
        held.require_mutable()?;
        held.inner.clone()
    };
    if !edit(&mut candidate) {
        return Ok(false);
    }
    hold_narrowed(slf, candidate)?;
    Ok(true)
}

/// Read the URL a narrowed view stands on, which its class already proved.
fn url_of(base: &PyUri) -> PyResult<CoreUrl> {
    CoreUrl::from_uri(base.inner.clone()).map_err(value_error)
}

/// Read the URN a narrowed view stands on, which its class already proved.
fn urn_of(base: &PyUri) -> PyResult<CoreUrn> {
    CoreUrn::from_uri(base.inner.clone()).map_err(value_error)
}

/// Read the ARN a narrowed view stands on, which its class already proved.
fn arn_of(base: &PyUri) -> PyResult<CoreArn> {
    CoreArn::from_uri(base.inner.clone()).map_err(value_error)
}

// ---------------------------------------------------------------------------
// `Uri`: the identifier itself, and the base class of every narrowing.
// ---------------------------------------------------------------------------

/// A normalized URI with sequence access to its path segments.
///
/// The Python view stays mutable until it is first hashed. Hashing locks that
/// one wrapper so its canonical value remains stable as a mapping key.
///
/// Calling `Uri(value)` answers the narrowing the scheme names - a `Url`, a
/// `Urn`, or an `Arn` - and a `Uri` when the value is none of them. The named
/// doors `from_str`, `from_path`, `from_parts` and `from_json` answer this
/// class itself, which is what a round trip through `copy` and `pickle` needs.
#[pyclass(
    name = "Uri",
    module = "yggdryl._native",
    subclass,
    skip_from_py_object
)]
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
                "a hashed identifier is frozen; copy it before mutation",
            ))
        } else {
            Ok(())
        }
    }
}

#[pymethods]
impl PyUri {
    /// Read any identifier, answering the narrowing its scheme names.
    #[new]
    #[allow(clippy::new_ret_no_self)] // `Uri(...)` answers a `Url`, `Urn`, or `Arn`.
    fn new(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        describe(py, core_uri_from_value(value)?)
    }

    /// Read any identifier, answering the narrowing its scheme names.
    #[staticmethod]
    fn from_value(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::new(py, value)
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

    /// Build a validated URI from its five components.
    ///
    /// Each component crosses as the text it is written as, and the core
    /// validates them against one another: a non-empty authority requires the
    /// `//` marker, a path after an authority is empty or slash-rooted, and a
    /// marker-less path does not begin with `//`.
    #[staticmethod]
    #[pyo3(signature = (scheme, authority, path, query = None, fragment = None))]
    fn from_parts(
        scheme: &str,
        authority: &str,
        path: &str,
        query: Option<&str>,
        fragment: Option<&str>,
    ) -> PyResult<Self> {
        CoreUri::from_parts(
            CoreScheme::from_str(scheme).map_err(value_error)?,
            CoreAuthority::from_str(authority).map_err(value_error)?,
            CoreUriPath::from_str(path).map_err(value_error)?,
            query.map(Into::into),
            fragment.map(Into::into),
        )
        .map(Self::from_core)
        .map_err(value_error)
    }

    #[staticmethod]
    fn from_json(value: &str) -> PyResult<Self> {
        CoreUri::from_json(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    fn validate(&self) -> PyResult<()> {
        self.inner.validate().map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.clone().into_json().map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_url(&self, py: Python<'_>) -> PyResult<Py<PyUrl>> {
        url_object(py, url_of(self)?)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_urn(&self, py: Python<'_>) -> PyResult<Py<PyUrn>> {
        urn_object(py, urn_of(self)?)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_arn(&self, py: Python<'_>) -> PyResult<Py<PyArn>> {
        arn_object(py, arn_of(self)?)
    }

    /// The location this identifier names, as a `Url`.
    ///
    /// A location locates itself; a name resolves to where it is - a URN under
    /// the process working directory, an Amazon S3 ARN to the `s3:` URL its
    /// bucket and key address - which is what lets any identifier be handed to
    /// a reader as the thing to open.
    fn locator(&self, py: Python<'_>) -> PyResult<Py<PyUrl>> {
        url_object(py, self.inner.locator().map_err(value_error)?)
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
    fn key(&self) -> Option<&str> {
        self.inner.key()
    }

    #[getter]
    fn region(&self) -> Option<&str> {
        self.inner.region()
    }

    /// The Azure storage account this location names, when it names one.
    #[getter]
    fn account(&self) -> Option<&str> {
        self.inner.account()
    }

    /// The store endpoint host with its explicit port, without a virtual
    /// container.
    #[getter]
    fn store_endpoint(&self) -> Option<&str> {
        self.inner.store_endpoint()
    }

    /// Return whether a store location writes its container into the hostname.
    fn is_virtual_hosted(&self) -> bool {
        self.inner.is_virtual_hosted()
    }

    /// The host with its optional port, without user information.
    #[getter]
    fn host_port(&self) -> &str {
        self.inner.authority().host_port()
    }

    /// The explicit port written in the authority, when one was written.
    #[getter]
    fn port(&self) -> Option<u16> {
        self.inner.authority().port()
    }

    /// The port a client dials when the authority omits one.
    ///
    /// This is the scheme's registered default, never a port written into the
    /// authority, which `port` answers.
    #[getter]
    fn default_port(&self) -> Option<u16> {
        self.inner.default_port()
    }

    /// Return whether the scheme addresses byte-oriented storage.
    fn is_storage(&self) -> bool {
        self.inner.scheme().is_storage()
    }

    #[getter]
    fn has_authority(&self) -> bool {
        self.inner.has_authority()
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
    /// copy of it, so `parameters()["symbol"] = "MSFT"` changes this value.
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

    /// The path components, with `.` dropped and `..` applied.
    #[getter]
    fn parts<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.inner.parts())
    }

    #[getter]
    fn parent(&self) -> Self {
        Self::from_core(self.inner.parent().unwrap_or_else(|| self.inner.clone()))
    }

    #[getter]
    fn parents<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let parents: Vec<Self> = self.inner.parents().map(Self::from_core).collect();
        PyTuple::new(py, parents)
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
    fn set_query(slf: &Bound<'_, Self>, query: Option<&str>) -> PyResult<()> {
        edit_identifier(slf, |uri| uri.set_query(query))
    }

    fn set_file_name(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_identifier(slf, |uri| uri.set_file_name(value))
    }

    fn set_stem(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_identifier(slf, |uri| uri.set_stem(value))
    }

    fn set_extension(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_identifier(slf, |uri| uri.set_extension(value))
    }

    fn set_extensions(slf: &Bound<'_, Self>, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let values = strings_from_iterable(values, "extensions")?;
        edit_identifier(slf, |uri| uri.set_extensions(values))
    }

    fn remove_extension(slf: &Bound<'_, Self>) -> PyResult<bool> {
        edit_identifier_if(slf, CoreUri::remove_extension)
    }

    fn clear_extensions(slf: &Bound<'_, Self>) -> PyResult<bool> {
        edit_identifier_if(slf, CoreUri::clear_extensions)
    }

    #[getter]
    fn mime_type(&self) -> PyMimeType {
        PyMimeType::from_core(self.inner.mime_type())
    }

    #[getter]
    fn media_type(&self) -> PyMediaType {
        PyMediaType::from_core(self.inner.media_type())
    }

    fn set_mime_type(slf: &Bound<'_, Self>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = core_mime_type_from_value(value)?;
        edit_identifier(slf, |uri| uri.set_mime_type(value))
    }

    fn set_media_type(slf: &Bound<'_, Self>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = core_media_type_from_value(value)?;
        edit_identifier(slf, |uri| uri.set_media_type(value))
    }

    /// Return this identifier with path components joined by the core path
    /// resolver.
    ///
    /// Scheme, authority, query, and fragment are preserved. Relative values
    /// extend the path with `.` and `..` resolved; an absolute value replaces
    /// the path. The source is never mutated, including after it is
    /// hash-locked.
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
        PyUriPathIterator {
            remaining: self.inner.path().segment_len(),
            inner: self.inner.clone(),
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

    /// The expression that rebuilds this value, naming the class it is.
    ///
    /// The class is read off the value rather than written into each one, so a
    /// narrowing can never report the base class by mistake.
    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(format!(
            "{}.from_str({:?})",
            slf.get_type().name()?,
            slf.borrow().inner.to_string()
        ))
    }

    /// Identifiers compare as the canonical URI they hold.
    ///
    /// A `Url` and the `Uri` it narrows are one identifier, so they compare and
    /// hash as one; two schemes can never spell the same text, so a name and a
    /// location still never meet.
    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let Ok(other) = other.extract::<PyRef<'_, PyUri>>() else {
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

    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<(Py<PyAny>, (String,))> {
        let callable = slf.get_type().getattr("from_str")?.unbind();
        let text = slf.borrow().inner.to_string();
        Ok((callable, (text,)))
    }

    fn __copy__(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let text = slf.borrow().inner.to_string();
        Ok(slf.get_type().call_method1("from_str", (text,))?.unbind())
    }

    fn __deepcopy__(slf: &Bound<'_, Self>, _memo: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Self::__copy__(slf)
    }
}

// ---------------------------------------------------------------------------
// `Url`: a location, and everything `pathlib` asks of one.
// ---------------------------------------------------------------------------

/// A URL view validated and normalized by the core URI model.
///
/// A `Url` is a `Uri` whose scheme spells a location, so it reads every
/// component through the base and adds what only a location answers: the
/// `pathlib` vocabulary, glob matching, Hive partitions, and the local
/// filesystem predicates.
#[pyclass(
    name = "Url",
    module = "yggdryl._native",
    extends = PyUri,
    skip_from_py_object
)]
pub(crate) struct PyUrl;

#[pymethods]
impl PyUrl {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        Ok(narrowed(core_url_from_value(value)?.into_uri()).add_subclass(Self))
    }

    #[staticmethod]
    fn from_value(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<Self>> {
        url_object(py, core_url_from_value(value)?)
    }

    #[staticmethod]
    fn from_str(py: Python<'_>, value: &str) -> PyResult<Py<Self>> {
        url_object(py, CoreUrl::from_str(value).map_err(value_error)?)
    }

    #[staticmethod]
    fn from_path(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<Self>> {
        let value = CoreUrl::from_path(path_string_from_value(value)?).map_err(value_error)?;
        url_object(py, value)
    }

    #[staticmethod]
    fn from_uri(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<Self>> {
        let value = CoreUrl::from_uri(core_uri_from_value(value)?).map_err(value_error)?;
        url_object(py, value)
    }

    #[staticmethod]
    fn from_json(py: Python<'_>, value: &str) -> PyResult<Py<Self>> {
        url_object(py, CoreUrl::from_json(value).map_err(value_error)?)
    }

    /// This location as the identifier it narrows.
    #[allow(clippy::wrong_self_convention)]
    fn into_uri(slf: &Bound<'_, Self>) -> PyUri {
        PyUri::from_core(slf.as_super().borrow().inner.clone())
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
    fn name(slf: &Bound<'_, Self>) -> String {
        slf.as_super()
            .borrow()
            .inner
            .file_name()
            .unwrap_or_default()
            .to_string()
    }

    /// The final extension with its leading dot, as `PurePath.suffix`.
    #[getter]
    fn suffix(slf: &Bound<'_, Self>) -> String {
        slf.as_super()
            .borrow()
            .inner
            .extension()
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default()
    }

    /// Every extension with leading dots, as `PurePath.suffixes`.
    #[getter]
    fn suffixes<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyTuple>> {
        let suffixes: Vec<String> = slf
            .as_super()
            .borrow()
            .inner
            .extensions()
            .map(|extension| format!(".{extension}"))
            .collect();
        PyTuple::new(slf.py(), suffixes)
    }

    /// The containing location, as `PurePath.parent`.
    ///
    /// A location at the root is its own parent, which is what `pathlib` does.
    #[getter]
    fn parent(slf: &Bound<'_, Self>) -> PyResult<Py<Self>> {
        let url = url_of(&slf.as_super().borrow())?;
        let parent = url.parent().unwrap_or(url);
        url_object(slf.py(), parent)
    }

    /// Every containing location, closest first, as `PurePath.parents`.
    #[getter]
    fn parents<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyTuple>> {
        let py = slf.py();
        let parents: Vec<Py<Self>> = url_of(&slf.as_super().borrow())?
            .parents()
            .map(|parent| url_object(py, parent))
            .collect::<PyResult<_>>()?;
        PyTuple::new(py, parents)
    }

    /// Join path components onto this location, as `PurePath.joinpath`.
    ///
    /// A `str` is one URL path component; an `os.PathLike` is an operating
    /// system path, joined component by component with its own separators.
    #[pyo3(signature = (*others))]
    fn joinpath(slf: &Bound<'_, Self>, others: &Bound<'_, PyTuple>) -> PyResult<Py<Self>> {
        let mut joined = url_of(&slf.as_super().borrow())?;
        for other in others {
            joined = join_url_component(&joined, &other)?;
        }
        url_object(slf.py(), joined)
    }

    /// `url / "child"`, as `PurePath.__truediv__`.
    fn __truediv__(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<Self>> {
        let joined = join_url_component(&url_of(&slf.as_super().borrow())?, other)?;
        url_object(slf.py(), joined)
    }

    /// This location with a different final component, as `with_name`.
    fn with_name(slf: &Bound<'_, Self>, value: &str) -> PyResult<Py<Self>> {
        let mut renamed = url_of(&slf.as_super().borrow())?;
        renamed.set_file_name(value).map_err(value_error)?;
        url_object(slf.py(), renamed)
    }

    /// This location with a different stem, as `with_stem`.
    fn with_stem(slf: &Bound<'_, Self>, value: &str) -> PyResult<Py<Self>> {
        let mut renamed = url_of(&slf.as_super().borrow())?;
        renamed.set_stem(value).map_err(value_error)?;
        url_object(slf.py(), renamed)
    }

    /// This location with a different final extension, as `with_suffix`.
    ///
    /// The leading dot is optional, and an empty suffix removes the extension.
    fn with_suffix(slf: &Bound<'_, Self>, value: &str) -> PyResult<Py<Self>> {
        let mut renamed = url_of(&slf.as_super().borrow())?;
        let suffix = value.strip_prefix('.').unwrap_or(value);
        if suffix.is_empty() {
            renamed.remove_extension();
        } else {
            renamed.set_extension(suffix).map_err(value_error)?;
        }
        url_object(slf.py(), renamed)
    }

    /// A URL path is always absolute, as `PurePath.is_absolute`.
    #[allow(clippy::unused_self)]
    fn is_absolute(&self) -> bool {
        true
    }

    /// The path in POSIX form, as `PurePath.as_posix`.
    fn as_posix(slf: &Bound<'_, Self>) -> String {
        slf.as_super().borrow().inner.path().as_str().to_string()
    }

    /// The whole location as text, as `PurePath.as_uri`.
    fn as_uri(slf: &Bound<'_, Self>) -> String {
        slf.as_super().borrow().inner.to_string()
    }

    /// Return whether this location matches `pattern`, as `PurePath.match`.
    ///
    /// A pattern with no separator matches the name at any depth; one with a
    /// separator is anchored at the path root.
    #[pyo3(name = "match")]
    fn matches(slf: &Bound<'_, Self>, pattern: &str) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.matches_glob(pattern))
    }

    /// Return whether the whole path matches, as `PurePath.full_match`.
    fn full_match(slf: &Bound<'_, Self>, pattern: &str) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.matches_glob(pattern))
    }

    /// Return whether this location is a glob pattern rather than one name.
    fn is_glob(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.is_glob())
    }

    /// Return whether the pattern crosses directory boundaries.
    ///
    /// A `**` segment is what makes a walk recurse rather than list one level.
    fn is_recursive_glob(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.is_recursive_glob())
    }

    /// Return whether `text` is a pattern rather than one plain name.
    ///
    /// This is what a walk asks of each pattern segment to decide whether it
    /// can descend into it directly or has to list and filter.
    #[staticmethod]
    fn is_pattern(text: &str) -> bool {
        CoreUrl::is_pattern(text)
    }

    /// Split a glob into the fixed location it starts from and its pattern.
    ///
    /// The root is the deepest place a listing can start; the pattern is the
    /// rest, written relative to that root, which is what `full_match_under`
    /// takes. A location that is not a glob is its own root with no pattern.
    fn glob_parts(slf: &Bound<'_, Self>) -> PyResult<(Py<Self>, Option<String>)> {
        let (root, pattern) = url_of(&slf.as_super().borrow())?
            .glob_parts()
            .map_err(value_error)?;
        Ok((url_object(slf.py(), root)?, pattern))
    }

    /// Return whether the path below `root` matches `pattern`.
    ///
    /// The pattern is anchored at `root` rather than at the path root, which
    /// is how a listing filters what `glob_parts` handed it. A location
    /// outside `root` never matches.
    fn full_match_under(
        slf: &Bound<'_, Self>,
        root: &Bound<'_, PyAny>,
        pattern: &str,
    ) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?
            .matches_glob_under(&core_url_from_value(root)?, pattern))
    }

    /// Return this location relative to `other`, as `PurePath.relative_to`.
    ///
    /// Raises `ValueError` when this location is not below `other`, which is
    /// what `pathlib` does.
    fn relative_to(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<String> {
        let value = url_of(&slf.as_super().borrow())?;
        let root = core_url_from_value(other)?;
        value
            .segments_under(&root)
            .map(|segments| segments.join("/"))
            .ok_or_else(|| {
                PyValueError::new_err(format!("{value} is not in the subpath of {root}"))
            })
    }

    /// Return whether this location is below `other`, as `is_relative_to`.
    fn is_relative_to(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?
            .segments_under(&core_url_from_value(other)?)
            .is_some())
    }

    /// Return whether something exists here now, as `Path.exists`.
    fn exists(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.exists())
    }

    /// Return whether this location is a directory, as `Path.is_dir`.
    fn is_dir(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.is_dir())
    }

    /// Return whether this location is a regular file, as `Path.is_file`.
    fn is_file(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.is_file())
    }

    /// Return whether the name begins with a dot, so a listing may skip it.
    fn is_private(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.is_private())
    }

    /// Return whether this location is on the local file system.
    ///
    /// Only a local URL converts to a path; every other scheme needs a client.
    fn is_local(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.is_local())
    }

    /// The MIME type of the local entry this location addresses.
    ///
    /// An existing directory is `application/x-directory`; another local entry
    /// is identified from its name, falling back to the generic file type. A
    /// remote location answers `mime_type`, with no network call.
    #[getter]
    fn local_mime_type(slf: &Bound<'_, Self>) -> PyResult<PyMimeType> {
        Ok(PyMimeType::from_core(
            url_of(&slf.as_super().borrow())?.local_mime_type(),
        ))
    }

    /// The Hive partition pairs this location's path spells out.
    #[getter]
    fn partitions<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            slf.py(),
            url_of(&slf.as_super().borrow())?.hive_partitions(),
        )
    }

    /// Return the value of one Hive partition column, when the path has it.
    fn partition(slf: &Bound<'_, Self>, column: &str) -> PyResult<Option<String>> {
        Ok(url_of(&slf.as_super().borrow())?.hive_partition(column))
    }

    /// The Hive partition pairs this location spells out below `root`.
    ///
    /// A directory that is part of the table's address is not a partition of
    /// it, so `/lake/year=2024` under `/lake/year=2024` spells out nothing. A
    /// location outside `root` spells out nothing either.
    fn partitions_under<'py>(
        slf: &Bound<'py, Self>,
        root: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyTuple>> {
        let pairs =
            url_of(&slf.as_super().borrow())?.hive_partitions_under(&core_url_from_value(root)?);
        PyTuple::new(slf.py(), pairs)
    }

    /// Return whether any path segment is a `column=value` partition.
    fn is_partitioned(slf: &Bound<'_, Self>) -> PyResult<bool> {
        Ok(url_of(&slf.as_super().borrow())?.is_hive_partitioned())
    }

    /// Extend this location with one `column=value` partition directory.
    fn with_partition(slf: &Bound<'_, Self>, column: &str, value: &str) -> PyResult<Py<Self>> {
        let extended = url_of(&slf.as_super().borrow())?
            .with_hive_partition(column, value)
            .map_err(value_error)?;
        url_object(slf.py(), extended)
    }
}

// ---------------------------------------------------------------------------
// `Urn`: a name, and where it resolves to.
// ---------------------------------------------------------------------------

/// A URN view with namespace-specific accessors.
///
/// A `Urn` is a `Uri` whose scheme spells a name. Its filename accessors read
/// the namespace-specific string rather than the whole path, and `locator`
/// answers where the name resolves to, which is what makes a name openable.
#[pyclass(
    name = "Urn",
    module = "yggdryl._native",
    extends = PyUri,
    skip_from_py_object
)]
pub(crate) struct PyUrn;

#[pymethods]
impl PyUrn {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        Ok(narrowed(core_urn_from_value(value)?.into_uri()).add_subclass(Self))
    }

    #[staticmethod]
    fn from_value(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<Self>> {
        urn_object(py, core_urn_from_value(value)?)
    }

    #[staticmethod]
    fn from_str(py: Python<'_>, value: &str) -> PyResult<Py<Self>> {
        urn_object(py, CoreUrn::from_str(value).map_err(value_error)?)
    }

    #[staticmethod]
    fn from_uri(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<Self>> {
        let value = CoreUrn::from_uri(core_uri_from_value(value)?).map_err(value_error)?;
        urn_object(py, value)
    }

    #[staticmethod]
    fn from_json(py: Python<'_>, value: &str) -> PyResult<Py<Self>> {
        urn_object(py, CoreUrn::from_json(value).map_err(value_error)?)
    }

    /// This name as the identifier it narrows.
    #[allow(clippy::wrong_self_convention)]
    fn into_uri(slf: &Bound<'_, Self>) -> PyUri {
        PyUri::from_core(slf.as_super().borrow().inner.clone())
    }

    /// The canonical lowercase namespace identifier.
    #[getter]
    fn namespace(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(urn_of(&slf.as_super().borrow())?.namespace().to_string())
    }

    /// The namespace-specific string, exactly as it was written.
    #[getter]
    fn namespace_specific(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(urn_of(&slf.as_super().borrow())?
            .namespace_specific()
            .to_string())
    }

    /// The relative path this name spells.
    ///
    /// The namespace leads it and the namespace-specific string's `:`
    /// separators are the ones after it, so `urn:lake:trades:2026:part.parquet`
    /// spells `lake/trades/2026/part.parquet`.
    fn locator_path(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(urn_of(&slf.as_super().borrow())?
            .locator_path()
            .map_err(value_error)?
            .as_str()
            .to_string())
    }

    /// Resolve this name under `base`, answering where it is.
    fn resolve(slf: &Bound<'_, Self>, base: &Bound<'_, PyAny>) -> PyResult<Py<PyUrl>> {
        let located = urn_of(&slf.as_super().borrow())?
            .resolve(&core_url_from_value(base)?)
            .map_err(value_error)?;
        url_object(slf.py(), located)
    }

    #[getter]
    fn file_name(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(urn_of(&slf.as_super().borrow())?
            .file_name()
            .map(str::to_string))
    }

    #[getter]
    fn stem(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(urn_of(&slf.as_super().borrow())?.stem().map(str::to_string))
    }

    #[getter]
    fn extension(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(urn_of(&slf.as_super().borrow())?
            .extension()
            .map(str::to_string))
    }

    #[getter]
    fn extensions<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyTuple>> {
        let urn = urn_of(&slf.as_super().borrow())?;
        let extensions: Vec<&str> = urn.extensions().collect();
        PyTuple::new(slf.py(), extensions)
    }

    #[getter]
    fn mime_type(slf: &Bound<'_, Self>) -> PyResult<PyMimeType> {
        Ok(PyMimeType::from_core(
            urn_of(&slf.as_super().borrow())?.mime_type(),
        ))
    }

    #[getter]
    fn media_type(slf: &Bound<'_, Self>) -> PyResult<PyMediaType> {
        Ok(PyMediaType::from_core(
            urn_of(&slf.as_super().borrow())?.media_type(),
        ))
    }

    fn set_file_name(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_urn(slf, |urn| urn.set_file_name(value))
    }

    fn set_stem(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_urn(slf, |urn| urn.set_stem(value))
    }

    fn set_extension(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_urn(slf, |urn| urn.set_extension(value))
    }

    fn set_extensions(slf: &Bound<'_, Self>, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let values = strings_from_iterable(values, "extensions")?;
        edit_urn(slf, |urn| urn.set_extensions(values))
    }

    fn set_mime_type(slf: &Bound<'_, Self>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = core_mime_type_from_value(value)?;
        edit_urn(slf, |urn| urn.set_mime_type(value))
    }

    fn set_media_type(slf: &Bound<'_, Self>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = core_media_type_from_value(value)?;
        edit_urn(slf, |urn| urn.set_media_type(value))
    }

    fn remove_extension(slf: &Bound<'_, Self>) -> PyResult<bool> {
        let mut removed = false;
        edit_urn(slf, |urn| {
            removed = urn.remove_extension();
            Ok(())
        })?;
        Ok(removed)
    }

    fn clear_extensions(slf: &Bound<'_, Self>) -> PyResult<bool> {
        let mut removed = false;
        edit_urn(slf, |urn| {
            removed = urn.clear_extensions();
            Ok(())
        })?;
        Ok(removed)
    }
}

/// Apply one core edit to the name a view stands on, atomically.
fn edit_urn(
    slf: &Bound<'_, PyUrn>,
    edit: impl FnOnce(&mut CoreUrn) -> yggdryl::Result<()>,
) -> PyResult<()> {
    let base = slf.as_super();
    let mut urn = {
        let held = base.borrow();
        held.require_mutable()?;
        urn_of(&held)?
    };
    edit(&mut urn).map_err(value_error)?;
    base.borrow_mut().inner = urn.into_uri();
    Ok(())
}

// ---------------------------------------------------------------------------
// `Arn`: the name AWS writes for one of its resources.
// ---------------------------------------------------------------------------

/// An AWS ARN view with field and resource accessors.
///
/// An `Arn` is a `Uri` whose scheme spells an AWS resource name. Its five
/// fields - partition, service, region, account, resource - are what AWS
/// decides, and its filename accessors read the resource rather than the whole
/// path. `locator` answers the `s3:` URL an Amazon S3 ARN addresses.
#[pyclass(
    name = "Arn",
    module = "yggdryl._native",
    extends = PyUri,
    skip_from_py_object
)]
pub(crate) struct PyArn;

#[pymethods]
impl PyArn {
    #[new]
    fn new(value: &Bound<'_, PyAny>) -> PyResult<PyClassInitializer<Self>> {
        Ok(narrowed(core_arn_from_value(value)?.into_uri()).add_subclass(Self))
    }

    #[staticmethod]
    fn from_value(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<Self>> {
        arn_object(py, core_arn_from_value(value)?)
    }

    #[staticmethod]
    fn from_str(py: Python<'_>, value: &str) -> PyResult<Py<Self>> {
        arn_object(py, CoreArn::from_str(value).map_err(value_error)?)
    }

    /// Build a validated ARN from its five fields.
    ///
    /// `region` and `account` are written as the empty string when the service
    /// names neither, which is the value an ARN gives them rather than an
    /// argument it leaves out.
    #[staticmethod]
    fn from_parts(
        py: Python<'_>,
        partition: &str,
        service: &str,
        region: &str,
        account: &str,
        resource: &str,
    ) -> PyResult<Py<Self>> {
        let value = CoreArn::from_parts(partition, service, region, account, resource)
            .map_err(value_error)?;
        arn_object(py, value)
    }

    #[staticmethod]
    fn from_uri(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<Self>> {
        let value = CoreArn::from_uri(core_uri_from_value(value)?).map_err(value_error)?;
        arn_object(py, value)
    }

    #[staticmethod]
    fn from_json(py: Python<'_>, value: &str) -> PyResult<Py<Self>> {
        arn_object(py, CoreArn::from_json(value).map_err(value_error)?)
    }

    /// This name as the identifier it narrows.
    #[allow(clippy::wrong_self_convention)]
    fn into_uri(slf: &Bound<'_, Self>) -> PyUri {
        PyUri::from_core(slf.as_super().borrow().inner.clone())
    }

    /// The partition: `aws`, `aws-cn`, `aws-us-gov`, or another AWS names.
    #[getter]
    fn partition(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(arn_of(&slf.as_super().borrow())?.partition().to_string())
    }

    /// The service namespace: `s3`, `iam`, `lambda`, and the rest.
    #[getter]
    fn service(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(arn_of(&slf.as_super().borrow())?.service().to_string())
    }

    /// The region, or `None` for a service that spans every region.
    #[getter]
    fn region(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?
            .region()
            .map(str::to_string))
    }

    /// The owning account, or `None` when the ARN names none.
    #[getter]
    fn account(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?
            .account()
            .map(str::to_string))
    }

    /// The resource field whole, its own `/` and `:` structure kept.
    #[getter]
    fn resource(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(arn_of(&slf.as_super().borrow())?.resource().to_string())
    }

    /// The separator the resource field uses, if it carries one.
    #[getter]
    fn resource_separator(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?
            .resource_separator()
            .map(|separator| separator.to_string()))
    }

    /// The resource type, when the resource names one.
    ///
    /// This is the syntactic split at the resource's first `/` or `:`. What
    /// that leading part means is the service's own business - for Amazon S3 it
    /// is the bucket, which `bucket` is the door for.
    #[getter]
    fn resource_type(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?
            .resource_type()
            .map(str::to_string))
    }

    /// What follows the type, or the whole resource when it names no type.
    #[getter]
    fn resource_id(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(arn_of(&slf.as_super().borrow())?.resource_id().to_string())
    }

    /// The container an ARN names: the bucket on Amazon S3, the table bucket
    /// on Amazon S3 Tables.
    #[getter]
    fn bucket(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?
            .bucket()
            .map(str::to_string))
    }

    /// The object key an Amazon S3 ARN names, below its bucket.
    #[getter]
    fn key(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?.key().map(str::to_string))
    }

    /// The table an Amazon S3 Tables ARN names, below its table bucket.
    #[getter]
    fn table(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?
            .table()
            .map(str::to_string))
    }

    #[getter]
    fn file_name(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?
            .file_name()
            .map(str::to_string))
    }

    #[getter]
    fn stem(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?.stem().map(str::to_string))
    }

    #[getter]
    fn extension(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
        Ok(arn_of(&slf.as_super().borrow())?
            .extension()
            .map(str::to_string))
    }

    #[getter]
    fn extensions<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyTuple>> {
        let arn = arn_of(&slf.as_super().borrow())?;
        let extensions: Vec<&str> = arn.extensions().collect();
        PyTuple::new(slf.py(), extensions)
    }

    #[getter]
    fn mime_type(slf: &Bound<'_, Self>) -> PyResult<PyMimeType> {
        Ok(PyMimeType::from_core(
            arn_of(&slf.as_super().borrow())?.mime_type(),
        ))
    }

    #[getter]
    fn media_type(slf: &Bound<'_, Self>) -> PyResult<PyMediaType> {
        Ok(PyMediaType::from_core(
            arn_of(&slf.as_super().borrow())?.media_type(),
        ))
    }

    fn set_file_name(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_arn(slf, |arn| arn.set_file_name(value))
    }

    fn set_stem(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_arn(slf, |arn| arn.set_stem(value))
    }

    fn set_extension(slf: &Bound<'_, Self>, value: &str) -> PyResult<()> {
        edit_arn(slf, |arn| arn.set_extension(value))
    }

    fn set_extensions(slf: &Bound<'_, Self>, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let values = strings_from_iterable(values, "extensions")?;
        edit_arn(slf, |arn| arn.set_extensions(values))
    }

    fn set_mime_type(slf: &Bound<'_, Self>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = core_mime_type_from_value(value)?;
        edit_arn(slf, |arn| arn.set_mime_type(value))
    }

    fn set_media_type(slf: &Bound<'_, Self>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let value = core_media_type_from_value(value)?;
        edit_arn(slf, |arn| arn.set_media_type(value))
    }

    fn remove_extension(slf: &Bound<'_, Self>) -> PyResult<bool> {
        let mut removed = false;
        edit_arn(slf, |arn| {
            removed = arn.remove_extension();
            Ok(())
        })?;
        Ok(removed)
    }

    fn clear_extensions(slf: &Bound<'_, Self>) -> PyResult<bool> {
        let mut removed = false;
        edit_arn(slf, |arn| {
            removed = arn.clear_extensions();
            Ok(())
        })?;
        Ok(removed)
    }
}

/// Apply one core edit to the ARN a view stands on, atomically.
fn edit_arn(
    slf: &Bound<'_, PyArn>,
    edit: impl FnOnce(&mut CoreArn) -> yggdryl::Result<()>,
) -> PyResult<()> {
    let base = slf.as_super();
    let mut arn = {
        let held = base.borrow();
        held.require_mutable()?;
        arn_of(&held)?
    };
    edit(&mut arn).map_err(value_error)?;
    base.borrow_mut().inner = arn.into_uri();
    Ok(())
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

/// A URL query, addressed as the `key=value` pairs it spells.
///
/// This is a live view of the value it was taken from - `url.parameters()` -
/// so every read goes back to that URL's query and every write replaces it.
/// Item syntax means a key: `parameters["symbol"]`, `del parameters["venue"]`,
/// and the mapping methods `get`, `pop`, `setdefault`, `update` and `clear`
/// behave as a `dict`'s do - a lookup that changes nothing writes nothing.
/// `keys`, `values` and `items` answer with tuples rather than views, because
/// a query may repeat a key.
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
    /// The identifier this view reads and writes through.
    ///
    /// The view holds the Python object, not a copy of its query, so a write
    /// is visible on the value a caller already has and a read never goes
    /// stale. Every narrowed class is a `Uri`, so one owner reads all four.
    owner: Py<PyUri>,
    decode: bool,
}

impl PyParameters {
    pub(crate) const fn over_uri(owner: Py<PyUri>, decode: bool) -> Self {
        Self { owner, decode }
    }

    /// Read the pairs the owner's query holds right now.
    ///
    /// The snapshot owns its pairs: the borrow on the Python object ends with
    /// this call, which is what lets an edited snapshot be written straight
    /// back to the same object.
    fn pairs(&self, py: Python<'_>) -> PyResult<CoreParameters<'static>> {
        Ok(self
            .owner
            .bind(py)
            .try_borrow()?
            .inner
            .parameters(self.decode)
            .map_err(value_error)?
            .into_owned())
    }

    /// Replace the owner's query with these pairs, refusing a frozen value.
    fn write(&self, py: Python<'_>, pairs: &CoreParameters<'_>) -> PyResult<()> {
        edit_identifier(self.owner.bind(py), |uri| uri.set_parameters(pairs))
    }

    /// Read, edit, and write back in one step.
    fn edit<R>(
        &self,
        py: Python<'_>,
        edit: impl FnOnce(&mut CoreParameters<'static>) -> PyResult<R>,
    ) -> PyResult<R> {
        self.edit_if(py, |pairs| Ok((edit(pairs)?, true)))
    }

    /// Read and edit, writing back only when the edit changed something.
    ///
    /// A lookup that finds nothing to do is a read, and a read must not
    /// rewrite the value it read: the query would be respelled - `flag`
    /// becomes `flag=`, `a&&b` loses its empty pair - and a frozen owner would
    /// refuse an operation that changes nothing.
    fn edit_if<R>(
        &self,
        py: Python<'_>,
        edit: impl FnOnce(&mut CoreParameters<'static>) -> PyResult<(R, bool)>,
    ) -> PyResult<R> {
        let mut pairs = self.pairs(py)?;
        let (answer, changed) = edit(&mut pairs)?;
        if changed {
            self.write(py, &pairs)?;
        }
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
        Ok(PyParameterIterator::new(
            self.pairs(py)?.keys().map(str::to_owned).collect(),
        ))
    }

    /// Return every key, in the order the query spells them.
    ///
    /// The three come back as tuples rather than as one-shot iterators: a
    /// query may repeat a key, so these are sequences, and a caller reads them
    /// more than once the way a `dict` view is read more than once.
    fn keys<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let keys: Vec<String> = self.pairs(py)?.keys().map(str::to_owned).collect();
        PyTuple::new(py, keys)
    }

    /// Return every value, in order, repeated keys included.
    fn values<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let values: Vec<String> = self.pairs(py)?.values().map(str::to_owned).collect();
        PyTuple::new(py, values)
    }

    /// Return every `(key, value)` pair, in order.
    fn items<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let items: Vec<(String, String)> = self
            .pairs(py)?
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        PyTuple::new(py, items)
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
        let removed = self.edit_if(py, |pairs| {
            let removed = pairs.remove(key);
            let changed = removed.is_some();
            Ok((removed, changed))
        })?;
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
        self.edit_if(py, |pairs| {
            if let Some(value) = pairs.get(key) {
                return Ok((value.to_owned(), false));
            }
            pairs.append(key, default).map_err(value_error)?;
            Ok((default.to_owned(), true))
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
            // A mapping is whatever answers `keys`, which is how `dict.update`
            // itself tells one from a sequence of pairs.
            if let Ok(keys) = values.call_method0("keys") {
                for key in keys.try_iter()? {
                    let key = key?;
                    let value = values.get_item(&key)?;
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
        let changed = !entries.is_empty();
        self.edit_if(py, |pairs| {
            for (key, value) in &entries {
                pairs.insert(key, value).map_err(value_error)?;
            }
            Ok(((), changed))
        })
    }

    /// Drop every pair, clearing the query.
    fn clear(&self, py: Python<'_>) -> PyResult<()> {
        self.edit_if(py, |pairs| {
            let changed = !pairs.is_empty();
            pairs.clear();
            Ok(((), changed))
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
        // A query may name one key twice, which no dict can, so such a query
        // equals no dict at all - and comparing pair by pair would call two
        // pairs of the same name a match for one entry, leaving room for a
        // key the dict holds and the query does not.
        let mut names: Vec<&str> = left.keys().collect();
        names.sort_unstable();
        let repeats = names.windows(2).any(|pair| pair[0] == pair[1]);
        let equal = !repeats
            && left.len() == mapping.len()
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

/// Iterator over a query's keys, in the order the query spells them.
#[pyclass(module = "yggdryl._native")]
pub(crate) struct PyParameterIterator {
    keys: std::vec::IntoIter<String>,
}

impl PyParameterIterator {
    fn new(keys: Vec<String>) -> Self {
        Self {
            keys: keys.into_iter(),
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

    fn __next__(&mut self, py: Python<'_>) -> Option<Py<PyString>> {
        Some(PyString::new(py, &self.keys.next()?).unbind())
    }

    fn __length_hint__(&self) -> usize {
        self.keys.len()
    }
}
