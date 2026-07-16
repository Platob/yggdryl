//! The `yggdryl.types` submodule's **fixed-width value layer** — one nullable value
//! ([`Scalar`](yggdryl_core::io::fixed::Scalar)) and one nullable column
//! ([`Serie`](yggdryl_core::io::fixed::Serie)) per primitive: `U8Scalar`/`U8Serie` …
//! `I256Scalar`/`I256Serie`, `F16Scalar`/`F16Serie` … `F64Scalar`/`F64Serie`.
//!
//! Mirrors `yggdryl_core::io::fixed`'s generic `Scalar<T>` / `Serie<T>` method-for-method; each
//! wrapper is macro-generated and delegates to the core. A `Scalar` is an immutable value (so it
//! is hashable, equatable, and pickles through its byte codec); a `Serie` is a mutable column (so,
//! like `bytearray`/`dict`, it is **not** hashable) with `len()`/indexing/iteration.
//!
//! **Value marshaling** depends on the element width: the small integers (`u8`…`u32`, `i8`…`i32`)
//! cross as native `int`; the wide integers (`u64`/`i64`/`u128`/`i128`) as a **decimal string**
//! (exact at any width); the 96/256-bit integers (`u96`/`i96`/`u256`/`i256`), which have no
//! cross-language numeric form, as their **little-endian bytes**; and the floats
//! (`f16`/`f32`/`f64`) as native `float`. `None` is a null element throughout.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList};

use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::fixed::{f16, NativeType, Scalar, Serie, I256, I96, U256, U96};
use yggdryl_core::io::{CastError, IoError};

use crate::types::{DataType, Field};
use crate::varvalues::{BinaryScalar, Utf8Scalar};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn io_err(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Maps a [`CastError`] to a Python `ValueError` carrying its guided text.
fn cast_err(error: CastError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Cross-FFI marshaling for one native element type: how a present value crosses to Python
/// (`to_py`) and back (`from_py`). One impl per width class (native int / decimal string / wide
/// little-endian bytes / float); the [`Scalar`]/[`Serie`] wrappers are otherwise identical.
pub(crate) trait PyNative: NativeType {
    /// This value as the Python object the binding exposes.
    fn to_py(self, py: Python<'_>) -> PyResult<PyObject>;
    /// A value from a Python object (an `int` / decimal string / bytes-like / `float`).
    fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Self>;
}

/// Small integers (`u8`…`u32`, `i8`…`i32`) — cross as a native Python `int`.
macro_rules! py_native_int {
    ($t:ty) => {
        impl PyNative for $t {
            fn to_py(self, py: Python<'_>) -> PyResult<PyObject> {
                Ok(self.into_py(py))
            }
            fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
                obj.extract()
            }
        }
    };
}
py_native_int!(u8);
py_native_int!(u16);
py_native_int!(u32);
py_native_int!(i8);
py_native_int!(i16);
py_native_int!(i32);

/// Wide native integers (`u64`/`i64`/`u128`/`i128`) — cross as an exact **decimal string** (an
/// `int` input is accepted too, via its `str()`).
macro_rules! py_str_int {
    ($t:ty) => {
        impl PyNative for $t {
            fn to_py(self, py: Python<'_>) -> PyResult<PyObject> {
                Ok(self.to_string().into_py(py))
            }
            fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
                let text: String = obj.str()?.extract()?;
                text.parse::<$t>().map_err(|_| {
                    PyValueError::new_err(format!(
                        "{text:?} is not a valid {} (out of range or non-integer)",
                        <$t as NativeType>::NAME
                    ))
                })
            }
        }
    };
}
py_str_int!(u64);
py_str_int!(i64);
py_str_int!(u128);
py_str_int!(i128);

/// 96/256-bit integers (`u96`/`i96`/`u256`/`i256`) — no cross-language numeric form, so they
/// cross as their **little-endian bytes** (exactly `$n` of them).
macro_rules! py_wide_int {
    ($t:ty, $n:literal) => {
        impl PyNative for $t {
            fn to_py(self, py: Python<'_>) -> PyResult<PyObject> {
                Ok(PyBytes::new_bound(py, &self.to_le_bytes())
                    .into_any()
                    .unbind())
            }
            fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
                let bytes: Vec<u8> = obj.extract()?;
                let array: [u8; $n] = bytes.as_slice().try_into().map_err(|_| {
                    PyValueError::new_err(format!(
                        "{} expects exactly {} little-endian bytes, got {}",
                        <$t as NativeType>::NAME,
                        $n,
                        bytes.len()
                    ))
                })?;
                Ok(<$t>::from_le_bytes(array))
            }
        }
    };
}
py_wide_int!(U96, 12);
py_wide_int!(I96, 12);
py_wide_int!(U256, 32);
py_wide_int!(I256, 32);

