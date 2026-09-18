//! What a string or a byte column declares, as frozen Python values.
//!
//! `StringParameters` is a layout beside a charset and a bound;
//! `BytesParameters` is a layout beside a bound. Both are what
//! `DataType.string_parameters` and `DataType.bytes_parameters` answer, and
//! what `DataType.string(...)` and `DataType.bytes(...)` read their arguments
//! into. The core owns every rule - which spellings name a layout, that a
//! fixed layout needs a width, that a bound is at least one byte - so a value
//! here is one the core already accepted.

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use yggdryl::types::{BytesType, StringLayout, StringType};
use yggdryl::{Charset, DataType as CoreDataType};

use crate::value_error;

/// Read string parameters out of Python's spelling of them.
pub(crate) fn core_string_parameters(
    layout: &str,
    charset: &str,
    bound: Option<u32>,
) -> PyResult<StringType> {
    let layout = StringLayout::from_str(layout).map_err(value_error)?;
    let charset = Charset::from_str(charset).map_err(value_error)?;
    let mut parameters = StringType::new(layout, charset);
    if let Some(bound) = bound {
        parameters = parameters.try_with_bound(bound).map_err(value_error)?;
    }
    parameters.validate().map_err(value_error)?;
    Ok(parameters)
}

/// Read a byte leaf out of Python's spelling of it.
///
/// The leaf is the whole declaration now, so a bound is not a second thing
/// beside it: a number restates the leaf as the one that carries it, and a
/// leaf that *is* a number stands with none. Both rules live on
/// [`BytesType::with_declared_bound`], so Python decides neither.
pub(crate) fn core_bytes_parameters(layout: &str, bound: Option<u32>) -> PyResult<BytesType> {
    BytesType::from_str(layout)
        .and_then(|leaf| leaf.with_declared_bound(bound))
        .map_err(value_error)
}

/// What a string column declares: its layout, its charset, and its bound.
///
/// The bound is one number with one reading per layout: the exact width on
/// the fixed layout, the maximum everywhere else. ``fixed`` and ``max`` read
/// it under whichever name the layout gives it.
#[pyclass(
    name = "StringParameters",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyStringParameters {
    inner: StringType,
}

impl PyStringParameters {
    pub(crate) const fn from_inner(inner: StringType) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyStringParameters {
    /// Declare a string: a layout, the charset its bytes are written in, and
    /// the bound its values are held to.
    ///
    /// ``layout`` takes any of a layout's three spellings (``string``,
    /// ``utf8``, ``ascii``; ``large_string``, ``large_utf8``, ...) and
    /// ``charset`` any documented charset alias.
    #[new]
    #[pyo3(signature = (layout="string", charset="utf-8", bound=None))]
    fn new(layout: &str, charset: &str, bound: Option<u32>) -> PyResult<Self> {
        core_string_parameters(layout, charset, bound).map(Self::from_inner)
    }

    /// The layout's general name: ``string``, ``fixed_string``, ``string_view``,
    /// ``large_string``, or ``large_string_view``.
    #[getter]
    fn layout(&self) -> &'static str {
        self.inner.layout().as_str()
    }

    /// The canonical name of the charset the stored bytes are written in.
    #[getter]
    fn charset(&self) -> &'static str {
        self.inner.charset().as_str()
    }

    /// The declared byte bound, whichever reading the layout gives it.
    #[getter]
    fn bound(&self) -> Option<u32> {
        self.inner.bound()
    }

    /// The exact bytes every value fills, on the fixed layout.
    #[getter]
    fn fixed(&self) -> Option<u32> {
        self.inner.fixed()
    }

    /// The most bytes a value may hold, on a variable layout.
    #[getter]
    fn max(&self) -> Option<u32> {
        self.inner.max()
    }

    /// A deterministic cross-language hash of the canonical spelling.
    fn stable_hash(&self) -> u64 {
        CoreDataType::String(self.inner).stable_hash()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        match self.inner.bound() {
            Some(bound) => format!(
                "StringParameters({:?}, {:?}, {bound})",
                self.layout(),
                self.charset()
            ),
            None => format!(
                "StringParameters({:?}, {:?})",
                self.layout(),
                self.charset()
            ),
        }
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(Py<PyAny>, Bound<'py, PyTuple>)> {
        Ok((
            py.get_type::<Self>().into_any().unbind(),
            PyTuple::new(
                py,
                [
                    self.layout().into_pyobject(py)?.into_any(),
                    self.charset().into_pyobject(py)?.into_any(),
                    self.inner.bound().into_pyobject(py)?.into_any(),
                ],
            )?,
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// What a byte column declares: its layout and its bound.
///
/// The bound is one number with one reading per layout: the exact width on
/// ``fixed_size_binary``, the maximum everywhere else.
#[pyclass(
    name = "BytesParameters",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyBytesParameters {
    inner: BytesType,
}

impl PyBytesParameters {
    pub(crate) const fn from_inner(inner: BytesType) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyBytesParameters {
    /// Declare a byte column: a leaf and, where the leaf carries one, its
    /// number.
    ///
    /// ``layout`` is one of the six leaves - ``binary``, ``large_binary``,
    /// ``binary_view``, ``large_binary_view``, ``fixed_binary`` or
    /// ``sized_binary`` - and only the last two take a ``bound``.
    #[new]
    #[pyo3(signature = (layout="binary", bound=None))]
    fn new(layout: &str, bound: Option<u32>) -> PyResult<Self> {
        core_bytes_parameters(layout, bound).map(Self::from_inner)
    }

    /// The leaf's name, one of the six.
    #[getter]
    fn layout(&self) -> &'static str {
        self.inner.as_str()
    }

    /// The declared byte bound, whichever reading the layout gives it.
    #[getter]
    fn bound(&self) -> Option<u32> {
        self.inner.bound()
    }

    /// The exact bytes every value fills, on the fixed layout.
    #[getter]
    fn fixed(&self) -> Option<u32> {
        self.inner.fixed()
    }

    /// The most bytes a value may hold, on a variable layout.
    #[getter]
    fn max(&self) -> Option<u32> {
        self.inner.max()
    }

    /// A deterministic cross-language hash of the canonical spelling.
    fn stable_hash(&self) -> u64 {
        CoreDataType::Bytes(self.inner).stable_hash()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        match self.inner.bound() {
            Some(bound) => format!("BytesParameters({:?}, {bound})", self.layout()),
            None => format!("BytesParameters({:?})", self.layout()),
        }
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.stable_hash())
    }

    fn __richcmp__(&self, other: &Bound<'_, PyAny>, operation: CompareOp) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(Py<PyAny>, Bound<'py, PyTuple>)> {
        Ok((
            py.get_type::<Self>().into_any().unbind(),
            PyTuple::new(
                py,
                [
                    self.layout().into_pyobject(py)?.into_any(),
                    self.inner.bound().into_pyobject(py)?.into_any(),
                ],
            )?,
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}
