//! The `yggdryl.field` submodule — Arrow primitive fields.
//!
//! Exposes one class per primitive field (`I8Field` … `F64Field`, `BooleanField`) plus
//! the sui-generis `NullField`, mirroring `yggdryl_field`. Each carries `name`,
//! `nullable`, its `data_type` (a
//! [`yggdryl.dtype`](super::dtype) class), the byte codec, value semantics, and `repr`.
//! The Arrow `to_arrow` / `from_arrow` interop is **Rust-only** (an `arrow_schema` value
//! does not cross the FFI boundary), exactly as for the dtype layer.

#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use std::collections::HashMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use yggdryl_field::{Field, TypedField};
use yggdryl_http::{Headers, HeadersBased};

/// Maps a [`yggdryl_field::FieldError`] to a Python `ValueError`.
fn field_err(error: yggdryl_field::FieldError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Builds a Python `dict[bytes, bytes]` from a field's headers (or `None`).
fn headers_to_dict<'py>(py: Python<'py>, headers: Option<&Headers>) -> Option<Bound<'py, PyDict>> {
    headers.map(|meta| {
        let dict = PyDict::new_bound(py);
        for (key, value) in meta.pairs() {
            dict.set_item(PyBytes::new_bound(py, key), PyBytes::new_bound(py, value))
                .expect("inserting into a fresh dict cannot fail");
        }
        dict
    })
}

/// Generates the pyo3 wrapper class for one primitive field. `$dtype` is the matching
/// [`yggdryl.dtype`](super::dtype) class the field's `data_type` returns.
macro_rules! py_primitive_field {
    ($( ($field:ident, $dtype:ident, $lit:literal) ),+ $(,)?) => {
        $(
            #[doc = concat!("A named, nullable `", $lit, "` field.")]
            #[pyclass(module = "yggdryl.field")]
            #[derive(Clone)]
            pub struct $field {
                pub(crate) inner: yggdryl_field::$field,
            }

            #[pymethods]
            impl $field {
                #[new]
                #[pyo3(signature = (name, nullable = false))]
                fn new(name: String, nullable: bool) -> Self {
                    Self { inner: yggdryl_field::$field::new(name, nullable) }
                }

                /// The field's name.
                #[getter]
                fn name(&self) -> String {
                    self.inner.name().to_string()
                }

                /// Whether the field's values may be null.
                #[getter]
                fn nullable(&self) -> bool {
                    self.inner.is_nullable()
                }

                /// The field's data type (a `yggdryl.dtype` class).
                #[getter]
                fn data_type(&self) -> crate::dtype::$dtype {
                    crate::dtype::$dtype { inner: TypedField::data_type(&self.inner) }
                }

                /// The field's headers as a `dict[bytes, bytes]`, or `None`.
                #[getter]
                fn headers<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyDict>> {
                    headers_to_dict(py, self.inner.headers())
                }

                /// Returns a copy of this field with `headers` (a `dict[bytes, bytes]`)
                /// attached.
                fn with_headers(&self, headers: HashMap<Vec<u8>, Vec<u8>>) -> Self {
                    Self {
                        inner: self.inner.clone().with_headers(Headers::from_pairs(headers)),
                    }
                }

                /// The field serialised to bytes (a nullable flag + the UTF-8 name).
                fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, &self.inner.serialize_bytes())
                }

                /// Reconstructs the field from its serialised bytes.
                #[staticmethod]
                fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                    yggdryl_field::$field::deserialize_bytes(bytes)
                        .map(|inner| Self { inner })
                        .map_err(field_err)
                }

                fn __eq__(&self, other: &Self) -> bool {
                    self.inner == other.inner
                }

                fn __hash__(&self) -> u64 {
                    let mut hasher = DefaultHasher::new();
                    self.inner.hash(&mut hasher);
                    hasher.finish()
                }

                fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
                    let ctor = py
                        .get_type_bound::<$field>()
                        .getattr("deserialize_bytes")?
                        .unbind();
                    let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                        .into_any()
                        .unbind();
                    Ok((ctor, (state,)))
                }

                fn __repr__(&self) -> String {
                    format!(
                        concat!(stringify!($field), "(name={:?}, nullable={})"),
                        self.inner.name(),
                        self.inner.is_nullable()
                    )
                }
            }
        )+
    };
}

py_primitive_field! {
    (I8Field, I8Type, "int8"),
    (I16Field, I16Type, "int16"),
    (I32Field, I32Type, "int32"),
    (I64Field, I64Type, "int64"),
    (U8Field, U8Type, "uint8"),
    (U16Field, U16Type, "uint16"),
    (U32Field, U32Type, "uint32"),
    (U64Field, U64Type, "uint64"),
    (F32Field, F32Type, "float32"),
    (F64Field, F64Type, "float64"),
    (BooleanField, BooleanType, "boolean"),
    (NullField, NullType, "null"),
}

/// Populates the `field` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<I8Field>()?;
    module.add_class::<I16Field>()?;
    module.add_class::<I32Field>()?;
    module.add_class::<I64Field>()?;
    module.add_class::<U8Field>()?;
    module.add_class::<U16Field>()?;
    module.add_class::<U32Field>()?;
    module.add_class::<U64Field>()?;
    module.add_class::<F32Field>()?;
    module.add_class::<F64Field>()?;
    module.add_class::<BooleanField>()?;
    module.add_class::<NullField>()?;
    Ok(())
}
