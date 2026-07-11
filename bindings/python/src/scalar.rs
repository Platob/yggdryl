//! The `yggdryl.scalar` submodule — Arrow primitive scalars.
//!
//! Exposes one class per primitive scalar (`I8Scalar` … `F64Scalar`,
//! `BooleanScalar`), mirroring `yggdryl_scalar`. A scalar wraps a single value or is
//! null; each carries `value`, `is_null`, its `data_type` (a
//! [`yggdryl.dtype`](super::dtype) class), the byte codec, value semantics, and `repr`.

#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl_scalar::{Scalar, TypedScalar};

/// Maps a [`yggdryl_scalar::ScalarError`] to a Python `ValueError`.
fn scalar_err(error: yggdryl_scalar::ScalarError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Generates the pyo3 wrapper class for one primitive scalar. `$dtype` is the matching
/// [`yggdryl.dtype`](super::dtype) class the scalar's `data_type` returns; `$native` is
/// its Python-visible value type.
macro_rules! py_primitive_scalar {
    ($( ($scalar:ident, $dtype:ident, $native:ty, $lit:literal) ),+ $(,)?) => {
        $(
            #[doc = concat!("A single, possibly-null `", $lit, "` value.")]
            #[pyclass(module = "yggdryl.scalar")]
            #[derive(Clone)]
            pub struct $scalar {
                pub(crate) inner: yggdryl_scalar::$scalar,
            }

            #[pymethods]
            impl $scalar {
                #[new]
                #[pyo3(signature = (value = None))]
                fn new(value: Option<$native>) -> Self {
                    let inner = match value {
                        Some(value) => yggdryl_scalar::$scalar::new(value),
                        None => yggdryl_scalar::$scalar::null(),
                    };
                    Self { inner }
                }

                /// A null scalar of this type.
                #[staticmethod]
                fn null() -> Self {
                    Self { inner: yggdryl_scalar::$scalar::null() }
                }

                /// The scalar's value, or `None` when null.
                #[getter]
                fn value(&self) -> Option<$native> {
                    TypedScalar::value(&self.inner)
                }

                /// Whether the scalar holds no value.
                #[getter]
                fn is_null(&self) -> bool {
                    self.inner.is_null()
                }

                /// The scalar's data type (a `yggdryl.dtype` class).
                #[getter]
                fn data_type(&self) -> crate::dtype::$dtype {
                    crate::dtype::$dtype { inner: TypedScalar::data_type(&self.inner) }
                }

                /// The scalar serialised to bytes (a null flag + the value's bytes).
                fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, &self.inner.serialize_bytes())
                }

                /// Reconstructs the scalar from its serialised bytes.
                #[staticmethod]
                fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                    yggdryl_scalar::$scalar::deserialize_bytes(bytes)
                        .map(|inner| Self { inner })
                        .map_err(scalar_err)
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
                        .get_type_bound::<$scalar>()
                        .getattr("deserialize_bytes")?
                        .unbind();
                    let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                        .into_any()
                        .unbind();
                    Ok((ctor, (state,)))
                }

                fn __repr__(&self) -> String {
                    match TypedScalar::value(&self.inner) {
                        Some(value) => format!("{}({})", stringify!($scalar), value),
                        None => format!("{}(null)", stringify!($scalar)),
                    }
                }
            }
        )+
    };
}

py_primitive_scalar! {
    (I8Scalar, I8Type, i8, "int8"),
    (I16Scalar, I16Type, i16, "int16"),
    (I32Scalar, I32Type, i32, "int32"),
    (I64Scalar, I64Type, i64, "int64"),
    (U8Scalar, U8Type, u8, "uint8"),
    (U16Scalar, U16Type, u16, "uint16"),
    (U32Scalar, U32Type, u32, "uint32"),
    (U64Scalar, U64Type, u64, "uint64"),
    (F32Scalar, F32Type, f32, "float32"),
    (F64Scalar, F64Type, f64, "float64"),
    (BooleanScalar, BooleanType, bool, "boolean"),
}

/// Populates the `scalar` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<I8Scalar>()?;
    module.add_class::<I16Scalar>()?;
    module.add_class::<I32Scalar>()?;
    module.add_class::<I64Scalar>()?;
    module.add_class::<U8Scalar>()?;
    module.add_class::<U16Scalar>()?;
    module.add_class::<U32Scalar>()?;
    module.add_class::<U64Scalar>()?;
    module.add_class::<F32Scalar>()?;
    module.add_class::<F64Scalar>()?;
    module.add_class::<BooleanScalar>()?;
    Ok(())
}
