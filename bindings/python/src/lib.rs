//! Python extension for **yggdryl**.
//!
//! Thin PyO3 wrappers over the Arrow-centric `yggdryl_core` types; each type lives
//! in its own module mirroring the Rust crate. All logic lives in the shared core
//! so the Python and Node bindings behave identically.

// The `#[pymethods]` macro injects an `.into()` on returned errors; our fallible
// methods already return `PyErr`, so clippy flags the macro-generated conversion.
#![allow(clippy::useless_conversion)]

mod binary;
mod binary_type;
mod field;
mod string;
mod whence;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use yggdryl_core::{AnyType, DataType};

pub(crate) use binary::Binary;
pub(crate) use binary_type::BinaryType;
pub(crate) use field::Field;
pub(crate) use string::Utf8;
pub(crate) use whence::Whence;

/// Maps any core error to a Python `ValueError`.
pub(crate) fn value_err<E: std::fmt::Display>(err: E) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// Hashes a core value the same way the bindings expose it through `__hash__`.
pub(crate) fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Renders a Rust `bool` as a Python `repr` (`True` / `False`).
pub(crate) fn py_bool(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

/// Wraps a core [`AnyType`] in the matching Python data-type object.
pub(crate) fn anytype_to_py(py: Python<'_>, ty: &AnyType) -> PyResult<PyObject> {
    Ok(match ty {
        AnyType::Binary(inner) => Py::new(py, BinaryType { inner: *inner })?.into_any(),
        AnyType::Utf8(inner) => Py::new(py, Utf8 { inner: *inner })?.into_any(),
    })
}

/// Extracts a core [`AnyType`] from a Python data-type object (`BinaryType`/`Utf8`).
pub(crate) fn py_to_anytype(obj: &Bound<'_, PyAny>) -> PyResult<AnyType> {
    if let Ok(binary) = obj.extract::<BinaryType>() {
        return Ok(binary.inner.to_any());
    }
    if let Ok(utf8) = obj.extract::<Utf8>() {
        return Ok(utf8.inner.to_any());
    }
    Err(PyValueError::new_err(
        "expected a yggdryl data type (BinaryType or Utf8)",
    ))
}

/// The compiled `yggdryl` extension module.
#[pymodule]
fn yggdryl(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<BinaryType>()?;
    module.add_class::<Utf8>()?;
    module.add_class::<Field>()?;
    module.add_class::<Binary>()?;
    module.add_class::<Whence>()?;
    Ok(())
}
