//! The `yggdryl.decimal` submodule — fixed-width decimals.
//!
//! Exposes the four Arrow decimal widths (`Decimal32` / `Decimal64` / `Decimal128` /
//! `Decimal256`), mirroring `yggdryl_core`'s `decimal` module. Each is an integer
//! **mantissa** scaled by a power of ten (`value = mantissa × 10^(−scale)`), with value
//! semantics (equal iff `serialize_bytes` are equal), a byte round-trip, `f64` / integer
//! conversion, rescaling, and widening / narrowing between the widths.
//!
//! Mantissa marshalling: Python integers are arbitrary-precision, so every width's mantissa
//! is a plain `int` — including `Decimal128`'s 128-bit and `Decimal256`'s 256-bit mantissa
//! (the latter bridged through its decimal string, as no native 256-bit int exists). The
//! constructor range-checks the mantissa against the width and raises a guided `ValueError`
//! naming the accepted range when it does not fit (`CLAUDE.md` rule 12).
//!
//! The direct narrow-to-narrow widenings (`Decimal32` → `Decimal64`, etc.) are Rust-only
//! `From` conveniences; the bindings expose the value-preserving ladder as
//! `to_decimal256()` (on the three narrow widths) and the fallible narrowing
//! `Decimal256.try_to_decimal128()`.

#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyInt};

use yggdryl_core::i256;

/// Maps a [`yggdryl_core::DecimalError`] to a Python `ValueError` (its guided text).
fn decimal_err(error: yggdryl_core::DecimalError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Generates the pyo3 wrapper for one decimal width whose mantissa marshals as a native
/// Python `int` (`Decimal32`/`Decimal64`/`Decimal128` — `i32`/`i64`/`i128`).
macro_rules! py_decimal_native {
    ($( ($name:ident, $int:ty, $lit:literal) ),+ $(,)?) => {
        $(
            #[doc = concat!("A fixed-width `", $lit, "` (integer mantissa × 10^(−scale)).")]
            #[pyclass(module = "yggdryl.decimal")]
            #[derive(Clone)]
            pub struct $name {
                pub(crate) inner: yggdryl_core::$name,
            }

            #[pymethods]
            impl $name {
                /// Builds a decimal from an integer `mantissa` and `scale` (default `0`),
                /// raising a guided `ValueError` if the mantissa does not fit the width.
                #[new]
                #[pyo3(signature = (mantissa, scale = 0))]
                fn new(mantissa: i128, scale: i8) -> PyResult<Self> {
                    yggdryl_core::$name::from_integer(mantissa, scale)
                        .map(|inner| Self { inner })
                        .map_err(decimal_err)
                }

                /// Builds a decimal approximating `value` at `scale` (rounding the mantissa).
                #[staticmethod]
                #[pyo3(signature = (value, scale = 0))]
                fn from_f64(value: f64, scale: i8) -> Self {
                    Self { inner: yggdryl_core::$name::from_f64(value, scale) }
                }

                /// The unscaled integer mantissa.
                #[getter]
                fn mantissa(&self) -> $int {
                    self.inner.mantissa()
                }

                /// The scale (number of fractional decimal digits).
                #[getter]
                fn scale(&self) -> i8 {
                    self.inner.scale()
                }

                /// The mantissa width in bits.
                #[getter]
                fn bits(&self) -> u32 {
                    yggdryl_core::$name::BITS
                }

                /// The value as a float (`mantissa / 10^scale`; lossy for large mantissas).
                fn to_f64(&self) -> f64 {
                    self.inner.to_f64()
                }

                /// The integer part, truncated toward zero, or `None` if it overflows `i128`.
                fn to_i128(&self) -> Option<i128> {
                    self.inner.to_i128()
                }

                /// Re-expresses the value at `new_scale`, raising a guided `ValueError` if the
                /// rescaled mantissa no longer fits the width.
                fn rescale(&self, new_scale: i8) -> PyResult<Self> {
                    self.inner
                        .rescale(new_scale)
                        .map(|inner| Self { inner })
                        .map_err(decimal_err)
                }

                /// Widens to a `Decimal256` (same scale; always exact).
                fn to_decimal256(&self) -> Decimal256 {
                    Decimal256 { inner: self.inner.to_decimal256() }
                }

                /// The value's bytes: the mantissa's little-endian bytes then the scale byte.
                fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, &self.inner.serialize_bytes())
                }

                /// Reconstructs a decimal from its serialised bytes.
                #[staticmethod]
                fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                    yggdryl_core::$name::deserialize_bytes(bytes)
                        .map(|inner| Self { inner })
                        .map_err(decimal_err)
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
                        .get_type_bound::<$name>()
                        .getattr("deserialize_bytes")?
                        .unbind();
                    let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                        .into_any()
                        .unbind();
                    Ok((ctor, (state,)))
                }

                /// The decimal value in plain form, e.g. `"123.45"`.
                fn __str__(&self) -> String {
                    self.inner.to_string()
                }

                fn __repr__(&self) -> String {
                    format!("{}({}, {})", stringify!($name), self.inner.mantissa(), self.inner.scale())
                }
            }
        )+
    };
}

