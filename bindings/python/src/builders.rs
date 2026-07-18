//! Module-level **generic builder** functions on `yggdryl` — the ergonomic front door that
//! hides the concrete class + its setup behind one call, **inferring the runtime type** of the
//! input and redirecting to the matching explicit surface (the same spirit as the module-level
//! [`open`](crate::open)).
//!
//! Each is one thin dispatch: [`buffer`] assembles a [`Heap`](crate::io::memory::Heap) from
//! `data` / `capacity` / `headers` / `mode`, [`array`] redirects a sequence of numbers to the
//! matching bulk `pwrite_<dtype>_array`, and [`device_buffer`] probes the device layer and hands
//! back the best device-memory buffer (an [`AmdHeap`](crate::io::amd::AmdHeap) on a real AMD
//! adapter, else a `Heap` — the CPU device-memory type). No logic beyond the dispatch lives here; the byte
//! / numeric work stays in `yggdryl_core`, and every result is a concrete binding class the caller
//! already knows.

// `useless_conversion`: pyo3's `#[pyfunction]` expansion wraps a fallible return in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat};

use crate::io::amd::AmdHeap;
use crate::io::memory::Heap;
use crate::io::mode::IOMode;
use yggdryl_core::headers;
use yggdryl_core::io::amd;
use yggdryl_core::io::memory::{self, IOBase};

/// The valid `array` dtype tokens, in one place so the guided error and the docs stay in sync.
const DTYPE_TOKENS: &str = "i8, u8, i16, u16, i32, u32, i64, u64, i128, u128, f32, f64";

/// Coerces the `headers` argument into a core [`headers::Headers`]: a `yggdryl.headers.Headers`
/// passes through, a `dict[str, str]` is built into one (insert semantics, insertion order).
/// Anything else is a guided `TypeError`.
fn coerce_headers(obj: &Bound<'_, PyAny>) -> PyResult<headers::Headers> {
    if let Ok(headers) = obj.extract::<crate::headers::Headers>() {
        return Ok(headers.inner);
    }
    if let Ok(dict) = obj.downcast::<PyDict>() {
        let mut headers = headers::Headers::with_capacity(dict.len());
        for (name, value) in dict.iter() {
            headers.insert(&name.extract::<String>()?, &value.extract::<String>()?);
        }
        return Ok(headers);
    }
    Err(PyTypeError::new_err(
        "headers must be a yggdryl.headers.Headers or a dict[str, str]",
    ))
}

