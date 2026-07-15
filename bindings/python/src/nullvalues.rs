//! The `yggdryl.types` submodule's **null** value layer — Arrow's `Null`: a type whose every
//! value is null, at zero storage. `NullScalar` is the (only) null value; `NullSerie` is a run of
//! nulls stored as just its length. Mirrors `yggdryl_core::io::fixed`'s `NullScalar` / `NullSerie`.
//!
//! Every `NullScalar` is equal (and hashes the same); two `NullSerie`s are equal iff they have the
//! same length. A `NullScalar` is immutable (hashable); a `NullSerie` grows via `push` / `extend`,
//! so — like `bytearray` — it is not hashable.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList, PyTuple};

use yggdryl_core::io::fixed::{
    NullField, NullScalar as CoreNullScalar, NullSerie as CoreNullSerie,
};
use yggdryl_core::io::{DataTypeId, IoError};

use crate::types::{DataType, Field};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn io_err(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// One **null** value — the null type's only inhabitant.
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct NullScalar {
    pub(crate) inner: CoreNullScalar,
}

#[pymethods]
impl NullScalar {
    /// The null value.
    #[new]
    fn new() -> Self {
        Self {
            inner: CoreNullScalar::null(),
        }
    }

    /// The null value (the cross-family name).
    #[staticmethod]
    fn null() -> Self {
        Self::new()
    }

    /// Always `True` — the null type has only the null value.
    #[getter]
    fn is_null(&self) -> bool {
        true
    }

    /// Always `False`.
    fn is_valid(&self) -> bool {
        false
    }

    /// The value, always `None`.
    #[getter]
    fn value(&self) -> Option<PyObject> {
        None
    }

    /// The type name, `"null"`.
    #[getter]
    fn type_name(&self) -> &'static str {
        DataTypeId::Null.name()
    }

    /// This scalar's [`DataType`] (`null`, byte width `0`).
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Null)
    }

    /// A [`Field`] naming a null column (a null column is always nullable).
    fn field(&self, name: &str) -> Field {
        Field {
            inner: NullField::new(name).erase(),
        }
    }

    /// This scalar broadcast to a length-1 [`NullSerie`].
    fn to_serie(&self) -> NullSerie {
        NullSerie {
            inner: self.inner.to_serie(),
        }
    }

    /// The scalar's canonical bytes — empty (a null value carries nothing).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs the null value (any input; there is only one value).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> Self {
        Self {
            inner: CoreNullScalar::deserialize_bytes(bytes),
        }
    }

    fn __eq__(&self, _other: &Self) -> bool {
        true // every null scalar is equal
    }

    fn __hash__(&self) -> u64 {
        0
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through the constructor (there is one null value, so no args).
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, Py<PyAny>)> {
        let ctor = py.get_type_bound::<NullScalar>().into_any().unbind();
        let args = PyTuple::empty_bound(py).into_any().unbind();
        Ok((ctor, args))
    }

    fn __repr__(&self) -> String {
        "NullScalar()".to_string()
    }
}

/// A **null column** — a run of `length` nulls, stored as just the length.
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct NullSerie {
    pub(crate) inner: CoreNullSerie,
}

#[pymethods]
impl NullSerie {
    /// A null column of `length` nulls (empty by default).
    #[new]
    #[pyo3(signature = (length = 0))]
    fn new(length: usize) -> Self {
        Self {
            inner: CoreNullSerie::with_len(length),
        }
    }

    /// Appends one null, growing the column by one.
    fn push(&mut self) {
        self.inner.push();
    }

    /// Grows the column by `count` nulls.
    fn extend(&mut self, count: usize) {
        self.inner.extend(count);
    }

    /// The number of null elements — always [`len`](NullSerie::__len__).
    #[getter]
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// Whether the column carries any nulls — `True` unless empty.
    #[getter]
    fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column is empty.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Element `index` as a [`NullScalar`] (always null); raises `IndexError` out of range.
    fn get_scalar(&self, index: usize) -> PyResult<NullScalar> {
        if index >= self.inner.len() {
            return Err(PyIndexError::new_err("Serie index out of range"));
        }
        Ok(NullScalar {
            inner: self.inner.get_scalar(index),
        })
    }

    /// This column's [`DataType`] (`null`).
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Null)
    }

    /// A [`Field`] naming this null column.
    fn to_field(&self, name: &str) -> Field {
        Field {
            inner: self.inner.to_field(name).erase(),
        }
    }

    /// The column's canonical bytes — its length as a little-endian `u64`.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a column from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        CoreNullSerie::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    /// Random access — `col[i]` is always `None` (negative indices allowed); raises `IndexError`
    /// out of range.
    fn __getitem__(&self, index: isize) -> PyResult<Option<PyObject>> {
        let len = self.inner.len() as isize;
        let resolved = if index < 0 { index + len } else { index };
        if resolved < 0 || resolved >= len {
            return Err(PyIndexError::new_err("Serie index out of range"));
        }
        Ok(None)
    }

    /// Iterates the elements — all `None`.
    fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let nones: Vec<Option<PyObject>> = (0..self.inner.len()).map(|_| None).collect();
        Ok(PyList::new_bound(py, nones)
            .call_method0("__iter__")?
            .unbind())
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<NullSerie>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!("NullSerie(len={})", self.inner.len())
    }
}

/// Adds the null `Scalar` / `Serie` classes to the `yggdryl.types` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<NullScalar>()?;
    module.add_class::<NullSerie>()?;
    Ok(())
}
