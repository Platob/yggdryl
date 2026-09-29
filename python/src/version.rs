//! Native Version value: a sixteen-bit major and minor and a text patch.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::{PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyInt, PyString};
use yggdryl::{Scalar, Version};

use crate::{compare, python_hash, value_error};

#[pyclass(
    name = "Version",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyVersion {
    pub(crate) inner: Version,
}

#[pymethods]
impl PyVersion {
    #[new]
    #[pyo3(signature = (major, minor=0, patch=None))]
    fn new(major: u16, minor: u16, patch: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let inner = match patch {
            None => Version::new(major, minor, None),
            Some(patch) => {
                if let Ok(text) = patch.cast::<PyString>() {
                    Version::new(major, minor, Some(text.to_str()?))
                } else if let Ok(number) = patch.cast::<PyInt>() {
                    Version::new(major, minor, Some(&decimal_digits(number)?))
                } else {
                    return Err(PyTypeError::new_err(format!(
                        "patch must be an int, a str or None, not {}",
                        patch.get_type().name()?
                    )));
                }
            }
        };
        Ok(Self { inner })
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        value
            .parse()
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    #[getter]
    fn major(&self) -> u16 {
        self.inner.major()
    }
    #[getter]
    fn minor(&self) -> u16 {
        self.inner.minor()
    }
    #[getter]
    fn patch(&self) -> Option<&str> {
        self.inner.patch()
    }
    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let (major, minor) = (self.major(), self.minor());
        Ok(match self.patch() {
            None => format!("Version({major}, {minor})"),
            Some(patch) => format!(
                "Version({major}, {minor}, {})",
                PyString::new(py, patch).repr()?
            ),
        })
    }
    fn stable_hash(&self) -> u64 {
        Scalar::Version(self.inner.clone()).stable_hash()
    }
    fn __hash__(&self) -> isize {
        python_hash(self.stable_hash())
    }
    fn __richcmp__(&self, other: &Self, operation: CompareOp) -> bool {
        compare(self.inner.cmp(&other.inner), operation)
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (u16, u16, Option<&str>)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.major(), self.minor(), self.patch()),
        )
    }
}

/// The decimal digits of a non-negative Python integer.
///
/// `int.__repr__` rather than `str()` for what `u64` cannot hold, because a
/// subclass may render itself otherwise; past the interpreter's
/// `sys.get_int_max_str_digits()` it refuses, and the refusal names the patch.
fn decimal_digits(number: &Bound<'_, PyInt>) -> PyResult<String> {
    if let Ok(value) = number.extract::<u64>() {
        return Ok(value.to_string());
    }
    if number.lt(0)? {
        return Err(PyOverflowError::new_err(
            "patch must be a non-negative integer",
        ));
    }
    number
        .py()
        .get_type::<PyInt>()
        .call_method1("__repr__", (number,))
        .map_err(|error| {
            PyValueError::new_err(format!(
                "patch has more digits than this interpreter renders: {error}"
            ))
        })?
        .extract()
}
