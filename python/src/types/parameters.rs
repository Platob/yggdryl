//! What a string or a byte column declares, as frozen Python values.
//!
//! `StringParameters` is one of the eighteen string leaves and
//! `BytesParameters` one of the six byte leaves, each with the number the
//! leaf carries. Both are what `DataType.string_parameters` and
//! `DataType.bytes_parameters` answer, and what `DataType.string(...)` and
//! `DataType.bytes(...)` read their arguments into. The core owns every
//! rule (which spellings name a leaf, which spellings take a charset, that a
//! leaf that is a number needs one, that a bound is at least one byte), so
//! a value here is one the core already accepted.

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use yggdryl::DataType as CoreDataType;
use yggdryl::{BytesType, StringType};

use crate::value_error;

/// Read a string leaf out of Python's spelling of it.
///
/// `layout` is any spelling of a leaf; `charset` is accepted only beside a
/// charset-free spelling (`string`, `fixed_string`, `large_string`, ...) and
/// restates the leaf in that charset's family; `bound` is the number the
/// leaf carries. All three rules live on [`StringType::from_declaration`],
/// so Python decides none of them.
pub(crate) fn core_string_parameters(
    layout: &str,
    charset: Option<&str>,
    bound: Option<u32>,
) -> PyResult<StringType> {
    StringType::from_declaration(layout, charset, bound).map_err(value_error)
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

/// What a string column declares: one of the eighteen leaves and, where the
/// leaf carries one, its number.
///
/// Six leaves per charset (plain, large, view, large view, fixed and sized)
/// in UTF-8, US-ASCII and windows-1252. The three fixed leaves carry an
/// exact width and the three sized leaves a maximum; ``fixed`` and ``max``
/// read the number under whichever name the leaf gives it.
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
    /// Declare a string: a leaf, and where the spelling leaves the charset
    /// open, the charset its bytes are written in.
    ///
    /// ``layout`` takes any spelling of a leaf - its canonical name
    /// (``utf8``, ``sized_cp1252``, ...) or a charset-free one (``string``,
    /// ``fixed_string``, ``large_string_view``, ``varchar``, ...). Only a
    /// charset-free spelling takes a ``charset``; ``bound`` is the number
    /// on a fixed or sized leaf, and ``string`` with a bound is the sized
    /// leaf written short.
    #[new]
    #[pyo3(signature = (layout="string", charset=None, bound=None))]
    fn new(layout: &str, charset: Option<&str>, bound: Option<u32>) -> PyResult<Self> {
        core_string_parameters(layout, charset, bound).map(Self::from_inner)
    }

    /// The leaf's canonical name, one of the eighteen.
    #[getter]
    fn layout(&self) -> &'static str {
        self.inner.as_str()
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
        // The leaf's name already says the charset, so the repr never
        // restates it: a charset beside a canonical name is refused.
        match self.inner.bound() {
            Some(bound) => format!("StringParameters({:?}, bound={bound})", self.layout()),
            None => format!("StringParameters({:?})", self.layout()),
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
                    py.None().into_bound(py).into_any(),
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
