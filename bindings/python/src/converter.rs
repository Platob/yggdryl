//! The `yggdryl.converter` submodule — dtype-keyed representation conversion.
//!
//! A thin facade over `yggdryl_core`'s [`converter`](yggdryl_core::codec::converter)
//! family. The core's typed converters ([`CastConverter`], [`StringConverter`], …) fix
//! their element types at compile time, which the FFI cannot hold, so the binding
//! keys them on a dtype **name** at runtime via
//! [`PrimitiveType`](yggdryl_converter::PrimitiveType) — one of `i8 … u64`, `f32`, `f64`.
//! An unknown name, an out-of-range value, or invalid UTF-8 raises a `ValueError`
//! whose message names the accepted dtypes / formats (core rule 12).

#![allow(clippy::useless_conversion)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl_buffer::IoPrimitive;
use yggdryl_converter::{ConverterKind, PrimitiveType, TypedConverter, Utf8Converter};

/// Maps a core [`ConvertError`](yggdryl_converter::ConvertError) to a Python `ValueError`.
fn convert_err(error: yggdryl_converter::ConvertError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Resolves a dtype name, or a guided `ValueError`.
fn dtype(name: &str) -> PyResult<PrimitiveType> {
    PrimitiveType::from_name(name).map_err(convert_err)
}

/// Resolves a converter-kind name, or a guided `ValueError`.
fn kind(name: &str) -> PyResult<ConverterKind> {
    ConverterKind::from_name(name).map_err(convert_err)
}

/// Resolves an optional dtype name (`None` passes through as no argument).
fn opt_dtype(name: Option<&str>) -> PyResult<Option<PrimitiveType>> {
    name.map(dtype).transpose()
}

/// Decodes exactly-`width` little-endian `bytes` into the Python scalar for `pt` — an
/// `int` for the integer dtypes, a `float` for `f32` / `f64`.
fn scalar_to_py(py: Python<'_>, pt: PrimitiveType, bytes: &[u8]) -> PyResult<PyObject> {
    Ok(match pt {
        PrimitiveType::I8 => <i8 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::I16 => <i16 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::I32 => <i32 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::I64 => <i64 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::U8 => <u8 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::U16 => <u16 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::U32 => <u32 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::U64 => <u64 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::F32 => <f32 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        PrimitiveType::F64 => <f64 as IoPrimitive>::from_le_slice(bytes).into_py(py),
        _ => return Err(PyValueError::new_err("unsupported dtype")),
    })
}

/// Extracts a Python scalar as the `pt` element and returns its little-endian bytes.
/// The integer dtypes widen the `int` to `i128` and defer the range check to the core
/// [`PrimitiveType::int_to_le_bytes`], so an out-of-range value raises the same guided
/// `ValueError` as the Node binding (rule 12), not an opaque `OverflowError`.
fn scalar_from_py(pt: PrimitiveType, value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    match pt {
        PrimitiveType::F32 => Ok(value.extract::<f32>()?.to_le_vec()),
        PrimitiveType::F64 => Ok(value.extract::<f64>()?.to_le_vec()),
        _ => pt
            .int_to_le_bytes(value.extract::<i128>()?)
            .map_err(convert_err),
    }
}

/// Casts packed little-endian `data` from `from_dtype` to `to_dtype` (C-style `as`),
/// element by element, returning the target's little-endian bytes.
#[pyfunction]
fn cast<'py>(
    py: Python<'py>,
    data: &[u8],
    from_dtype: &str,
    to_dtype: &str,
) -> PyResult<Bound<'py, PyBytes>> {
    let out = dtype(from_dtype)?
        .cast_bytes(dtype(to_dtype)?, data)
        .map_err(convert_err)?;
    Ok(PyBytes::new_bound(py, &out))
}