/// Floats (`f16`/`f32`/`f64`) — cross as a native Python `float` (`f16` via `f32`).
impl PyNative for f64 {
    fn to_py(self, py: Python<'_>) -> PyResult<PyObject> {
        Ok(self.into_py(py))
    }
    fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        obj.extract()
    }
}
impl PyNative for f32 {
    fn to_py(self, py: Python<'_>) -> PyResult<PyObject> {
        Ok((self as f64).into_py(py))
    }
    fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(obj.extract::<f64>()? as f32)
    }
}
impl PyNative for f16 {
    fn to_py(self, py: Python<'_>) -> PyResult<PyObject> {
        Ok((self.to_f32() as f64).into_py(py))
    }
    fn from_py(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(f16::from_f32(obj.extract::<f64>()? as f32))
    }
}

/// `None` / a Python `None` → a null element; otherwise the marshaled value.
pub(crate) fn extract_option<T: PyNative>(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<T>> {
    Ok(match value {
        None => None,
        Some(value) if value.is_none() => None,
        Some(value) => Some(T::from_py(value)?),
    })
}

/// A Python iterable of value-or-`None` → optional elements (for `from_options` / the ctor).
fn extract_options<T: PyNative>(seq: &Bound<'_, PyAny>) -> PyResult<Vec<Option<T>>> {
    let mut out = Vec::new();
    for item in seq.iter()? {
        let item = item?;
        out.push(if item.is_none() {
            None
        } else {
            Some(T::from_py(&item)?)
        });
    }
    Ok(out)
}

/// A Python iterable of present values → native elements (for `from_values`).
pub(crate) fn extract_values<T: PyNative>(seq: &Bound<'_, PyAny>) -> PyResult<Vec<T>> {
    let mut out = Vec::new();
    for item in seq.iter()? {
        out.push(T::from_py(&item?)?);
    }
    Ok(out)
}

/// Generates the `Scalar` wrapper (one nullable value) for a fixed-width element type.
macro_rules! py_scalar {
    ($Scalar:ident, $Serie:ident, $t:ty, $lit:literal) => {
        #[doc = concat!("A single, nullable `", $lit, "` value.")]
        #[pyclass(module = "yggdryl.types")]
        #[derive(Clone)]
        pub struct $Scalar {
            pub(crate) inner: Scalar<$t>,
        }

        #[pymethods]
        impl $Scalar {
            /// A scalar from a value (`None`, the default, is null).
            #[new]
            #[pyo3(signature = (value = None))]
            fn new(value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
                Ok(Self {
                    inner: match extract_option::<$t>(value)? {
                        Some(value) => Scalar::of(value),
                        None => Scalar::null(),
                    },
                })
            }

            /// The null scalar.
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: Scalar::null(),
                }
            }

            /// The value, or `None` if null.
            #[getter]
            fn value(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
                self.inner.value().map(|value| value.to_py(py)).transpose()
            }

            /// Whether the scalar is null.
            #[getter]
            fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The element type's name (e.g. `"i64"`).
            #[getter]
            fn type_name(&self) -> &'static str {
                <$t as NativeType>::NAME
            }

            /// This scalar's [`DataType`].
            #[getter]
            fn data_type(&self) -> DataType {
                DataType::of(<$t as NativeType>::TYPE_ID)
            }

            /// A [`Field`] naming a column of this scalar's type.
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: &str, nullable: bool) -> Field {
                Field {
                    inner: CoreField::of(
                        name,
                        <$t as NativeType>::TYPE_ID,
                        <$t as NativeType>::WIDTH,
                        nullable,
                    ),
                }
            }

            /// This scalar broadcast to a length-1 column.
            fn to_serie(&self) -> $Serie {
                $Serie {
                    inner: self.inner.to_serie(),
                }
            }

            /// This scalar as a **binary** scalar — the value's canonical little-endian bytes (a
            /// null stays null). The universal "any → binary" bridge; reverse with
            /// `BinaryScalar.to_<type>`.
            fn to_binary(&self) -> BinaryScalar {
                BinaryScalar {
                    inner: self.inner.to_binary(),
                }
            }

            /// The scalar's canonical bytes (one validity byte then the little-endian value).
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }

            /// Reconstructs a scalar from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                Scalar::<$t>::deserialize_bytes(bytes)
                    .map(|inner| Self { inner })
                    .map_err(io_err)
            }

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
                    Some(value) => format!("{}({value:?})", stringify!($Scalar)),
                    None => format!("{}(null)", stringify!($Scalar)),
                }
            }
        }
    };
}