py_decimal_native! {
    (Decimal32, i32, "decimal32"),
    (Decimal64, i64, "decimal64"),
    (Decimal128, i128, "decimal128"),
}

/// Bridges a Python `int` to an [`i256`] via its decimal string — there is no native
/// 256-bit integer, so the mantissa crosses as text. Raises a guided error if the value is
/// not an `int` or does not fit the 256-bit range.
fn py_into_i256(value: &Bound<'_, PyAny>) -> PyResult<i256> {
    if value.downcast::<PyInt>().is_err() {
        return Err(PyTypeError::new_err(
            "expected an int mantissa for Decimal256",
        ));
    }
    let text: String = value.str()?.extract()?;
    i256::from_string(&text).ok_or_else(|| {
        PyValueError::new_err(format!(
            "mantissa is out of range for decimal256; expected {}..={}",
            i256::MIN,
            i256::MAX
        ))
    })
}

/// Turns an [`i256`] into a Python `int` via its decimal string.
fn i256_into_py(py: Python<'_>, value: i256) -> PyResult<PyObject> {
    Ok(py
        .get_type_bound::<PyInt>()
        .call1((value.to_string(),))?
        .unbind())
}

/// A fixed-width `decimal256` (256-bit integer mantissa × 10^(−scale)). Its mantissa
/// marshals as a Python `int` through the value's decimal string.
#[pyclass(module = "yggdryl.decimal")]
#[derive(Clone)]
pub struct Decimal256 {
    pub(crate) inner: yggdryl_core::Decimal256,
}

#[pymethods]
impl Decimal256 {
    /// Builds a decimal from an integer `mantissa` and `scale` (default `0`), raising a
    /// guided `ValueError` if the mantissa exceeds the 256-bit range.
    #[new]
    #[pyo3(signature = (mantissa, scale = 0))]
    fn new(mantissa: &Bound<'_, PyAny>, scale: i8) -> PyResult<Self> {
        Ok(Self {
            inner: yggdryl_core::Decimal256::new(py_into_i256(mantissa)?, scale),
        })
    }

    /// Builds a decimal approximating `value` at `scale`.
    #[staticmethod]
    #[pyo3(signature = (value, scale = 0))]
    fn from_f64(value: f64, scale: i8) -> Self {
        Self {
            inner: yggdryl_core::Decimal256::from_f64(value, scale),
        }
    }

    /// The unscaled integer mantissa (a Python `int`, possibly beyond 128 bits).
    #[getter]
    fn mantissa(&self, py: Python<'_>) -> PyResult<PyObject> {
        i256_into_py(py, self.inner.mantissa())
    }

    /// The scale (number of fractional decimal digits).
    #[getter]
    fn scale(&self) -> i8 {
        self.inner.scale()
    }

    /// The mantissa width in bits.
    #[getter]
    fn bits(&self) -> u32 {
        yggdryl_core::Decimal256::BITS
    }

    /// The value as a float (`mantissa / 10^scale`; lossy for large mantissas).
    fn to_f64(&self) -> f64 {
        self.inner.to_f64()
    }

    /// The integer part, truncated toward zero, or `None` if it exceeds `i128`.
    fn to_i128(&self) -> Option<i128> {
        self.inner.to_i128()
    }

    /// Re-expresses the value at `new_scale`, raising a guided `ValueError` on overflow.
    fn rescale(&self, new_scale: i8) -> PyResult<Self> {
        self.inner
            .rescale(new_scale)
            .map(|inner| Self { inner })
            .map_err(decimal_err)
    }

    /// Narrows to a `Decimal128` if the mantissa fits `i128`, else raises a guided error.
    fn try_to_decimal128(&self) -> PyResult<Decimal128> {
        self.inner
            .try_to_decimal128()
            .map(|inner| Decimal128 { inner })
            .map_err(decimal_err)
    }

    /// The value's bytes: the mantissa's 32 little-endian bytes then the scale byte.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a decimal from its serialised bytes.
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        yggdryl_core::Decimal256::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(decimal_err)
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
            .get_type_bound::<Decimal256>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    /// The decimal value in plain form, e.g. `"123.45"`.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "Decimal256({}, {})",
            self.inner.mantissa(),
            self.inner.scale()
        )
    }
}

/// Populates the `decimal` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Decimal32>()?;
    module.add_class::<Decimal64>()?;
    module.add_class::<Decimal128>()?;
    module.add_class::<Decimal256>()?;
    Ok(())
}
