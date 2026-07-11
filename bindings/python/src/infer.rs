//! The `yggdryl.infer` submodule — runtime type inference for the interpreted API.
//!
//! A convenience layer, **binding-only by design** (`CLAUDE.md` rule 13): it has no
//! `yggdryl-core` counterpart because the Rust core reaches its typed buffers through
//! explicit generics, while the dynamically-typed Python API can read the runtime type
//! of a value and pick the matching buffer for the caller. Everything here is sugar
//! over the explicit constructors in [`yggdryl.buffer`](crate::buffer) — reach for
//! those directly when a value is ambiguous (e.g. forcing `int32`) or out of range.
//!
//! `buffer(values)` maps a Python value to a buffer as follows — identical to the
//! Node binding's mapping:
//!
//! | Python value                     | Result buffer   |
//! |----------------------------------|-----------------|
//! | `bytes` / `bytearray`            | `U8Buffer`      |
//! | sequence of `bool`               | `BooleanBuffer` |
//! | sequence of `int` (i64 range)    | `I64Buffer`     |
//! | sequence of `float`              | `F64Buffer`     |
//!
//! `bool` is checked before `int` (in Python `bool` subclasses `int`). The element type is
//! inferred from the **first non-null** element, and a `None` (null) element becomes that
//! type's [`default_value`](yggdryl_dtype::TypedDataType::default_value) (`0` / `0.0` /
//! `False`) — so a nullable column materialises into a non-nullable buffer. An empty
//! sequence, an all-null sequence, a mixed sequence, an out-of-`i64`-range `int`, or an
//! unsupported element type raises a `ValueError` / `TypeError` naming the explicit
//! constructor to use.

#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyByteArray, PyBytes, PyFloat, PyInt, PySequence};

use yggdryl_dtype::{BooleanType, F64Type, I64Type, TypedDataType};

use crate::buffer::{BooleanBuffer, F64Buffer, I64Buffer};
use crate::io::ByteBuffer;

/// Wraps the merged byte buffer (`U8Buffer` is `ByteBuffer`) for the bytes-like paths.
fn u8_buffer(bytes: Vec<u8>) -> ByteBuffer {
    ByteBuffer {
        inner: yggdryl_buffer::U8Buffer::from_vec(bytes),
    }
}

/// Builds the typed buffer matching the runtime type of `values`, inferring the
/// element type so the caller need not name a buffer class. See the module docs for
/// the mapping. Ambiguous or unsupported input raises a guided error naming the
/// explicit constructor to reach for instead.
#[pyfunction]
fn buffer(py: Python<'_>, values: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    // A bytes-like object is the byte buffer directly (not a sequence of ints).
    if let Ok(bytes) = values.downcast::<PyBytes>() {
        return Ok(u8_buffer(bytes.as_bytes().to_vec()).into_py(py));
    }
    if let Ok(array) = values.downcast::<PyByteArray>() {
        return Ok(u8_buffer(array.to_vec()).into_py(py));
    }

    let sequence = values.downcast::<PySequence>().map_err(|_| {
        PyTypeError::new_err(
            "cannot infer a buffer: pass a list/tuple of int, float, or bool, \
             or a bytes-like object for a U8Buffer",
        )
    })?;
    let n = sequence.len()?;
    if n == 0 {
        return Err(PyValueError::new_err(
            "cannot infer the element type from an empty sequence; call an explicit \
             constructor, e.g. yggdryl.buffer.I64Buffer([])",
        ));
    }

    // Infer the element type from the first non-null element; nulls (`None`) become the
    // type's default value, so a nullable column materialises into a non-nullable buffer.
    let mut first_present = None;
    for i in 0..n {
        let item = sequence.get_item(i)?;
        if !item.is_none() {
            first_present = Some(item);
            break;
        }
    }
    let first = first_present.ok_or_else(|| {
        PyValueError::new_err(
            "cannot infer the element type: every value is null; call an explicit \
             constructor, e.g. yggdryl.buffer.I64Buffer([...])",
        )
    })?;

    // `bool` first: in Python `True`/`False` are instances of `int`.
    if first.is_instance_of::<PyBool>() {
        let default = BooleanType::new().default_value();
        let mut bits = Vec::with_capacity(n);
        for i in 0..n {
            let item = sequence.get_item(i)?;
            bits.push(if item.is_none() {
                default
            } else {
                item.extract::<bool>().map_err(|_| {
                    PyValueError::new_err(
                        "cannot infer a BooleanBuffer: every non-null element must be a \
                         bool; use an explicit yggdryl.buffer constructor for a mixed \
                         sequence",
                    )
                })?
            });
        }
        return Ok(BooleanBuffer {
            inner: yggdryl_buffer::BooleanBuffer::from_bits(&bits),
        }
        .into_py(py));
    }
    if first.is_instance_of::<PyInt>() {
        let default = I64Type::new().default_value();
        let mut ints = Vec::with_capacity(n);
        for i in 0..n {
            let item = sequence.get_item(i)?;
            ints.push(if item.is_none() {
                default
            } else {
                item.extract::<i64>().map_err(|_| {
                    PyValueError::new_err(
                        "cannot infer an I64Buffer: every non-null element must be an int \
                         in the signed 64-bit range; for wider or unsigned integers use an \
                         explicit constructor such as yggdryl.buffer.U64Buffer",
                    )
                })?
            });
        }
        return Ok(I64Buffer {
            inner: yggdryl_buffer::I64Buffer::from_vec(ints),
        }
        .into_py(py));
    }
    if first.is_instance_of::<PyFloat>() {
        let default = F64Type::new().default_value();
        let mut floats = Vec::with_capacity(n);
        for i in 0..n {
            let item = sequence.get_item(i)?;
            // Require an actual `float`: `extract::<f64>()` would silently accept an `int`
            // (via `__float__`), losing precision and contradicting the mixed-sequence error.
            floats.push(if item.is_none() {
                default
            } else if item.is_instance_of::<PyFloat>() {
                item.extract::<f64>()?
            } else {
                return Err(PyValueError::new_err(
                    "cannot infer an F64Buffer: every non-null element must be a float; \
                     use an explicit yggdryl.buffer constructor for a mixed sequence",
                ));
            });
        }
        return Ok(F64Buffer {
            inner: yggdryl_buffer::F64Buffer::from_vec(floats),
        }
        .into_py(py));
    }

    let shown = first.repr()?.to_string_lossy().into_owned();
    Err(PyTypeError::new_err(format!(
        "cannot infer a buffer from an element {shown}: supported element types are \
         bool, int, and float (or pass a bytes-like object for a U8Buffer)"
    )))
}

/// Populates the `infer` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(pyo3::wrap_pyfunction!(buffer, module)?)?;
    Ok(())
}
