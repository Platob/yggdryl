//! The `yggdryl.decimal` submodule's **columnar** decimal types — one nullable value carrying its
//! column `(precision, scale)` (`D32Scalar` … `D256Scalar`) and one nullable decimal column
//! (`D32Serie` … `D256Serie`), mirroring `yggdryl_core::io::fixed`'s `DecimalScalar<B>` /
//! `DecimalSerie<B>`.
//!
//! A column fixes one `(precision, scale)` (Arrow's model): a value is re-expressed at that scale
//! (a guided `ValueError` if it does not fit exactly, or exceeds the precision). **Values cross as
//! exact decimal strings** (`"123.45"`) — the same form across Python and Node, and losslessly
//! parseable into the `D32` … `D256` value types for arithmetic. A `Scalar`
//! is an immutable value (hashable by its decimal value, so `2.5` equals `2.50`); a `Serie` is a
//! mutable column (unhashable).

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList};

use yggdryl_core::io::fixed::{
    Dec128, Dec256, Dec32, Dec64, Decimal, DecimalBacking, DecimalError, DecimalScalar,
    DecimalSerie,
};

/// Maps a core [`DecimalError`] to a Python `ValueError` (its guided text passes through unchanged).
fn dec_err(error: DecimalError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Parses a decimal literal (`"-123.45"`) into a value of width `B`.
fn parse_dec<B: DecimalBacking>(text: &str) -> PyResult<Decimal<B>> {
    text.parse::<Decimal<B>>().map_err(dec_err)
}

/// Generates the columnar `Scalar` **and** `Serie` for one decimal width.
macro_rules! py_dec_col {
    ($Scalar:ident, $Serie:ident, $B:ty, $lit:literal) => {
        #[doc = concat!("A single, nullable `", $lit, "` value carrying its column `(precision, scale)`.")]
        #[pyclass(module = "yggdryl.decimal")]
        #[derive(Clone)]
        pub struct $Scalar {
            pub(crate) inner: DecimalScalar<$B>,
        }

        #[pymethods]
        impl $Scalar {
            /// A scalar from a decimal string. With no `precision`/`scale` they are inferred from
            /// the value; pass **both** to pin the column type (re-expressing the value, a
            /// `ValueError` if it does not fit). `value=None` is a null of the given (or default)
            /// `(precision, scale)`.
            #[new]
            #[pyo3(signature = (value = None, precision = None, scale = None))]
            fn new(
                value: Option<&str>,
                precision: Option<u8>,
                scale: Option<i8>,
            ) -> PyResult<Self> {
                match value {
                    None => Ok(Self {
                        inner: DecimalScalar::null(
                            precision.unwrap_or(<$B as DecimalBacking>::MAX_PRECISION),
                            scale.unwrap_or(0),
                        ),
                    }),
                    Some(text) => {
                        let value = parse_dec::<$B>(text)?;
                        match (precision, scale) {
                            (Some(precision), Some(scale)) => {
                                DecimalScalar::with_precision_scale(value, precision, scale)
                                    .map(|inner| Self { inner })
                                    .map_err(dec_err)
                            }
                            _ => Ok(Self {
                                inner: DecimalScalar::of(value),
                            }),
                        }
                    }
                }
            }

            /// The null scalar of the given `(precision, scale)`.
            #[staticmethod]
            fn null(precision: u8, scale: i8) -> Self {
                Self {
                    inner: DecimalScalar::null(precision, scale),
                }
            }

            /// The value as a decimal string, or `None` if null.
            #[getter]
            fn value(&self) -> Option<String> {
                self.inner.value().map(|value| value.to_string())
            }

            /// Whether the scalar is null.
            #[getter]
            fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The column precision.
            #[getter]
            fn precision(&self) -> u8 {
                self.inner.precision()
            }

            /// The column scale.
            #[getter]
            fn scale(&self) -> i8 {
                self.inner.scale()
            }

            /// The scalar's canonical bytes (`[validity][precision][scale][coefficient]`).
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }

            /// Reconstructs a scalar from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                DecimalScalar::<$B>::deserialize_bytes(bytes)
                    .map(|inner| Self { inner })
                    .map_err(|error| PyValueError::new_err(error.to_string()))
            }

            /// Value equality (`2.5` equals `2.50`).
            fn __eq__(&self, other: &Self) -> bool {
                self.inner == other.inner
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
                    .get_type_bound::<$Scalar>()
                    .getattr("deserialize_bytes")?
                    .unbind();
                let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                    .into_any()
                    .unbind();
                Ok((ctor, (state,)))
            }

            fn __repr__(&self) -> String {
                match self.inner.value() {
                    Some(value) => format!(
                        "{}(\"{}\", precision={}, scale={})",
                        stringify!($Scalar),
                        value,
                        self.inner.precision(),
                        self.inner.scale()
                    ),
                    None => format!(
                        "{}(null, precision={}, scale={})",
                        stringify!($Scalar),
                        self.inner.precision(),
                        self.inner.scale()
                    ),
                }
            }
        }

        #[doc = concat!("A nullable column of `", $lit, "` values at one `(precision, scale)`.")]
        #[pyclass(module = "yggdryl.decimal")]
        #[derive(Clone)]
        pub struct $Serie {
            pub(crate) inner: DecimalSerie<$B>,
        }

        #[pymethods]
        impl $Serie {
            /// A column of `(precision, scale)` from a list of decimal-string-or-`None` (empty by
            /// default). Each value is re-expressed at the column's scale.
            #[new]
            #[pyo3(signature = (precision, scale, values = None))]
            fn new(
                precision: u8,
                scale: i8,
                values: Option<Vec<Option<String>>>,
            ) -> PyResult<Self> {
                match values {
                    None => Ok(Self {
                        inner: DecimalSerie::new(precision, scale),
                    }),
                    Some(values) => {
                        let mut options = Vec::with_capacity(values.len());
                        for value in values {
                            options.push(match value {
                                Some(text) => Some(parse_dec::<$B>(&text)?),
                                None => None,
                            });
                        }
                        DecimalSerie::from_options(precision, scale, &options)
                            .map(|inner| Self { inner })
                            .map_err(dec_err)
                    }
                }
            }

            /// A non-null column from a list of present decimal strings.
            #[staticmethod]
            fn from_values(precision: u8, scale: i8, values: Vec<String>) -> PyResult<Self> {
                let mut owned = Vec::with_capacity(values.len());
                for text in values {
                    owned.push(parse_dec::<$B>(&text)?);
                }
                DecimalSerie::from_values(precision, scale, &owned)
                    .map(|inner| Self { inner })
                    .map_err(dec_err)
            }

            /// A column from a list of this type's scalars — each item is a `$Scalar` (or `None`, a
            /// null element). The column `(precision, scale)` is taken from the first present scalar
            /// (defaulting to `(MAX_PRECISION, 0)` when the list has no value); each value is
            /// re-expressed at that scale. The inverse of `get_scalar` over the whole column.
            #[staticmethod]
            fn from_scalars(scalars: &Bound<'_, PyAny>) -> PyResult<Self> {
                let mut items: Vec<Option<DecimalScalar<$B>>> = Vec::new();
                for item in scalars.iter()? {
                    let item = item?;
                    items.push(if item.is_none() {
                        None
                    } else {
                        Some(item.extract::<$Scalar>()?.inner)
                    });
                }
                let (precision, scale) = items
                    .iter()
                    .flatten()
                    .next()
                    .map(|scalar| (scalar.precision(), scalar.scale()))
                    .unwrap_or((<$B as DecimalBacking>::MAX_PRECISION, 0));
                let inners: Vec<DecimalScalar<$B>> = items
                    .into_iter()
                    .map(|item| item.unwrap_or_else(|| DecimalScalar::null(precision, scale)))
                    .collect();
                DecimalSerie::from_scalars(precision, scale, &inners)
                    .map(|inner| Self { inner })
                    .map_err(dec_err)
            }

            /// Appends one element (a decimal string, or `None` for a null).
            #[pyo3(signature = (value = None))]
            fn push(&mut self, value: Option<&str>) -> PyResult<()> {
                let decimal = match value {
                    Some(text) => Some(parse_dec::<$B>(text)?),
                    None => None,
                };
                self.inner.push(decimal).map_err(dec_err)
            }

            /// The value at `index` (a decimal string), or `None` if null or out of range.
            fn get(&self, index: usize) -> Option<String> {
                self.inner.get(index).map(|value| value.to_string())
            }

            /// Element `index` as a scalar (carrying the column's `(precision, scale)`).
            fn get_scalar(&self, index: usize) -> $Scalar {
                $Scalar {
                    inner: self.inner.get_scalar(index),
                }
            }

            /// Overwrites element `index` (a decimal string, or `None` for a null); raises
            /// `ValueError` out of range or if the value does not fit `(precision, scale)`.
            #[pyo3(signature = (index, value = None))]
            fn set(&mut self, index: usize, value: Option<&str>) -> PyResult<()> {
                let decimal = match value {
                    Some(text) => Some(parse_dec::<$B>(text)?),
                    None => None,
                };
                self.inner.set(index, decimal).map_err(dec_err)
            }

            /// The column precision.
            #[getter]
            fn precision(&self) -> u8 {
                self.inner.precision()
            }

            /// The column scale.
            #[getter]
            fn scale(&self) -> i8 {
                self.inner.scale()
            }

            /// The number of null elements.
            #[getter]
            fn null_count(&self) -> usize {
                self.inner.null_count()
            }

            /// Whether the column carries any nulls.
            #[getter]
            fn has_nulls(&self) -> bool {
                self.inner.has_nulls()
            }

            /// Whether the column is empty.
            fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            /// The elements as a list of decimal-string-or-`None`, in order.
            fn to_options(&self) -> Vec<Option<String>> {
                (0..self.inner.len())
                    .map(|index| self.inner.get(index).map(|value| value.to_string()))
                    .collect()
            }

            /// The column's canonical bytes (`[len][precision][scale][flags][validity?][values]`).
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }

            /// Reconstructs a column from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                DecimalSerie::<$B>::deserialize_bytes(bytes)
                    .map(|inner| Self { inner })
                    .map_err(|error| PyValueError::new_err(error.to_string()))
            }

            fn __len__(&self) -> usize {
                self.inner.len()
            }

            fn __bool__(&self) -> bool {
                !self.inner.is_empty()
            }

            /// Random access — `col[i]` returns the value string or `None` (negative indices
            /// allowed); raises `IndexError` out of range.
            fn __getitem__(&self, index: isize) -> PyResult<Option<String>> {
                let len = self.inner.len() as isize;
                let resolved = if index < 0 { index + len } else { index };
                if resolved < 0 || resolved >= len {
                    return Err(PyIndexError::new_err("Serie index out of range"));
                }
                Ok(self.inner.get(resolved as usize).map(|value| value.to_string()))
            }

            /// Iterates the elements as decimal-string-or-`None`, in order.
            fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                Ok(PyList::new_bound(py, self.to_options())
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
                    .get_type_bound::<$Serie>()
                    .getattr("deserialize_bytes")?
                    .unbind();
                let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
                    .into_any()
                    .unbind();
                Ok((ctor, (state,)))
            }

            fn __repr__(&self) -> String {
                format!(
                    "{}(len={}, precision={}, scale={}, null_count={})",
                    stringify!($Serie),
                    self.inner.len(),
                    self.inner.precision(),
                    self.inner.scale(),
                    self.inner.null_count()
                )
            }
        }
    };
}

