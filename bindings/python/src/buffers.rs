//! The `yggdryl.types` submodule's **typed buffers** — a contiguous, growable store of one
//! fixed-width primitive: `U8Buffer` … `I256Buffer`, `F16Buffer` … `F64Buffer`, mirroring
//! `yggdryl_core::io::fixed`'s `Buffer<T>`.
//!
//! A `Buffer` is the raw, **non-nullable** values store the columnar [`Serie`](crate::values) sits
//! on (for nullable columns use a `Serie`; for byte I/O with a cursor use
//! [`Bytes`](crate::bytes)). Values marshal exactly like the scalars: small ints as `int`, wide
//! ints as decimal strings, the 96/256-bit ints as little-endian `bytes`, floats as `float`. The
//! raw little-endian element bytes are the byte codec (`to_bytes` / `from_bytes`).

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyList};

use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::fixed::{f16, Buffer, NativeType, I256, I96, U256, U96};

use crate::types::{DataType, Field};
use crate::values::{extract_values, PyNative};

/// Generates the typed `Buffer` wrapper for one fixed-width element type.
macro_rules! py_buffer {
    ($Buffer:ident, $t:ty, $lit:literal) => {
        #[doc = concat!("A contiguous, non-nullable buffer of `", $lit, "` values.")]
        #[pyclass(module = "yggdryl.types")]
        #[derive(Clone)]
        pub struct $Buffer {
            pub(crate) inner: Buffer<$t>,
        }

        #[pymethods]
        impl $Buffer {
            /// A buffer from an iterable of present values (empty by default).
            #[new]
            #[pyo3(signature = (values = None))]
            fn new(values: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
                Ok(Self {
                    inner: match values {
                        None => Buffer::new(),
                        Some(seq) => Buffer::from_slice(&extract_values::<$t>(seq)?),
                    },
                })
            }

            /// A buffer wrapping raw little-endian element `bytes` (the inverse of `to_bytes`).
            #[staticmethod]
            fn from_bytes(bytes: &[u8]) -> Self {
                Self {
                    inner: Buffer::from_bytes(bytes),
                }
            }

            /// The number of elements.
            #[getter]
            fn count(&self) -> usize {
                self.inner.count()
            }

            /// The element at `index`, or `None` if out of range.
            fn get(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyObject>> {
                self.inner
                    .get(index)
                    .map(|value| value.to_py(py))
                    .transpose()
            }

            /// Overwrites element `index`; raises `IndexError` out of range.
            fn set(&mut self, index: usize, value: &Bound<'_, PyAny>) -> PyResult<()> {
                if index >= self.inner.count() {
                    return Err(PyIndexError::new_err("Buffer index out of range"));
                }
                self.inner.set(index, <$t as PyNative>::from_py(value)?);
                Ok(())
            }

            /// Appends one element, growing the buffer.
            fn push(&mut self, value: &Bound<'_, PyAny>) -> PyResult<()> {
                self.inner.push(<$t as PyNative>::from_py(value)?);
                Ok(())
            }

            /// The elements as a list, in order.
            fn to_values(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
                (0..self.inner.count())
                    .map(|index| self.inner.get(index).expect("index < count").to_py(py))
                    .collect()
            }

            /// The raw little-endian element bytes (one copy) — the inverse of `from_bytes`.
            fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                PyBytes::new_bound(py, self.inner.as_bytes())
            }

            /// This buffer's [`DataType`].
            #[getter]
            fn data_type(&self) -> DataType {
                DataType::of(<$t as NativeType>::TYPE_ID)
            }

            /// A [`Field`] naming a column of this buffer's element type.
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

            fn __len__(&self) -> usize {
                self.inner.count()
            }

            fn __bool__(&self) -> bool {
                self.inner.count() != 0
            }

            fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                self.to_bytes(py)
            }

            /// Random access — `buf[i]` returns the value (negative indices allowed); raises
            /// `IndexError` out of range.
            fn __getitem__(&self, py: Python<'_>, index: isize) -> PyResult<PyObject> {
                let len = self.inner.count() as isize;
                let resolved = if index < 0 { index + len } else { index };
                if resolved < 0 || resolved >= len {
                    return Err(PyIndexError::new_err("Buffer index out of range"));
                }
                self.inner
                    .get(resolved as usize)
                    .expect("index in range")
                    .to_py(py)
            }

            /// Iterates the elements, in order.
            fn __iter__(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
                Ok(PyList::new_bound(py, self.to_values(py)?)
                    .call_method0("__iter__")?
                    .unbind())
            }

            /// Content equality (the raw bytes).
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

            fn __repr__(&self) -> String {
                format!("{}(count={})", stringify!($Buffer), self.inner.count())
            }
        }
    };
}

py_buffer!(U8Buffer, u8, "u8");
py_buffer!(U16Buffer, u16, "u16");
py_buffer!(U32Buffer, u32, "u32");
py_buffer!(U64Buffer, u64, "u64");
py_buffer!(U96Buffer, U96, "u96");
py_buffer!(U128Buffer, u128, "u128");
py_buffer!(U256Buffer, U256, "u256");
py_buffer!(I8Buffer, i8, "i8");
py_buffer!(I16Buffer, i16, "i16");
py_buffer!(I32Buffer, i32, "i32");
py_buffer!(I64Buffer, i64, "i64");
py_buffer!(I96Buffer, I96, "i96");
py_buffer!(I128Buffer, i128, "i128");
py_buffer!(I256Buffer, I256, "i256");
py_buffer!(F16Buffer, f16, "f16");
py_buffer!(F32Buffer, f32, "f32");
py_buffer!(F64Buffer, f64, "f64");

/// Adds every typed `Buffer` class to the `yggdryl.types` submodule.
macro_rules! register_buffers {
    ($module:ident, $($Buffer:ident),* $(,)?) => {
        $( $module.add_class::<$Buffer>()?; )*
    };
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    register_buffers!(
        module, U8Buffer, U16Buffer, U32Buffer, U64Buffer, U96Buffer, U128Buffer, U256Buffer,
        I8Buffer, I16Buffer, I32Buffer, I64Buffer, I96Buffer, I128Buffer, I256Buffer, F16Buffer,
        F32Buffer, F64Buffer,
    );
    Ok(())
}
