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
mod charset;
mod field;
mod jsonparams;
mod utf8;
mod utf8_type;
mod whence;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::wrap_pyfunction;
use yggdryl_core::{AnyScalar, AnyType, DataType};

pub(crate) use binary::Binary;
pub(crate) use binary_type::BinaryType;
pub(crate) use charset::Charset;
pub(crate) use field::Field;
pub(crate) use jsonparams::JsonParams;
pub(crate) use utf8::Utf8;
pub(crate) use utf8_type::Utf8Type;
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
        AnyType::Utf8(inner) => Py::new(py, Utf8Type { inner: *inner })?.into_any(),
    })
}

/// Extracts a core [`AnyType`] from a Python data-type object.
pub(crate) fn py_to_anytype(obj: &Bound<'_, PyAny>) -> PyResult<AnyType> {
    if let Ok(binary) = obj.extract::<BinaryType>() {
        return Ok(binary.inner.to_any());
    }
    if let Ok(utf8) = obj.extract::<Utf8Type>() {
        return Ok(utf8.inner.to_any());
    }
    Err(PyValueError::new_err(
        "expected a yggdryl data type (BinaryType or Utf8Type)",
    ))
}

/// Wraps a core [`AnyScalar`] in the matching Python scalar value object.
pub(crate) fn anyscalar_to_py(py: Python<'_>, scalar: AnyScalar) -> PyResult<PyObject> {
    Ok(match scalar {
        AnyScalar::Binary(inner) => Py::new(py, Binary { inner })?.into_any(),
        AnyScalar::Utf8(inner) => Py::new(py, Utf8 { inner })?.into_any(),
    })
}

/// The compiled `yggdryl` extension module.
#[pymodule]
fn yggdryl(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<BinaryType>()?;
    module.add_class::<Utf8Type>()?;
    module.add_class::<Field>()?;
    module.add_class::<Binary>()?;
    module.add_class::<Utf8>()?;
    module.add_class::<Whence>()?;
    module.add_class::<Charset>()?;
    module.add_class::<JsonParams>()?;
    module.add_function(wrap_pyfunction!(jsonparams::set_json_params, module)?)?;
    module.add_function(wrap_pyfunction!(jsonparams::json_params, module)?)?;
    module.add_function(wrap_pyfunction!(jsonparams::reset_json_params, module)?)?;
    Ok(())
}