/// Generates the `Serie` wrapper (one nullable column) for a fixed-width element type.
macro_rules! py_serie {
    ($Scalar:ident, $Serie:ident, $t:ty, $lit:literal) => {
        #[doc = concat!("A nullable column of `", $lit, "` values.")]
        #[pyclass(module = "yggdryl.types")]
        #[derive(Clone)]
        pub struct $Serie {
            pub(crate) inner: Serie<$t>,
        }

        #[pymethods]
        impl $Serie {
            /// A column from an iterable of value-or-`None` (empty by default).
            #[new]
            #[pyo3(signature = (values = None))]
            fn new(values: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
                Ok(Self {
                    inner: match values {
                        None => Serie::new(),
                        Some(seq) => Serie::from_options(&extract_options::<$t>(seq)?),
                    },
                })
            }

            /// A non-null column from an iterable of present values.
            #[staticmethod]
            fn from_values(values: &Bound<'_, PyAny>) -> PyResult<Self> {
                Ok(Self {
                    inner: Serie::from_values(&extract_values::<$t>(values)?),
                })
            }

            /// A length-1 column broadcasting `scalar`.
            #[staticmethod]
            fn from_scalar(scalar: &$Scalar) -> Self {
                Self {
                    inner: Serie::from_scalar(scalar.inner),
                }
            }

            /// A column from a list of this type's scalars — each item is a `$Scalar` (or `None`, a
            /// null element). The inverse of `get_scalar` over the whole column.
            #[staticmethod]
            fn from_scalars(scalars: &Bound<'_, PyAny>) -> PyResult<Self> {
                let mut inners = Vec::new();
                for item in scalars.iter()? {
                    let item = item?;
                    inners.push(if item.is_none() {
                        Scalar::null()
                    } else {
                        item.extract::<$Scalar>()?.inner
                    });
                }
                Ok(Self {
                    inner: Serie::from_scalars(&inners),
                })
            }

            /// Appends one element (`None` is a null).
            #[pyo3(signature = (value = None))]
            fn push(&mut self, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
                self.inner.push(extract_option::<$t>(value)?);
                Ok(())
            }

            /// The element at `index`, or `None` if it is null or out of range.
            fn get(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyObject>> {
                self.inner
                    .get(index)
                    .map(|value| value.to_py(py))
                    .transpose()
            }

            /// The element at `index` as a scalar (null if null or out of range).
            fn get_scalar(&self, index: usize) -> $Scalar {
                $Scalar {
                    inner: self.inner.get_scalar(index),
                }
            }

            /// This column as a single scalar, if it holds exactly one element.
            fn as_scalar(&self) -> Option<$Scalar> {
                self.inner.as_scalar().map(|inner| $Scalar { inner })
            }

            /// Overwrites element `index` (`None` writes a null); raises `ValueError` if out of range.
            #[pyo3(signature = (index, value = None))]
            fn set(&mut self, index: usize, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
                self.inner
                    .set(index, extract_option::<$t>(value)?)
                    .map_err(io_err)
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

            /// The elements as a list of value-or-`None`, in order.
            fn to_options(&self, py: Python<'_>) -> PyResult<Vec<Option<PyObject>>> {
                self.inner
                    .to_options()
                    .into_iter()
                    .map(|value| value.map(|value| value.to_py(py)).transpose())
                    .collect()
            }

            /// This column's [`DataType`].
            #[getter]
            fn data_type(&self) -> DataType {
                DataType::of(<$t as NativeType>::TYPE_ID)
            }

            /// A [`Field`] naming this column with explicit nullability.
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: &str, nullable: bool) -> Field {
                Field {
                    inner: CoreField::of(
                        name,
                        <$t as NativeType>::TYPE_ID,
                        <$t as NativeType>::WIDTH,
                        nullable,
                    ),
                }
            }

            /// A [`Field`] naming this column, nullability **inferred** from whether it holds nulls.
            fn to_field(&self, name: &str) -> Field {
                Field {
                    inner: CoreField::of(
                        name,
                        <$t as NativeType>::TYPE_ID,
                        <$t as NativeType>::WIDTH,
                        self.inner.has_nulls(),
                    ),
                }
            }

            /// The column's canonical bytes (`[len][flags][validity?][values]`).
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }

            /// Reconstructs a column from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                Serie::<$t>::deserialize_bytes(bytes)
                    .map(|inner| Self { inner })
                    .map_err(io_err)
            }

            fn __len__(&self) -> usize {
                self.inner.len()
            }

            fn __bool__(&self) -> bool {
                !self.inner.is_empty()
            }

            /// Random access — `col[i]` returns the value or `None` (negative indices allowed);
            /// raises `IndexError` out of range.
            fn __getitem__(&self, py: Python<'_>, index: isize) -> PyResult<Option<PyObject>> {
                let len = self.inner.len() as isize;
                let resolved = if index < 0 { index + len } else { index };
                if resolved < 0 || resolved >= len {
                    return Err(PyIndexError::new_err("Serie index out of range"));
                }
                self.inner
                    .get(resolved as usize)
                    .map(|value| value.to_py(py))
                    .transpose()
            }

            /// Iterates the elements as value-or-`None`, in order.
            fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                Ok(PyList::new_bound(py, self.to_options(py)?)
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
                    "{}(len={}, null_count={})",
                    stringify!($Serie),
                    self.inner.len(),
                    self.inner.null_count()
                )
            }
        }
    };
}

