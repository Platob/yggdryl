//! Native numeric Version value.

use pyo3::class::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::types::PyAny;
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
    #[pyo3(signature = (major, minor=0, patch=0))]
    fn new(major: u8, minor: u8, patch: u16) -> Self {
        Self {
            inner: Version::new(major, minor, patch),
        }
    }

    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        value
            .parse()
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    #[getter]
    fn major(&self) -> u8 {
        self.inner.major()
    }
    #[getter]
    fn minor(&self) -> u8 {
        self.inner.minor()
    }
    #[getter]
    fn patch(&self) -> u16 {
        self.inner.patch()
    }
    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self) -> String {
        format!(
            "Version({}, {}, {})",
            self.major(),
            self.minor(),
            self.patch()
        )
    }
    fn stable_hash(&self) -> u64 {
        Scalar::Version(self.inner).stable_hash()
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
    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (u8, u8, u16)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.major(), self.minor(), self.patch()),
        )
    }
}
