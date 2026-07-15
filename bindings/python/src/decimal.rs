//! The `yggdryl.decimal` submodule — the fixed-width **scaled-decimal value types** `D32` / `D64`
//! / `D128` / `D256`, mirroring `yggdryl_core::io::fixed`'s `Decimal<B>` family. Each value is a
//! coefficient integer scaled by a power of ten (`value = coefficient × 10^-scale`), with checked
//! arithmetic, true numeric ordering, value identity (`2.5 == 2.50`, hash-equal), a byte codec,
//! conversions to/from integers and floats, and casts between the widths.
//!
//! The coefficient marshals as a native Python `int` for every width — including `D256`'s 256-bit
//! coefficient, carried through its decimal string (no native 256-bit int exists). The
//! constructor range-checks it in the core, so the guided `ValueError` reads identically across
//! Python, Node, and Rust.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::pyclass::CompareOp;
use pyo3::types::{PyBytes, PyInt};

use yggdryl_core::io::fixed::{Dec128, Dec256, Dec32, Dec64, DecimalError};

/// Maps a core [`DecimalError`] to a Python `ValueError` (its guided text passes through unchanged).
fn decimal_err(error: DecimalError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The decimal string of a Python `int`, or a `TypeError` if the value is not an `int`.
fn int_to_string(value: &Bound<'_, PyAny>) -> PyResult<String> {
    if value.downcast::<PyInt>().is_err() {
        return Err(PyTypeError::new_err("expected an int coefficient"));
    }
    value.str()?.extract()
}

/// A Python `int` from a decimal string (`int("-12345")`) — carries any width, including beyond 128 bits.
fn string_to_int(py: Python<'_>, text: &str) -> PyResult<PyObject> {
    Ok(py.get_type_bound::<PyInt>().call1((text,))?.unbind())
}

/// Generates the pyo3 wrapper for one decimal width.
macro_rules! py_decimal {
    ($Wrapper:ident, $core:ty, $lit:literal) => {
        #[doc = concat!("A fixed-width `", $lit, "` decimal — a coefficient integer × 10^(−scale).")]
        #[pyclass(module = "yggdryl.decimal")]
        #[derive(Clone)]
        pub struct $Wrapper {
            pub(crate) inner: $core,
        }

        #[pymethods]
        impl $Wrapper {
            /// Builds a decimal from an integer `coefficient` and `scale` (default `0`), raising a
            /// guided `ValueError` if the coefficient does not fit the width.
            #[new]
            #[pyo3(signature = (coefficient, scale = 0))]
            fn new(coefficient: &Bound<'_, PyAny>, scale: i8) -> PyResult<Self> {
                let text = int_to_string(coefficient)?;
                <$core>::from_coeff_str(&text, scale)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// Parses a decimal literal (`"-123.45"`), raising a guided `ValueError` if malformed.
            #[staticmethod]
            fn from_string(text: &str) -> PyResult<Self> {
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// The decimal nearest `value` at `scale` (default `0`), raising `ValueError` for a
            /// non-finite float or an out-of-range result.
            #[staticmethod]
            #[pyo3(signature = (value, scale = 0))]
            fn from_float(value: f64, scale: i8) -> PyResult<Self> {
                <$core>::from_f64(value, scale)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// The unscaled integer coefficient (a native `int`, any width).
            #[getter]
            fn coefficient(&self, py: Python<'_>) -> PyResult<PyObject> {
                string_to_int(py, &self.inner.coefficient_string())
            }

            /// The scale (number of fractional decimal digits).
            #[getter]
            fn scale(&self) -> i8 {
                self.inner.scale()
            }

            /// The value's precision — its significant-digit count.
            #[getter]
            fn precision(&self) -> u32 {
                self.inner.precision()
            }

            /// The width's maximum precision (`9`/`18`/`38`/`76`).
            #[getter]
            fn max_precision(&self) -> u8 {
                <$core>::max_precision()
            }

            /// The coefficient width in bits (`32`/`64`/`128`/`256`).
            #[getter]
            fn bits(&self) -> u32 {
                <$core>::bit_width()
            }

            /// Whether the value is exactly zero.
            fn is_zero(&self) -> bool {
                self.inner.is_zero()
            }
            /// Whether the value is strictly negative.
            fn is_negative(&self) -> bool {
                self.inner.is_negative()
            }
            /// Whether the value is strictly positive.
            fn is_positive(&self) -> bool {
                self.inner.is_positive()
            }

            /// The value as a float (lossy beyond 53 bits of mantissa).
            fn to_float(&self) -> f64 {
                self.inner.to_f64()
            }

            /// The exact integer value, raising `ValueError` if it has a fractional part or exceeds
            /// `i128`.
            fn to_int(&self) -> PyResult<i128> {
                self.inner.to_i128().map_err(decimal_err)
            }

            /// `int(self)` — the integer part, truncated toward zero (any width, via the digits).
            fn __int__(&self, py: Python<'_>) -> PyResult<PyObject> {
                string_to_int(py, &self.inner.trunc().coefficient_string())
            }

            /// `float(self)` — the value as a Python `float` (lossy).
            fn __float__(&self) -> f64 {
                self.inner.to_f64()
            }

            /// This value as a native Python [`decimal.Decimal`] — exact, via its decimal string.
            fn to_decimal<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
                py.import_bound("decimal")?
                    .getattr("Decimal")?
                    .call1((self.inner.to_string(),))
            }

            /// A decimal from a native Python [`decimal.Decimal`] (or any value with a decimal
            /// string form, including scientific notation), raising `ValueError` if it is not a
            /// finite decimal.
            #[staticmethod]
            fn from_decimal(value: &Bound<'_, PyAny>) -> PyResult<Self> {
                let text: String = value.str()?.extract()?;
                text.parse::<$core>()
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// This value re-expressed at `new_scale`, exactly — raising `ValueError` if lowering
            /// the scale would drop non-zero digits, or on overflow.
            fn rescale(&self, new_scale: i8) -> PyResult<Self> {
                self.inner
                    .rescale(new_scale)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// This value at `new_scale`, rounding dropped digits half-away-from-zero.
            fn round_to_scale(&self, new_scale: i8) -> PyResult<Self> {
                self.inner
                    .round_to_scale(new_scale)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// This value at `new_scale`, truncating dropped digits toward zero.
            fn trunc_to_scale(&self, new_scale: i8) -> PyResult<Self> {
                self.inner
                    .trunc_to_scale(new_scale)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// The integer part, truncated toward zero (scale `0`).
            fn trunc(&self) -> Self {
                Self {
                    inner: self.inner.trunc(),
                }
            }

            /// The value with trailing fractional zeros stripped (`2.50` → `2.5`).
            fn normalized(&self) -> Self {
                Self {
                    inner: self.inner.normalized(),
                }
            }

            /// `self / other` at `result_scale`, raising `ValueError` on divide-by-zero or overflow.
            fn div(&self, other: &Self, result_scale: i8) -> PyResult<Self> {
                self.inner
                    .checked_div(&other.inner, result_scale)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// This value cast to `d32`, raising `ValueError` if it does not fit.
            fn to_d32(&self) -> PyResult<D32> {
                self.inner
                    .cast::<Dec32>()
                    .map(|inner| D32 { inner })
                    .map_err(decimal_err)
            }
            /// This value cast to `d64`, raising `ValueError` if it does not fit.
            fn to_d64(&self) -> PyResult<D64> {
                self.inner
                    .cast::<Dec64>()
                    .map(|inner| D64 { inner })
                    .map_err(decimal_err)
            }
            /// This value cast to `d128`, raising `ValueError` if it does not fit.
            fn to_d128(&self) -> PyResult<D128> {
                self.inner
                    .cast::<Dec128>()
                    .map(|inner| D128 { inner })
                    .map_err(decimal_err)
            }
            /// This value cast to `d256` (always exact from a narrower width).
            fn to_d256(&self) -> PyResult<D256> {
                self.inner
                    .cast::<Dec256>()
                    .map(|inner| D256 { inner })
                    .map_err(decimal_err)
            }

            /// The canonical byte encoding — `[scale][coefficient little-endian]` of the normalized
            /// value.
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }

            /// Reconstructs a decimal from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                <$core>::deserialize_bytes(bytes)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// `self + other`, raising `ValueError` on overflow.
            fn __add__(&self, other: &Self) -> PyResult<Self> {
                self.inner
                    .checked_add(&other.inner)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }
            /// `self - other`, raising `ValueError` on overflow.
            fn __sub__(&self, other: &Self) -> PyResult<Self> {
                self.inner
                    .checked_sub(&other.inner)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }
            /// `self * other`, raising `ValueError` on overflow.
            fn __mul__(&self, other: &Self) -> PyResult<Self> {
                self.inner
                    .checked_mul(&other.inner)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }
            /// `self % other` (scales aligned), raising `ValueError` on divide-by-zero.
            fn __mod__(&self, other: &Self) -> PyResult<Self> {
                self.inner
                    .checked_rem(&other.inner)
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }
            /// `-self`, raising `ValueError` on overflow (the two's-complement minimum).
            fn __neg__(&self) -> PyResult<Self> {
                self.inner
                    .checked_neg()
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }
            /// The absolute value, raising `ValueError` on overflow.
            fn __abs__(&self) -> PyResult<Self> {
                self.inner
                    .checked_abs()
                    .map(|inner| Self { inner })
                    .map_err(decimal_err)
            }

            /// True numeric comparison (`2.5 == 2.50`, and `2.5 < 2.75`).
            fn __richcmp__(&self, other: &Self, op: CompareOp) -> bool {
                op.matches(self.inner.cmp(&other.inner))
            }

            fn __hash__(&self) -> u64 {
                let mut hasher = DefaultHasher::new();
                self.inner.hash(&mut hasher);
                hasher.finish()
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
                    .get_type_bound::<$Wrapper>()
                    .getattr("deserialize_bytes")?
                    .unbind();
                let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                    .into_any()
                    .unbind();
                Ok((ctor, (state,)))
            }

            /// The value in plain decimal form, e.g. `"123.45"`.
            fn __str__(&self) -> String {
                self.inner.to_string()
            }

            fn __repr__(&self) -> String {
                // The value in decimal form (which preserves the scale: 123.45 vs 123.450).
                format!("{}(\"{}\")", stringify!($Wrapper), self.inner)
            }
        }
    };
}

py_decimal!(D32, yggdryl_core::io::fixed::D32, "d32");
py_decimal!(D64, yggdryl_core::io::fixed::D64, "d64");
py_decimal!(D128, yggdryl_core::io::fixed::D128, "d128");
py_decimal!(D256, yggdryl_core::io::fixed::D256, "d256");

/// Populates the `decimal` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<D32>()?;
    module.add_class::<D64>()?;
    module.add_class::<D128>()?;
    module.add_class::<D256>()?;
    Ok(())
}