/// Declares the `Scalar` **and** `Serie` wrapper for one fixed-width element type.
macro_rules! py_fixed {
    ($Scalar:ident, $Serie:ident, $t:ty, $lit:literal) => {
        py_scalar!($Scalar, $Serie, $t, $lit);
        py_serie!($Scalar, $Serie, $t, $lit);
    };
}

py_fixed!(U8Scalar, U8Serie, u8, "u8");
py_fixed!(U16Scalar, U16Serie, u16, "u16");
py_fixed!(U32Scalar, U32Serie, u32, "u32");
py_fixed!(U64Scalar, U64Serie, u64, "u64");
py_fixed!(U96Scalar, U96Serie, U96, "u96");
py_fixed!(U128Scalar, U128Serie, u128, "u128");
py_fixed!(U256Scalar, U256Serie, U256, "u256");
py_fixed!(I8Scalar, I8Serie, i8, "i8");
py_fixed!(I16Scalar, I16Serie, i16, "i16");
py_fixed!(I32Scalar, I32Serie, i32, "i32");
py_fixed!(I64Scalar, I64Serie, i64, "i64");
py_fixed!(I96Scalar, I96Serie, I96, "i96");
py_fixed!(I128Scalar, I128Serie, i128, "i128");
py_fixed!(I256Scalar, I256Serie, I256, "i256");
py_fixed!(F16Scalar, F16Serie, f16, "f16");
py_fixed!(F32Scalar, F32Serie, f32, "f32");
py_fixed!(F64Scalar, F64Serie, f64, "f64");