/// Infers the element dtype of `values` from the runtime types: `"f64"` if **any** element is a
/// Python `float`, else `"i64"` (every element a Python `int`). Empty infers `"i64"`.
fn infer_dtype(values: &Bound<'_, PyAny>) -> PyResult<&'static str> {
    for item in values.iter()? {
        if item?.is_instance_of::<PyFloat>() {
            return Ok("f64");
        }
    }
    Ok("i64")
}

/// **Generic buffer builder** — assembles a [`Heap`](crate::io::memory::Heap) behind one call,
/// hiding the constructor + `with_capacity` + `set_headers` + `set_mode`. `data` (bytes /
/// bytearray) is copied into the buffer, or an empty one is built (pre-sized to `capacity` when
/// given). `headers` is a `yggdryl.headers.Headers` **or** a `dict[str, str]`; `mode` is a
/// `yggdryl.io.IOMode`.
#[pyfunction]
#[pyo3(signature = (data = None, *, capacity = None, headers = None, mode = None))]
fn buffer(
    data: Option<Vec<u8>>,
    capacity: Option<usize>,
    headers: Option<&Bound<'_, PyAny>>,
    mode: Option<IOMode>,
) -> PyResult<Heap> {
    let mut inner = match data {
        Some(bytes) => {
            let mut inner = memory::Heap::from_vec(bytes);
            if let Some(capacity) = capacity {
                inner.ensure_capacity(capacity as u64);
            }
            inner
        }
        None => match capacity {
            Some(capacity) => memory::Heap::with_capacity(capacity),
            None => memory::Heap::new(),
        },
    };
    if let Some(headers) = headers {
        inner.set_headers(coerce_headers(headers)?);
    }
    if let Some(mode) = mode {
        inner.set_mode(mode.into());
    }
    Ok(Heap { inner })
}

/// **Generic numeric-array builder** — writes `values` (a sequence of numbers) into a fresh
/// [`Heap`](crate::io::memory::Heap) as a dense little-endian array, redirecting by `dtype` to
/// the matching bulk `pwrite_<dtype>_array`. When `dtype` is omitted it is **inferred**: `"i64"`
/// when every value is a Python `int`, `"f64"` when any is a `float`. `dtype` names the element
/// type (`i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`, `i128`, `u128`, `f32`, `f64`); an
/// unknown token raises a guided `ValueError`. Read the values back with the Heap's
/// `pread_<dtype>_array` (or `pread_byte_array` for `u8`).
#[pyfunction]
#[pyo3(signature = (values, dtype = None))]
fn array(values: &Bound<'_, PyAny>, dtype: Option<String>) -> PyResult<Heap> {
    let dtype = match dtype {
        Some(dtype) => dtype,
        None => infer_dtype(values)?.to_string(),
    };
    let mut inner = memory::Heap::new();
    let ioerr = |e: memory::IoError| PyValueError::new_err(e.to_string());
    match dtype.as_str() {
        "i8" => inner
            .pwrite_i8_array(0, &values.extract::<Vec<i8>>()?)
            .map_err(ioerr)?,
        // No dense `pwrite_u8_array` exists — the byte surface *is* the `u8` array; the caller
        // reads these back with `pread_byte_array`.
        "u8" => {
            inner.pwrite_byte_array(0, &values.extract::<Vec<u8>>()?);
        }
        "i16" => inner
            .pwrite_i16_array(0, &values.extract::<Vec<i16>>()?)
            .map_err(ioerr)?,
        "u16" => inner
            .pwrite_u16_array(0, &values.extract::<Vec<u16>>()?)
            .map_err(ioerr)?,
        "i32" => inner
            .pwrite_i32_array(0, &values.extract::<Vec<i32>>()?)
            .map_err(ioerr)?,
        "u32" => inner
            .pwrite_u32_array(0, &values.extract::<Vec<u32>>()?)
            .map_err(ioerr)?,
        "i64" => inner
            .pwrite_i64_array(0, &values.extract::<Vec<i64>>()?)
            .map_err(ioerr)?,
        "u64" => inner
            .pwrite_u64_array(0, &values.extract::<Vec<u64>>()?)
            .map_err(ioerr)?,
        "i128" => inner
            .pwrite_i128_array(0, &values.extract::<Vec<i128>>()?)
            .map_err(ioerr)?,
        "u128" => inner
            .pwrite_u128_array(0, &values.extract::<Vec<u128>>()?)
            .map_err(ioerr)?,
        "f32" => inner
            .pwrite_f32_array(0, &values.extract::<Vec<f32>>()?)
            .map_err(ioerr)?,
        "f64" => inner
            .pwrite_f64_array(0, &values.extract::<Vec<f64>>()?)
            .map_err(ioerr)?,
        other => {
            return Err(PyValueError::new_err(format!(
                "unknown dtype {other:?}: expected one of {DTYPE_TOKENS}"
            )))
        }
    }
    Ok(Heap { inner })
}

/// Whether `device_buffer` should back onto a real AMD adapter (an [`AmdHeap`]) rather than the
/// CPU [`Heap`]. `None` probes the device layer (device when an AMD adapter is present); a `str`
/// selects (`"cpu"` → CPU, `"amd"` / `"gpu"` / `"cuda"` → device); a `yggdryl.amd.AmdDevice`
/// decides by its `is_present()`.
fn wants_gpu(device: Option<&Bound<'_, PyAny>>) -> PyResult<bool> {
    match device {
        None => Ok(amd::detect().is_some()),
        Some(device) => {
            if let Ok(device) = device.extract::<crate::io::amd::AmdDevice>() {
                Ok(device.inner.is_present())
            } else if let Ok(name) = device.extract::<String>() {
                match name.to_ascii_lowercase().as_str() {
                    "cpu" => Ok(false),
                    "amd" | "gpu" | "cuda" => Ok(true),
                    other => Err(PyValueError::new_err(format!(
                        "unknown device {other:?}: expected \"cpu\", \"amd\", \"gpu\", \"cuda\", \
                         or a yggdryl.amd.AmdDevice"
                    ))),
                }
            } else {
                Err(PyTypeError::new_err(
                    "device must be a str (\"cpu\" / \"amd\") or a yggdryl.amd.AmdDevice",
                ))
            }
        }
    }
}

/// **Best device-memory buffer** — hides the device probe + class selection. Returns an
/// [`AmdHeap`] (device memory, `data` uploaded host → device) when a real AMD adapter is available
/// or `device` names a non-CPU one, else a [`Heap`](crate::io::memory::Heap) (the CPU
/// device-memory type — a `Heap` is simply the CPU heap) holding `data`. Both share the byte-I/O
/// surface, so the caller reads / writes the result the same way regardless of where its memory
/// lives.
#[pyfunction]
#[pyo3(signature = (data = None, *, device = None))]
fn device_buffer(
    py: Python<'_>,
    data: Option<Vec<u8>>,
    device: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyObject> {
    if wants_gpu(device)? {
        let inner = match data {
            Some(bytes) => amd::AmdHeap::from_host(&bytes),
            None => amd::AmdHeap::new(),
        };
        Ok(Py::new(py, AmdHeap { inner })?.into_any())
    } else {
        let inner = match data {
            Some(bytes) => memory::Heap::from_vec(bytes),
            None => memory::Heap::new(),
        };
        Ok(Py::new(py, Heap { inner })?.into_any())
    }
}

/// Registers the module-level builder functions on the top `yggdryl` module.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(buffer, module)?)?;
    module.add_function(wrap_pyfunction!(array, module)?)?;
    module.add_function(wrap_pyfunction!(device_buffer, module)?)?;
    Ok(())
}
