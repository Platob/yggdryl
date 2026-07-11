//! The `yggdryl.dtype` submodule — Arrow primitive data types.
//!
//! Exposes one class per primitive data type (`I8Type` … `F64Type`, plus the
//! bit-packed `BooleanType` and the sui-generis `NullType`), mirroring `yggdryl_dtype`.
//! Each carries the type-identity
//! surface — `name`, `byte_width`, `primitive_tag`, the byte codec
//! (`serialize_bytes` / `deserialize_bytes`), value semantics (`==` / `hash` / pickle),
//! and `repr`. The Arrow `to_arrow` / `from_arrow` interop is **Rust-only** (an
//! `arrow_schema` value does not cross the FFI boundary), exactly as for the buffers'
//! Arrow interop.

// The `#[pymethods]` macro emits identity `.into()` conversions on `PyResult`
// returns that clippy flags as useless; silence it at module scope.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl_dtype::DataType;

/// Maps a [`yggdryl_dtype::DTypeError`] to a Python `ValueError`.
fn dtype_err(error: yggdryl_dtype::DTypeError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Generates the pyo3 wrapper class for one primitive data type. `$tag` is the core
/// [`PrimitiveType`](yggdryl_converter::PrimitiveType) tag name (e.g. `Some("i64")`), or
/// `None` for `Boolean`.
macro_rules! py_primitive_dtype {
    ($( ($name:ident, $lit:literal, $tag:expr) ),+ $(,)?) => {
        $(
            #[doc = concat!("The `", $lit, "` primitive data type.")]
            #[pyclass(module = "yggdryl.dtype")]
            #[derive(Clone)]
            pub struct $name {
                pub(crate) inner: yggdryl_dtype::$name,
            }

            #[pymethods]
            impl $name {
                #[new]
                fn new() -> Self {
                    Self { inner: yggdryl_dtype::$name::new() }
                }

                /// The canonical lower-snake type name, e.g. `"int64"`.
                #[getter]
                fn name(&self) -> &'static str {
                    self.inner.name()
                }

                /// The fixed value width in bytes, or `None` for `boolean` (bit-packed).
                #[getter]
                fn byte_width(&self) -> Option<usize> {
                    self.inner.byte_width()
                }

                /// The core `PrimitiveType` tag name (e.g. `"i64"`), or `None` for
                /// `boolean`.
                #[getter]
                fn primitive_tag(&self) -> Option<&'static str> {
                    $tag
                }

                /// The type's (empty) serialised payload.
                fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
                    PyBytes::new_bound(py, &self.inner.serialize_bytes())
                }

                /// Reconstructs the type from its serialised payload (must be empty).
                #[staticmethod]
                fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
                    yggdryl_dtype::$name::deserialize_bytes(bytes)
                        .map(|inner| Self { inner })
                        .map_err(dtype_err)
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

                fn __repr__(&self) -> String {
                    concat!(stringify!($name), "()").to_string()
                }
            }
        )+
    };
}

py_primitive_dtype! {
    (I8Type, "int8", Some("i8")),
    (I16Type, "int16", Some("i16")),
    (I32Type, "int32", Some("i32")),
    (I64Type, "int64", Some("i64")),
    (U8Type, "uint8", Some("u8")),
    (U16Type, "uint16", Some("u16")),
    (U32Type, "uint32", Some("u32")),
    (U64Type, "uint64", Some("u64")),
    (F32Type, "float32", Some("f32")),
    (F64Type, "float64", Some("f64")),
    (BooleanType, "boolean", None),
    (NullType, "null", None),
}

/// Populates the `dtype` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<I8Type>()?;
    module.add_class::<I16Type>()?;
    module.add_class::<I32Type>()?;
    module.add_class::<I64Type>()?;
    module.add_class::<U8Type>()?;
    module.add_class::<U16Type>()?;
    module.add_class::<U32Type>()?;
    module.add_class::<U64Type>()?;
    module.add_class::<F32Type>()?;
    module.add_class::<F64Type>()?;
    module.add_class::<BooleanType>()?;
    module.add_class::<NullType>()?;
    Ok(())
}