/// Adds the numeric `to_<type>` casts to one castable `Scalar` **and** `Serie` (the
/// [`NumericCast`](yggdryl_core::io::NumericCast) subset — `u8`…`u64`, `i8`…`i128`, the floats).
/// Each cast is range-checked for an integer target (a guided `ValueError`) and precision-lossy
/// for a float; a null casts to a null of the target. The scalar also gets the universal
/// `to_utf8` bridge (`to_binary` is on every scalar).
macro_rules! py_numeric_casts {
    ($Scalar:ident, $Serie:ident, $t:ty) => {
        #[pymethods]
        impl $Scalar {
            /// This scalar cast to `u8` (range-checked; null stays null).
            fn to_u8(&self) -> PyResult<U8Scalar> {
                self.inner
                    .cast::<u8>()
                    .map(|inner| U8Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `u16`.
            fn to_u16(&self) -> PyResult<U16Scalar> {
                self.inner
                    .cast::<u16>()
                    .map(|inner| U16Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `u32`.
            fn to_u32(&self) -> PyResult<U32Scalar> {
                self.inner
                    .cast::<u32>()
                    .map(|inner| U32Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `u64`.
            fn to_u64(&self) -> PyResult<U64Scalar> {
                self.inner
                    .cast::<u64>()
                    .map(|inner| U64Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `i8`.
            fn to_i8(&self) -> PyResult<I8Scalar> {
                self.inner
                    .cast::<i8>()
                    .map(|inner| I8Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `i16`.
            fn to_i16(&self) -> PyResult<I16Scalar> {
                self.inner
                    .cast::<i16>()
                    .map(|inner| I16Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `i32`.
            fn to_i32(&self) -> PyResult<I32Scalar> {
                self.inner
                    .cast::<i32>()
                    .map(|inner| I32Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `i64`.
            fn to_i64(&self) -> PyResult<I64Scalar> {
                self.inner
                    .cast::<i64>()
                    .map(|inner| I64Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `i128`.
            fn to_i128(&self) -> PyResult<I128Scalar> {
                self.inner
                    .cast::<i128>()
                    .map(|inner| I128Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `f16` (precision-lossy).
            fn to_f16(&self) -> PyResult<F16Scalar> {
                self.inner
                    .cast::<f16>()
                    .map(|inner| F16Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `f32`.
            fn to_f32(&self) -> PyResult<F32Scalar> {
                self.inner
                    .cast::<f32>()
                    .map(|inner| F32Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar cast to `f64`.
            fn to_f64(&self) -> PyResult<F64Scalar> {
                self.inner
                    .cast::<f64>()
                    .map(|inner| F64Scalar { inner })
                    .map_err(cast_err)
            }
            /// This scalar as a **UTF-8** scalar — the value's decimal text (a null stays null).
            fn to_utf8(&self) -> Utf8Scalar {
                Utf8Scalar {
                    inner: self.inner.to_utf8(),
                }
            }
        }

        #[pymethods]
        impl $Serie {
            /// This column cast to `u8` element-for-element (nulls preserved; range-checked).
            fn to_u8(&self) -> PyResult<U8Serie> {
                self.inner
                    .cast::<u8>()
                    .map(|inner| U8Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `u16`.
            fn to_u16(&self) -> PyResult<U16Serie> {
                self.inner
                    .cast::<u16>()
                    .map(|inner| U16Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `u32`.
            fn to_u32(&self) -> PyResult<U32Serie> {
                self.inner
                    .cast::<u32>()
                    .map(|inner| U32Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `u64`.
            fn to_u64(&self) -> PyResult<U64Serie> {
                self.inner
                    .cast::<u64>()
                    .map(|inner| U64Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `i8`.
            fn to_i8(&self) -> PyResult<I8Serie> {
                self.inner
                    .cast::<i8>()
                    .map(|inner| I8Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `i16`.
            fn to_i16(&self) -> PyResult<I16Serie> {
                self.inner
                    .cast::<i16>()
                    .map(|inner| I16Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `i32`.
            fn to_i32(&self) -> PyResult<I32Serie> {
                self.inner
                    .cast::<i32>()
                    .map(|inner| I32Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `i64`.
            fn to_i64(&self) -> PyResult<I64Serie> {
                self.inner
                    .cast::<i64>()
                    .map(|inner| I64Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `i128`.
            fn to_i128(&self) -> PyResult<I128Serie> {
                self.inner
                    .cast::<i128>()
                    .map(|inner| I128Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `f16` (precision-lossy).
            fn to_f16(&self) -> PyResult<F16Serie> {
                self.inner
                    .cast::<f16>()
                    .map(|inner| F16Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `f32`.
            fn to_f32(&self) -> PyResult<F32Serie> {
                self.inner
                    .cast::<f32>()
                    .map(|inner| F32Serie { inner })
                    .map_err(cast_err)
            }
            /// This column cast to `f64`.
            fn to_f64(&self) -> PyResult<F64Serie> {
                self.inner
                    .cast::<f64>()
                    .map(|inner| F64Serie { inner })
                    .map_err(cast_err)
            }
        }
    };
}

py_numeric_casts!(U8Scalar, U8Serie, u8);
py_numeric_casts!(U16Scalar, U16Serie, u16);
py_numeric_casts!(U32Scalar, U32Serie, u32);
py_numeric_casts!(U64Scalar, U64Serie, u64);
py_numeric_casts!(I8Scalar, I8Serie, i8);
py_numeric_casts!(I16Scalar, I16Serie, i16);
py_numeric_casts!(I32Scalar, I32Serie, i32);
py_numeric_casts!(I64Scalar, I64Serie, i64);
py_numeric_casts!(I128Scalar, I128Serie, i128);
py_numeric_casts!(F16Scalar, F16Serie, f16);
py_numeric_casts!(F32Scalar, F32Serie, f32);
py_numeric_casts!(F64Scalar, F64Serie, f64);

/// Adds every fixed-width `Scalar` / `Serie` class to the `yggdryl.types` submodule.
macro_rules! register_all {
    ($module:ident, $($Scalar:ident, $Serie:ident);* $(;)?) => {
        $(
            $module.add_class::<$Scalar>()?;
            $module.add_class::<$Serie>()?;
        )*
    };
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    register_all!(module,
        U8Scalar, U8Serie;
        U16Scalar, U16Serie;
        U32Scalar, U32Serie;
        U64Scalar, U64Serie;
        U96Scalar, U96Serie;
        U128Scalar, U128Serie;
        U256Scalar, U256Serie;
        I8Scalar, I8Serie;
        I16Scalar, I16Serie;
        I32Scalar, I32Serie;
        I64Scalar, I64Serie;
        I96Scalar, I96Serie;
        I128Scalar, I128Serie;
        I256Scalar, I256Serie;
        F16Scalar, F16Serie;
        F32Scalar, F32Serie;
        F64Scalar, F64Serie;
    );
    Ok(())
}
