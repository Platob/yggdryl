//! The `yggdryl.factory` submodule: a convenient **type-inference** factory.
//!
//! `factory.scalar(value)`, `factory.dtype(value)` and
//! `factory.field(name, value)` infer the data type from a native Python value and
//! build the matching `yggdryl.scalar` / `yggdryl.dtype` / `yggdryl.field` object,
//! so a value crosses without naming its type. The inference mirrors the model's
//! available types: `int` → `int64`, `bytes` / `bytearray` → `binary`, `None` →
//! `null`, and a homogeneous `list` of ints → an `int64` serie. A value the model
//! has no type for (a `float`, `str`, `bool`, `dict`, or a list of anything but
//! ints) raises a `ValueError` — build it through the explicit per-type factories.

// pyo3's `#[pyfunction]` expansion re-wraps the already-`PyErr` result into `PyErr`;
// clippy flags that generated conversion (on the return-type span) as useless.
#![allow(clippy::useless_conversion)]

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyByteArray, PyBytes, PyInt, PyList};

use crate::DataErr;

/// The inferred type, carrying the extracted native value (built once so `scalar`
/// keeps it while `dtype` / `field` ignore it).
enum Inferred {
    Null,
    Int64(i64),
    Binary(Vec<u8>),
    Serie(Vec<i64>),
}

/// Raises a `ValueError` naming the Python type the model cannot infer.
fn unsupported(py_type: &str) -> PyErr {
    DataErr::Message(format!(
        "cannot infer a yggdryl type from a Python {py_type}; the model has no matching type — \
         use int / bytes / None / a list of int, or an explicit per-type factory"
    ))
    .into()
}

/// Infer the data type from `value`, extracting the native value.
fn infer(value: &Bound<'_, PyAny>) -> PyResult<Inferred> {
    if value.is_none() {
        return Ok(Inferred::Null);
    }
    // A Python `bool` is an `int` subclass; reject it before the int check so it does
    // not silently become an int64.
    if value.is_instance_of::<PyBool>() {
        return Err(unsupported("bool"));
    }
    if value.is_instance_of::<PyInt>() {
        let integer = value.extract::<i64>().map_err(|_| {
            PyErr::from(DataErr::Message(
                "cannot infer a scalar: the integer is outside the int64 range; build it with the \
                 explicit uint64 / int64 factory"
                    .to_string(),
            ))
        })?;
        return Ok(Inferred::Int64(integer));
    }
    if value.is_instance_of::<PyBytes>() || value.is_instance_of::<PyByteArray>() {
        return Ok(Inferred::Binary(value.extract()?));
    }
    if value.is_instance_of::<PyList>() {
        // A homogeneous list of ints → an int64 serie (the model's only bindable
        // serie); an empty list defaults to it too.
        let values = value.extract::<Vec<i64>>().map_err(|_| {
            PyErr::from(DataErr::Message(
                "cannot infer a serie: expected a list of int64 values".to_string(),
            ))
        })?;
        return Ok(Inferred::Serie(values));
    }
    Err(unsupported(
        &value
            .get_type()
            .name()
            .map(|name| name.to_string())
            .unwrap_or_else(|_| "value".to_string()),
    ))
}

/// Infer the data type from `value` and build the matching `yggdryl.scalar`.
#[pyfunction]
fn scalar(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    Ok(match infer(value)? {
        Inferred::Null => Py::new(py, crate::scalar::NullScalar::default())?.into_any(),
        Inferred::Int64(integer) => Py::new(
            py,
            crate::scalar::Int64Scalar {
                inner: yggdryl_scalar::Int64Scalar::new(integer),
            },
        )?
        .into_any(),
        Inferred::Binary(bytes) => Py::new(
            py,
            crate::scalar::BinaryScalar {
                inner: yggdryl_scalar::BinaryScalar::new(bytes),
            },
        )?
        .into_any(),
        Inferred::Serie(values) => Py::new(
            py,
            crate::scalar::Int64Serie {
                inner: yggdryl_scalar::Int64Serie::from(values),
            },
        )?
        .into_any(),
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.dtype`.
#[pyfunction]
fn dtype(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    Ok(match infer(value)? {
        Inferred::Null => Py::new(py, crate::dtype::NullType::default())?.into_any(),
        Inferred::Int64(_) => Py::new(py, crate::dtype::Int64Type::default())?.into_any(),
        Inferred::Binary(_) => Py::new(py, crate::dtype::BinaryType::default())?.into_any(),
        Inferred::Serie(_) => Py::new(py, crate::dtype::Int64SerieType::default())?.into_any(),
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.field` named
/// `name`.
#[pyfunction]
#[pyo3(signature = (name, value, nullable = true))]
fn field(
    py: Python<'_>,
    name: String,
    value: &Bound<'_, PyAny>,
    nullable: bool,
) -> PyResult<PyObject> {
    Ok(match infer(value)? {
        Inferred::Null => Py::new(
            py,
            crate::field::NullField {
                inner: yggdryl_field::NullField::new(name, nullable),
            },
        )?
        .into_any(),
        Inferred::Int64(_) => Py::new(
            py,
            crate::field::Int64Field {
                inner: yggdryl_field::Int64Field::new(name, nullable),
            },
        )?
        .into_any(),
        Inferred::Binary(_) => Py::new(
            py,
            crate::field::BinaryField {
                inner: yggdryl_field::BinaryField::new(name, nullable),
            },
        )?
        .into_any(),
        Inferred::Serie(_) => Py::new(
            py,
            crate::field::Int64SerieField {
                inner: yggdryl_field::TypedSerieField::new(name, nullable),
            },
        )?
        .into_any(),
    })
}

/// Populates the `yggdryl.factory` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(scalar, module)?)?;
    module.add_function(wrap_pyfunction!(dtype, module)?)?;
    module.add_function(wrap_pyfunction!(field, module)?)?;
    Ok(())
}
