//! The `yggdryl.scalar` submodule — Arrow primitive scalars.
//!
//! Exposes one class per primitive scalar (`I8Scalar` … `F64Scalar`,
//! `BooleanScalar`) plus the sui-generis `NullScalar` (whose value is always `None`),
//! mirroring `yggdryl_scalar`. A scalar wraps a single, always-present
//! value (nullability is modelled separately — a `NullType` value and, later, union
//! types); each carries `value`, its `data_type` (a [`yggdryl.dtype`](super::dtype)
//! class), the byte codec, value semantics, and `repr`. The 64-bit `I64Scalar` /
//! `U64Scalar` are specialised (out of the primitive macro) so an out-of-range `int`
//! raises a guided `ValueError` matching the Node binding, not an opaque `OverflowError`.

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
            #[doc = concat!("A single `", $lit, "` value (always present).")]
            #[pyclass(module = "yggdryl.scalar")]
            #[derive(Clone)]
            pub struct $scalar {
                pub(crate) inner: yggdryl_scalar::$scalar,
            }

            #[pymethods]
            impl $scalar {
                #[new]
                fn new(value: $native) -> Self {
                    Self { inner: yggdryl_scalar::$scalar::new(value) }
                }

                /// The default scalar of this type (its data type's default value).
                #[staticmethod]
                fn default_scalar() -> Self {
                    Self { inner: yggdryl_scalar::$scalar::default_scalar() }
                }

                /// The scalar's value (always present).
                #[getter]
                fn value(&self) -> $native {
                    TypedScalar::value(&self.inner)
                }

                /// The scalar's data type (a `yggdryl.dtype` class).
                #[getter]
                fn data_type(&self) -> crate::dtype::$dtype {
                    crate::dtype::$dtype { inner: TypedScalar::data_type(&self.inner) }
                }

                /// The scalar serialised to its value's little-endian bytes.
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
                    format!("{}({})", stringify!($scalar), TypedScalar::value(&self.inner))
                }
            }
        )+
    };
}

py_primitive_scalar! {
    (I8Scalar, I8Type, i8, "int8"),
    (I16Scalar, I16Type, i16, "int16"),
    (I32Scalar, I32Type, i32, "int32"),
    (U8Scalar, U8Type, u8, "uint8"),
    (U16Scalar, U16Type, u16, "uint16"),
    (U32Scalar, U32Type, u32, "uint32"),
    (F32Scalar, F32Type, f32, "float32"),
    (F64Scalar, F64Type, f64, "float64"),
    (BooleanScalar, BooleanType, bool, "boolean"),
}

/// Generates the pyo3 wrapper for one 64-bit integer scalar (`I64Scalar` / `U64Scalar`),
/// specialised out of [`py_primitive_scalar`] so its constructor **range-checks** the `int`
/// against the width and raises the same guided `ValueError` as the Node binding (rule 12)
/// rather than pyo3's opaque `OverflowError`. `$native` is the stored type; the ctor takes a
/// wide `i128` so the check — not a truncating conversion — is what rejects out-of-range.
macro_rules! py_checked_int_scalar {
    ($( ($scalar:ident, $dtype:ident, $native:ty, $lit:literal, $msg:literal) ),+ $(,)?) => {
        $(
            #[doc = concat!("A single `", $lit, "` value (always present). An out-of-range `int` raises a guided `ValueError`.")]
            #[pyclass(module = "yggdryl.scalar")]
            #[derive(Clone)]
            pub struct $scalar {
                pub(crate) inner: yggdryl_scalar::$scalar,
            }

            #[pymethods]
            impl $scalar {
                #[new]
                fn new(value: i128) -> PyResult<Self> {
                    let value = <$native>::try_from(value)
                        .map_err(|_| PyValueError::new_err($msg))?;
                    Ok(Self { inner: yggdryl_scalar::$scalar::new(value) })
                }

                /// The default scalar of this type (its data type's default value).
                #[staticmethod]
                fn default_scalar() -> Self {
                    Self { inner: yggdryl_scalar::$scalar::default_scalar() }
                }

                /// The scalar's value (always present).
                #[getter]
                fn value(&self) -> $native {
                    TypedScalar::value(&self.inner)
                }

                /// The scalar's data type (a `yggdryl.dtype` class).
                #[getter]
                fn data_type(&self) -> crate::dtype::$dtype {
                    crate::dtype::$dtype { inner: TypedScalar::data_type(&self.inner) }
                }

                /// The scalar serialised to its value's little-endian bytes.
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
                    format!("{}({})", stringify!($scalar), TypedScalar::value(&self.inner))
                }
            }
        )+
    };
}

py_checked_int_scalar! {
    (I64Scalar, I64Type, i64, "int64",
     "value out of range for int64; expected -9223372036854775808..=9223372036854775807"),
    (U64Scalar, U64Type, u64, "uint64",
     "value out of range for uint64; expected 0..=18446744073709551615"),
}

/// The single value of the `null` data type — a scalar whose value is "null".
///
/// A scalar is always present, so this is not a nullable wrapper: it is the one value of
/// the sui-generis `NullType`. Its `value` is always `None` (the null value) and it
/// serialises to zero bytes.
#[pyclass(module = "yggdryl.scalar")]
#[derive(Clone)]
pub struct NullScalar {
    pub(crate) inner: yggdryl_scalar::NullScalar,
}

#[pymethods]
impl NullScalar {
    #[new]
    fn new() -> Self {
        Self {
            inner: yggdryl_scalar::NullScalar::new(),
        }
    }

    /// The default scalar of this type — the null value.
    #[staticmethod]
    fn default_scalar() -> Self {
        Self {
            inner: yggdryl_scalar::NullScalar::default_scalar(),
        }
    }

    /// The scalar's value — always `None` (the null value).
    #[getter]
    fn value(&self, py: Python<'_>) -> PyObject {
        py.None()
    }

    /// The scalar's data type (a `yggdryl.dtype.NullType`).
    #[getter]
    fn data_type(&self) -> crate::dtype::NullType {
        crate::dtype::NullType {
            inner: TypedScalar::data_type(&self.inner),
        }
    }

    /// The scalar serialised to its (empty) value bytes.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs the scalar from its serialised bytes (which must be empty).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        yggdryl_scalar::NullScalar::deserialize_bytes(bytes)
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
            .get_type_bound::<NullScalar>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        "NullScalar()".to_string()
    }
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
    module.add_class::<NullScalar>()?;
    Ok(())
}