py_dec_col!(D32Scalar, D32Serie, Dec32, "d32");
py_dec_col!(D64Scalar, D64Serie, Dec64, "d64");
py_dec_col!(D128Scalar, D128Serie, Dec128, "d128");
py_dec_col!(D256Scalar, D256Serie, Dec256, "d256");

// Phase 8 reshape (`filter` / `fill_null` / `to_list` / `to_struct` / `to_map`). A decimal column's
// `fill_null` takes a single-element decimal Serie carrier so the core can guard a scale mismatch;
// decimals get no arithmetic (out of the twelve-numeric scope).
crate::nested::reshape_methods!(D32Serie);
crate::nested::reshape_methods!(D64Serie);
crate::nested::reshape_methods!(D128Serie);
crate::nested::reshape_methods!(D256Serie);

// Phase 9 slice assignment (`serie[a:b] = other`) — the `set_slice` named twin is on every decimal
// column via `reshape_methods!` above.
crate::nested::slice_setitem!(D32Serie, D64Serie, D128Serie, D256Serie);

/// Adds the columnar decimal `Scalar` / `Serie` classes to the `yggdryl.decimal` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<D32Scalar>()?;
    module.add_class::<D32Serie>()?;
    module.add_class::<D64Scalar>()?;
    module.add_class::<D64Serie>()?;
    module.add_class::<D128Scalar>()?;
    module.add_class::<D128Serie>()?;
    module.add_class::<D256Scalar>()?;
    module.add_class::<D256Serie>()?;
    Ok(())
}