/// Flexibly parses `text` into a `dtype` scalar — accepts decimal, `0x`/`0o`/`0b`
/// integers with `_` separators and signs, and decimal/scientific floats.
#[pyfunction]
fn parse(py: Python<'_>, text: &str, dtype_name: &str) -> PyResult<PyObject> {
    let primitive = dtype(dtype_name)?;
    let bytes = primitive.parse_bytes(text).map_err(convert_err)?;
    scalar_to_py(py, primitive, &bytes)
}

/// Converts a numeric scalar `value` from `from_dtype` to `to_dtype` (C-style `as`),
/// e.g. `convert(300, "i32", "u8")` or `convert(3, "i32", "f32")`.
#[pyfunction]
fn convert(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    from_dtype: &str,
    to_dtype: &str,
) -> PyResult<PyObject> {
    let from = dtype(from_dtype)?;
    let to = dtype(to_dtype)?;
    let bytes = scalar_from_py(from, value)?;
    let out = from.cast_bytes(to, &bytes).map_err(convert_err)?;
    scalar_to_py(py, to, &out)
}

/// Renders a `dtype` scalar `value` to its string form.
#[pyfunction]
fn format(value: &Bound<'_, PyAny>, dtype_name: &str) -> PyResult<String> {
    let primitive = dtype(dtype_name)?;
    let bytes = scalar_from_py(primitive, value)?;
    primitive.format_bytes(&bytes).map_err(convert_err)
}

/// Runs a named converter forward over the whole `data` byte array — the general
/// "overall" convert. `converter` is one of `"cast"`, `"string"`, `"bytes"`, `"utf8"`;
/// `from_dtype` / `to_dtype` name the dtype arguments the kind needs (both for `cast`,
/// one — `from_dtype` — for `string` / `bytes`, neither for `utf8`).
#[pyfunction]
#[pyo3(signature = (data, converter, from_dtype=None, to_dtype=None))]
fn convert_bytes<'py>(
    py: Python<'py>,
    data: &[u8],
    converter: &str,
    from_dtype: Option<&str>,
    to_dtype: Option<&str>,
) -> PyResult<Bound<'py, PyBytes>> {
    let out = kind(converter)?
        .convert_bytes(data, opt_dtype(from_dtype)?, opt_dtype(to_dtype)?)
        .map_err(convert_err)?;
    Ok(PyBytes::new_bound(py, &out))
}

/// Runs a named converter backward over the whole `data` byte array — the exact
/// inverse of [`convert_bytes`], with the same arguments (e.g. `invert_bytes(le, "string",
/// "i32")` renders i32 bytes back to their decimal text).
#[pyfunction]
#[pyo3(signature = (data, converter, from_dtype=None, to_dtype=None))]
fn invert_bytes<'py>(
    py: Python<'py>,
    data: &[u8],
    converter: &str,
    from_dtype: Option<&str>,
    to_dtype: Option<&str>,
) -> PyResult<Bound<'py, PyBytes>> {
    let out = kind(converter)?
        .invert_bytes(data, opt_dtype(from_dtype)?, opt_dtype(to_dtype)?)
        .map_err(convert_err)?;
    Ok(PyBytes::new_bound(py, &out))
}

/// Encodes `text` to its UTF-8 bytes.
#[pyfunction]
fn utf8_encode<'py>(py: Python<'py>, text: &str) -> Bound<'py, PyBytes> {
    PyBytes::new_bound(py, text.as_bytes())
}

/// Decodes UTF-8 `data` to a string, raising `ValueError` (naming the failing offset)
/// on invalid UTF-8.
#[pyfunction]
fn utf8_decode(data: &[u8]) -> PyResult<String> {
    Utf8Converter::new()
        .decode(data.to_vec())
        .map_err(convert_err)
}

/// Populates the `converter` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(pyo3::wrap_pyfunction!(cast, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(convert, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(convert_bytes, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(invert_bytes, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(parse, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(format, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(utf8_encode, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(utf8_decode, module)?)?;
    Ok(())
}
