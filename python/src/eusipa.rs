//! The product category of a structured product: one four-digit code of
//! EUSIPA's European Derivative Map, which the SSPA's Swiss Derivative Map
//! numbers the same way - an `int` read by the core's [`Eusipa::new`], a
//! `str` by [`Eusipa::from_text`], held by its shape alone.

use pyo3::class::basic::CompareOp;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyInt, PyString};
use yggdryl_market::Eusipa;

use crate::{compare, python_hash, value_error};

/// One EUSIPA product category, held by its shape - four digits opening
/// with `1`, an investment product, or `2`, a leverage product - whichever
/// map lists it.
#[pyclass(
    name = "Eusipa",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyEusipa {
    inner: Eusipa,
}

#[pymethods]
impl PyEusipa {
    /// `code` an `int` - a negative or too-wide one an `OverflowError`, any
    /// other outside `1000` to `2999` a `ValueError` - or a `str` of four
    /// digits once trimmed, read by the core's own reader; anything else,
    /// a `bool` included, a `TypeError`.
    #[new]
    fn new(code: &Bound<'_, PyAny>) -> PyResult<Self> {
        let inner = if let Ok(text) = code.cast::<PyString>() {
            Eusipa::from_text(text.to_str()?)
        } else if code.is_instance_of::<PyInt>() && !code.is_instance_of::<PyBool>() {
            Eusipa::new(code.extract()?)
        } else {
            return Err(PyTypeError::new_err(format!(
                "code must be an int or a str, not {}",
                code.get_type().name()?
            )));
        };
        inner.map(|inner| Self { inner }).map_err(value_error)
    }

    /// The four-digit code.
    #[getter]
    fn code(&self) -> u16 {
        self.inner.code()
    }
    /// The first two digits: `12` yield enhancement, `23` constant
    /// leverage.
    #[getter]
    fn group(&self) -> u8 {
        self.inner.group()
    }
    /// The first digit: `1` an investment product, `2` a leverage product.
    #[getter]
    fn level(&self) -> u8 {
        self.inner.level()
    }
    /// The English name EUSIPA's European Derivative Map of February 2024
    /// gives the code; `None` where that map lists no such member.
    #[getter]
    fn name(&self) -> Option<&'static str> {
        self.inner.name()
    }
    /// The English name the SSPA's Swiss Derivative Map - 2023 and 2026,
    /// one list - gives the code; `None` where that map lists no such
    /// member.
    #[getter]
    fn sspa_name(&self) -> Option<&'static str> {
        self.inner.sspa_name()
    }
    /// Whether either map lists the code.
    #[getter]
    fn is_listed(&self) -> bool {
        self.inner.is_listed()
    }

    fn __int__(&self) -> u16 {
        self.inner.code()
    }
    fn __str__(&self) -> String {
        self.inner.to_string()
    }
    fn __repr__(&self) -> String {
        format!("Eusipa({})", self.inner)
    }
    /// The code's own hash: the category is its code.
    fn __hash__(&self) -> isize {
        python_hash(u64::from(self.inner.code()))
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
    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (u16,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.code(),),
        )
    }
}
