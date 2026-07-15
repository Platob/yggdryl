//! The `yggdryl.types` submodule's **variable-length value layer** — one nullable value
//! (`Utf8Scalar` / `BinaryScalar`) and one nullable column (`Utf8Serie` / `BinarySerie`) per
//! variable-length kind, mirroring `yggdryl_core::io::var`'s generic `ByteScalar<E>` /
//! `ByteSerie<E>`.
//!
//! A **UTF-8** value crosses as `str`; a **binary** value as `bytes`. A `Utf8` value is validated
//! (a bad decode raises `ValueError`); binary accepts any bytes. A `Scalar` is an immutable value
//! (hashable, pickles through its byte codec); a `Serie` is a mutable column (unhashable, like
//! `bytearray`) whose per-element `set` may rewrite trailing offsets. `None` is a null throughout.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList};

use yggdryl_core::io::fixed::f16;
use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::var::{Binary, ByteScalar, ByteSerie, Utf8, VarElement};
use yggdryl_core::io::{CastError, IoError};

use crate::types::{DataType, Field};
use crate::values::{
    F16Scalar, F32Scalar, F64Scalar, I128Scalar, I16Scalar, I32Scalar, I64Scalar, I8Scalar,
    U16Scalar, U32Scalar, U64Scalar, U8Scalar,
};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn io_err(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Maps a [`CastError`] to a Python `ValueError` carrying its guided text.
fn cast_err(error: CastError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Cross-FFI marshaling for one variable-length kind: its Python value type (`str` for UTF-8,
/// `bytes` for binary) to and from the raw bytes the core stores.
pub(crate) trait PyVarKind: VarElement {
    /// The stored bytes as the Python value (`str` / `bytes`).
    fn bytes_to_py(bytes: &[u8], py: Python<'_>) -> PyObject;
    /// A Python value (`str` / bytes-like) as raw bytes.
    fn py_to_bytes(obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>>;
    /// The value's `repr` fragment (`"abc"` for UTF-8, `b'\\x01'` for binary).
    fn value_repr(bytes: &[u8]) -> String;
}

impl PyVarKind for Utf8 {
    fn bytes_to_py(bytes: &[u8], py: Python<'_>) -> PyObject {
        // The bytes entered through a validated path, so they are always valid UTF-8.
        std::str::from_utf8(bytes).unwrap_or_default().into_py(py)
    }
    fn py_to_bytes(obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
        Ok(obj.extract::<String>()?.into_bytes())
    }
    fn value_repr(bytes: &[u8]) -> String {
        format!("{:?}", std::str::from_utf8(bytes).unwrap_or_default())
    }
}

impl PyVarKind for Binary {
    fn bytes_to_py(bytes: &[u8], py: Python<'_>) -> PyObject {
        PyBytes::new_bound(py, bytes).into_any().unbind()
    }
    fn py_to_bytes(obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
        obj.extract()
    }
    fn value_repr(bytes: &[u8]) -> String {
        PyBytesRepr(bytes).to_string()
    }
}

/// A `bytes`-literal `Debug` (`b'\x01\xff'`) for a binary value's `repr`.
struct PyBytesRepr<'a>(&'a [u8]);
impl std::fmt::Display for PyBytesRepr<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "b'")?;
        for &byte in self.0 {
            match byte {
                b'\\' => write!(f, "\\\\")?,
                b'\'' => write!(f, "\\'")?,
                0x20..=0x7e => write!(f, "{}", byte as char)?,
                _ => write!(f, "\\x{byte:02x}")?,
            }
        }
        write!(f, "'")
    }
}

/// `None` / a Python `None` → a null element; otherwise the value's raw bytes.
fn extract_var_option<E: PyVarKind>(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<Vec<u8>>> {
    Ok(match value {
        None => None,
        Some(value) if value.is_none() => None,
        Some(value) => Some(E::py_to_bytes(value)?),
    })
}

/// A Python iterable of value-or-`None` → optional byte values (for the `Serie` ctor).
fn extract_var_options<E: PyVarKind>(seq: &Bound<'_, PyAny>) -> PyResult<Vec<Option<Vec<u8>>>> {
    let mut out = Vec::new();
    for item in seq.iter()? {
        let item = item?;
        out.push(if item.is_none() {
            None
        } else {
            Some(E::py_to_bytes(&item)?)
        });
    }
    Ok(out)
}

/// The `(id, byte_width)` a variable-length field of kind `E` carries.
fn var_field<E: VarElement>(name: &str, nullable: bool) -> Field {
    let id = <E as VarElement>::TYPE_ID;
    Field {
        inner: CoreField::of(name, id, id.fixed_byte_width().unwrap_or(0), nullable),
    }
}

/// Generates the `Scalar` **and** `Serie` wrappers for one variable-length kind.
macro_rules! py_var {
    ($Scalar:ident, $Serie:ident, $E:ty, $lit:literal) => {
        #[doc = concat!("A single, nullable `", $lit, "` value.")]
        #[pyclass(module = "yggdryl.types")]
        #[derive(Clone)]
        pub struct $Scalar {
            pub(crate) inner: ByteScalar<$E>,
        }

        #[pymethods]
        impl $Scalar {
            /// A scalar from a value (`None`, the default, is null).
            #[new]
            #[pyo3(signature = (value = None))]
            fn new(value: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
                let bytes = extract_var_option::<$E>(value)?;
                ByteScalar::<$E>::new(bytes.as_deref())
                    .map(|inner| Self { inner })
                    .map_err(io_err)
            }

            /// The null scalar.
            #[staticmethod]
            fn null() -> Self {
                Self {
                    inner: ByteScalar::null(),
                }
            }

            /// The value, or `None` if null.
            #[getter]
            fn value(&self, py: Python<'_>) -> Option<PyObject> {
                self.inner
                    .value_bytes()
                    .map(|bytes| <$E as PyVarKind>::bytes_to_py(bytes, py))
            }

            /// Whether the scalar is null.
            #[getter]
            fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The element type's name (`"utf8"` / `"binary"`).
            #[getter]
            fn type_name(&self) -> &'static str {
                <$E as VarElement>::NAME
            }

            /// This scalar's [`DataType`].
            #[getter]
            fn data_type(&self) -> DataType {
                DataType::of(<$E as VarElement>::TYPE_ID)
            }

            /// A [`Field`] naming a column of this scalar's type.
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: &str, nullable: bool) -> Field {
                var_field::<$E>(name, nullable)
            }

            /// The scalar's canonical bytes (validity byte, then `[len][bytes]` if present).
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }

            /// Reconstructs a scalar from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                ByteScalar::<$E>::deserialize_bytes(bytes)
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
                match self.inner.value_bytes() {
                    Some(bytes) => {
                        format!(
                            "{}({})",
                            stringify!($Scalar),
                            <$E as PyVarKind>::value_repr(bytes)
                        )
                    }
                    None => format!("{}(null)", stringify!($Scalar)),
                }
            }
        }

        #[doc = concat!("A nullable column of `", $lit, "` values.")]
        #[pyclass(module = "yggdryl.types")]
        #[derive(Clone)]
        pub struct $Serie {
            pub(crate) inner: ByteSerie<$E>,
        }

        #[pymethods]
        impl $Serie {
            /// A column from an iterable of value-or-`None` (empty by default).
            #[new]
            #[pyo3(signature = (values = None))]
            fn new(values: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
                match values {
                    None => Ok(Self {
                        inner: ByteSerie::new(),
                    }),
                    Some(seq) => {
                        let owned = extract_var_options::<$E>(seq)?;
                        let refs: Vec<Option<&[u8]>> = owned.iter().map(|o| o.as_deref()).collect();
                        ByteSerie::<$E>::from_byte_values(&refs)
                            .map(|inner| Self { inner })
                            .map_err(io_err)
                    }
                }
            }

            /// Appends one element (`None` is a null).
            #[pyo3(signature = (value = None))]
            fn push(&mut self, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
                let bytes = extract_var_option::<$E>(value)?;
                self.inner.push_bytes(bytes.as_deref()).map_err(io_err)
            }

            /// The element at `index`, or `None` if it is null or out of range.
            fn get(&self, py: Python<'_>, index: usize) -> Option<PyObject> {
                self.inner
                    .get_bytes(index)
                    .map(|bytes| <$E as PyVarKind>::bytes_to_py(bytes, py))
            }

            /// The element at `index` as a scalar (null if null or out of range).
            fn get_scalar(&self, index: usize) -> $Scalar {
                $Scalar {
                    inner: self.inner.get_scalar(index),
                }
            }

            /// Overwrites element `index` (`None` writes a null); a length change rewrites the
            /// trailing offsets. Raises `ValueError` if out of range.
            #[pyo3(signature = (index, value = None))]
            fn set(&mut self, index: usize, value: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
                let bytes = extract_var_option::<$E>(value)?;
                self.inner
                    .set_bytes(index, bytes.as_deref())
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

            /// The total number of value bytes (excluding offsets / validity).
            #[getter]
            fn data_len(&self) -> usize {
                self.inner.data_len()
            }

            /// The elements as a list of value-or-`None`, in order.
            fn to_options(&self, py: Python<'_>) -> Vec<Option<PyObject>> {
                (0..self.inner.len())
                    .map(|index| {
                        self.inner
                            .get_bytes(index)
                            .map(|bytes| <$E as PyVarKind>::bytes_to_py(bytes, py))
                    })
                    .collect()
            }

            /// This column's [`DataType`].
            #[getter]
            fn data_type(&self) -> DataType {
                DataType::of(<$E as VarElement>::TYPE_ID)
            }

            /// A [`Field`] naming this column with explicit nullability.
            #[pyo3(signature = (name, nullable = true))]
            fn field(&self, name: &str, nullable: bool) -> Field {
                var_field::<$E>(name, nullable)
            }

            /// A [`Field`] naming this column, nullability **inferred** from whether it holds nulls.
            fn to_field(&self, name: &str) -> Field {
                var_field::<$E>(name, self.inner.has_nulls())
            }

            /// The column's canonical bytes (`[len][flags][validity?][offsets][data_len][data]`).
            fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, &self.inner.serialize_bytes())
            }

            /// Reconstructs a column from [`serialize_bytes`](Self::serialize_bytes).
            #[staticmethod]
            fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                ByteSerie::<$E>::deserialize_bytes(bytes)
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
                Ok(self
                    .inner
                    .get_bytes(resolved as usize)
                    .map(|bytes| <$E as PyVarKind>::bytes_to_py(bytes, py)))
            }

            /// Iterates the elements as value-or-`None`, in order.
            fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                Ok(PyList::new_bound(py, self.to_options(py))
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

py_var!(Utf8Scalar, Utf8Serie, Utf8, "utf8");
py_var!(BinaryScalar, BinarySerie, Binary, "binary");

/// The reverse of the fixed scalars' `to_utf8` bridge — parse this UTF-8 scalar's text into a
/// numeric scalar (a null stays null; a bad parse is a guided `ValueError`).
#[pymethods]
impl Utf8Scalar {
    /// Parse this text into a `u8` scalar.
    fn to_u8(&self) -> PyResult<U8Scalar> {
        self.inner
            .parse_to::<u8>()
            .map(|inner| U8Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into a `u16` scalar.
    fn to_u16(&self) -> PyResult<U16Scalar> {
        self.inner
            .parse_to::<u16>()
            .map(|inner| U16Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into a `u32` scalar.
    fn to_u32(&self) -> PyResult<U32Scalar> {
        self.inner
            .parse_to::<u32>()
            .map(|inner| U32Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into a `u64` scalar.
    fn to_u64(&self) -> PyResult<U64Scalar> {
        self.inner
            .parse_to::<u64>()
            .map(|inner| U64Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into an `i8` scalar.
    fn to_i8(&self) -> PyResult<I8Scalar> {
        self.inner
            .parse_to::<i8>()
            .map(|inner| I8Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into an `i16` scalar.
    fn to_i16(&self) -> PyResult<I16Scalar> {
        self.inner
            .parse_to::<i16>()
            .map(|inner| I16Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into an `i32` scalar.
    fn to_i32(&self) -> PyResult<I32Scalar> {
        self.inner
            .parse_to::<i32>()
            .map(|inner| I32Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into an `i64` scalar.
    fn to_i64(&self) -> PyResult<I64Scalar> {
        self.inner
            .parse_to::<i64>()
            .map(|inner| I64Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into an `i128` scalar.
    fn to_i128(&self) -> PyResult<I128Scalar> {
        self.inner
            .parse_to::<i128>()
            .map(|inner| I128Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into an `f16` scalar.
    fn to_f16(&self) -> PyResult<F16Scalar> {
        self.inner
            .parse_to::<f16>()
            .map(|inner| F16Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into an `f32` scalar.
    fn to_f32(&self) -> PyResult<F32Scalar> {
        self.inner
            .parse_to::<f32>()
            .map(|inner| F32Scalar { inner })
            .map_err(cast_err)
    }
    /// Parse this text into an `f64` scalar.
    fn to_f64(&self) -> PyResult<F64Scalar> {
        self.inner
            .parse_to::<f64>()
            .map(|inner| F64Scalar { inner })
            .map_err(cast_err)
    }
}

/// The reverse of the fixed scalars' `to_binary` bridge — read this binary scalar's little-endian
/// bytes back into a numeric scalar (a null stays null; a width mismatch is a guided `ValueError`).
#[pymethods]
impl BinaryScalar {
    /// Read these bytes as a `u8` scalar.
    fn to_u8(&self) -> PyResult<U8Scalar> {
        self.inner
            .read_to::<u8>()
            .map(|inner| U8Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as a `u16` scalar.
    fn to_u16(&self) -> PyResult<U16Scalar> {
        self.inner
            .read_to::<u16>()
            .map(|inner| U16Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as a `u32` scalar.
    fn to_u32(&self) -> PyResult<U32Scalar> {
        self.inner
            .read_to::<u32>()
            .map(|inner| U32Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as a `u64` scalar.
    fn to_u64(&self) -> PyResult<U64Scalar> {
        self.inner
            .read_to::<u64>()
            .map(|inner| U64Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as an `i8` scalar.
    fn to_i8(&self) -> PyResult<I8Scalar> {
        self.inner
            .read_to::<i8>()
            .map(|inner| I8Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as an `i16` scalar.
    fn to_i16(&self) -> PyResult<I16Scalar> {
        self.inner
            .read_to::<i16>()
            .map(|inner| I16Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as an `i32` scalar.
    fn to_i32(&self) -> PyResult<I32Scalar> {
        self.inner
            .read_to::<i32>()
            .map(|inner| I32Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as an `i64` scalar.
    fn to_i64(&self) -> PyResult<I64Scalar> {
        self.inner
            .read_to::<i64>()
            .map(|inner| I64Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as an `i128` scalar.
    fn to_i128(&self) -> PyResult<I128Scalar> {
        self.inner
            .read_to::<i128>()
            .map(|inner| I128Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as an `f16` scalar.
    fn to_f16(&self) -> PyResult<F16Scalar> {
        self.inner
            .read_to::<f16>()
            .map(|inner| F16Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as an `f32` scalar.
    fn to_f32(&self) -> PyResult<F32Scalar> {
        self.inner
            .read_to::<f32>()
            .map(|inner| F32Scalar { inner })
            .map_err(cast_err)
    }
    /// Read these bytes as an `f64` scalar.
    fn to_f64(&self) -> PyResult<F64Scalar> {
        self.inner
            .read_to::<f64>()
            .map(|inner| F64Scalar { inner })
            .map_err(cast_err)
    }
}

/// Adds the variable-length `Scalar` / `Serie` classes to the `yggdryl.types` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Utf8Scalar>()?;
    module.add_class::<Utf8Serie>()?;
    module.add_class::<BinaryScalar>()?;
    module.add_class::<BinarySerie>()?;
    Ok(())
}
