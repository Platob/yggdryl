//! Top-level **builder** functions — the ergonomic, type-inferring front doors that hide the
//! concrete binding classes behind one call, mirroring the spirit of the top-level [`open`].
//!
//! Each function dispatches on the runtime type / arguments and redirects to the matching
//! `memory.Heap` / `gpu.AmdBuffer` wrapper — JS-level convenience glue over the existing classes,
//! with no logic beyond the dispatch:
//!
//! - [`buffer`] builds a `memory.Heap` from optional bytes plus an options object
//!   (`{ capacity, headers, mode }`);
//! - [`array`] builds a `memory.Heap` from a numeric array, inferring (or taking) an element
//!   `dtype` and writing it through the matching vectorized bulk kernel;
//! - [`device_buffer`] returns the **best available** device buffer — a `gpu.AmdBuffer` when a
//!   real GPU is present (or `"amd"` is requested), else a `memory.Heap` (the CPU device memory).
//!
//! Every failing build surfaces as a thrown `Error` carrying a guided message.

use napi::bindgen_prelude::{BigInt, Either, Object, Uint8Array};
use napi_derive::napi;

use crate::headers::Headers;
use crate::io::gpu::AmdBuffer;
use crate::io::memory::{to_error, Heap};
use yggdryl_core::io::gpu as gpu_core;
use yggdryl_core::io::memory::{self as mem_core, IOBase};
use yggdryl_core::io::{IOMode as CoreIOMode, IoError};

/// The element `dtype` tokens `array` accepts, for the guided error naming the valid set.
const DTYPE_TOKENS: &str = "i8, u8, i16, u16, i32, u32, i64, u64, i128, u128, f32, f64";

/// Resolves an [`IOMode`](CoreIOMode) from the `mode` field of a `buffer` options object — a
/// string dispatches to the core name parser (`"rw"`), a number to the wire-stable value
/// (`3`). Mirrors `io.parseIoMode`; throws the core's guided text on an unknown token.
fn resolve_mode(value: Either<String, i64>) -> napi::Result<CoreIOMode> {
    match value {
        Either::A(name) => CoreIOMode::parse_str(&name).map_err(to_error),
        Either::B(number) => match u8::try_from(number) {
            Ok(byte) => CoreIOMode::from_u8(byte).map_err(to_error),
            Err(_) => Err(to_error(IoError::UnknownName {
                kind: "IOMode",
                input: number.to_string(),
                expected: "1 (read), 2 (write), 3 (read_write), 4 (append), 5 (overwrite)",
            })),
        },
    }
}

/// Reads the optional `headers` field of a `buffer` options object into a core [`Headers`],
/// **inferring** its runtime type: a `headers.Headers` **instance** is cloned, a plain
/// `Record<string, string>` object is materialized into a fresh `Headers` (one entry per key).
/// Returns `None` when the field is absent.
fn resolve_headers(options: &Object) -> napi::Result<Option<yggdryl_core::headers::Headers>> {
    // A `headers.Headers` instance — cloned straight through.
    let instance: napi::Result<Option<&Headers>> = options.get("headers");
    if let Ok(Some(handle)) = instance {
        return Ok(Some(handle.inner.clone()));
    }
    // Else a plain object of string → string, materialized into an ordered Headers map.
    let map: Option<std::collections::HashMap<String, String>> = options.get("headers")?;
    if let Some(map) = map {
        let mut headers = yggdryl_core::headers::Headers::with_capacity(map.len());
        for (name, value) in &map {
            headers.append(name, value);
        }
        return Ok(Some(headers));
    }
    Ok(None)
}

/// Builds a **`memory.Heap`** — the generic, type-inferring buffer front door. `data` (a
/// `Buffer` / `Uint8Array`) seeds the heap with a **copy** of its bytes; omit it for an empty
/// heap. `options` tunes the result:
///
/// - `capacity` pre-allocates an **empty** heap (`Heap.withCapacity`) so appends do not
///   reallocate (ignored when `data` is given — the copy already sizes the buffer);
/// - `headers` (a `headers.Headers` **or** a plain `{ name: value }` object) becomes the heap's
///   metadata map (`setHeaders`);
/// - `mode` (an `io.IOMode`, or its name/number) sets the access mode (`setMode`).
#[napi(
    ts_args_type = "data?: Uint8Array, options?: { capacity?: number, headers?: headers.Headers | Record<string, string>, mode?: io.IOMode }",
    ts_return_type = "memory.Heap"
)]
pub fn buffer(data: Option<Uint8Array>, options: Option<Object>) -> napi::Result<Heap> {
    let capacity: Option<u32> = match &options {
        Some(options) => options.get("capacity")?,
        None => None,
    };

    // The bytes decide the base: a copy of `data`, else a capacity-sized (or empty) heap.
    let mut inner = match (&data, capacity) {
        (Some(bytes), _) => mem_core::Heap::from_slice(bytes.as_ref()),
        (None, Some(capacity)) => mem_core::Heap::with_capacity(capacity as usize),
        (None, None) => mem_core::Heap::new(),
    };

    if let Some(options) = &options {
        if let Some(headers) = resolve_headers(options)? {
            inner.set_headers(headers);
        }
        let mode: Option<Either<String, i64>> = options.get("mode")?;
        if let Some(mode) = mode {
            inner.set_mode(resolve_mode(mode)?);
        }
    }

    Ok(Heap { inner })
}

/// One element of an [`array`] input — a JS `number` (crossing as `f64`) or a `bigint` (for the
/// 64-/128-bit dtypes).
type ArrayValue = Either<f64, BigInt>;

/// Interprets an [`ArrayValue`] as a signed 128-bit integer (the widest signed carrier).
fn as_i128(value: &ArrayValue) -> i128 {
    match value {
        Either::A(number) => *number as i128,
        Either::B(big) => big.get_i128().0,
    }
}

/// Interprets an [`ArrayValue`] as an unsigned 128-bit integer (the widest unsigned carrier).
fn as_u128(value: &ArrayValue) -> u128 {
    match value {
        Either::A(number) => *number as u128,
        Either::B(big) => big.get_u128().1,
    }
}

/// Interprets an [`ArrayValue`] as an `f64` (a `bigint` widens through its `i64` value).
fn as_f64(value: &ArrayValue) -> f64 {
    match value {
        Either::A(number) => *number,
        Either::B(big) => big.get_i64().0 as f64,
    }
}

/// Whether every value is an integer — a whole `number` (`Number.isInteger`) or a `bigint` —
/// the signal that a `dtype`-less array should default to `"i64"` rather than `"f64"`.
fn all_integers(values: &[ArrayValue]) -> bool {
    values.iter().all(|value| match value {
        Either::A(number) => number.is_finite() && number.fract() == 0.0,
        Either::B(_) => true,
    })
}

/// Builds a **`memory.Heap`** holding `values` written as a dense little-endian array of
/// `dtype` — the generic, type-inferring typed-array front door. `values` is a `number[]` (or a
/// `bigint[]` for the 64-/128-bit dtypes). `dtype` is one of `i8`, `u8`, `i16`, `u16`, `i32`,
/// `u32`, `i64`, `u64`, `i128`, `u128`, `f32`, `f64`; when **omitted** it is inferred — `"i64"`
/// if every value is an integer, `"f64"` if any is fractional. Throws a guided `Error` naming
/// the valid tokens for an unknown `dtype`.
#[napi(
    ts_args_type = "values: Array<number | bigint>, dtype?: string",
    ts_return_type = "memory.Heap"
)]
pub fn array(values: Vec<ArrayValue>, dtype: Option<String>) -> napi::Result<Heap> {
    let dtype = match dtype {
        Some(dtype) => dtype,
        None if all_integers(&values) => "i64".to_string(),
        None => "f64".to_string(),
    };

    // Build a pre-sized heap and stream `values` through the matching vectorized bulk kernel.
    let inner = match dtype.as_str() {
        "i8" => {
            let typed: Vec<i8> = values.iter().map(|v| as_i128(v) as i8).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len());
            heap.pwrite_i8_array(0, &typed).map_err(to_error)?;
            heap
        }
        "u8" => {
            let typed: Vec<u8> = values.iter().map(|v| as_u128(v) as u8).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len());
            heap.pwrite_byte_array(0, &typed);
            heap
        }
        "i16" => {
            let typed: Vec<i16> = values.iter().map(|v| as_i128(v) as i16).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 2);
            heap.pwrite_i16_array(0, &typed).map_err(to_error)?;
            heap
        }
        "u16" => {
            let typed: Vec<u16> = values.iter().map(|v| as_u128(v) as u16).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 2);
            heap.pwrite_u16_array(0, &typed).map_err(to_error)?;
            heap
        }
        "i32" => {
            let typed: Vec<i32> = values.iter().map(|v| as_i128(v) as i32).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 4);
            heap.pwrite_i32_array(0, &typed).map_err(to_error)?;
            heap
        }
        "u32" => {
            let typed: Vec<u32> = values.iter().map(|v| as_u128(v) as u32).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 4);
            heap.pwrite_u32_array(0, &typed).map_err(to_error)?;
            heap
        }
        "i64" => {
            let typed: Vec<i64> = values.iter().map(|v| as_i128(v) as i64).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 8);
            heap.pwrite_i64_array(0, &typed).map_err(to_error)?;
            heap
        }
        "u64" => {
            let typed: Vec<u64> = values.iter().map(|v| as_u128(v) as u64).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 8);
            heap.pwrite_u64_array(0, &typed).map_err(to_error)?;
            heap
        }
        "i128" => {
            let typed: Vec<i128> = values.iter().map(as_i128).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 16);
            heap.pwrite_i128_array(0, &typed).map_err(to_error)?;
            heap
        }
        "u128" => {
            let typed: Vec<u128> = values.iter().map(as_u128).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 16);
            heap.pwrite_u128_array(0, &typed).map_err(to_error)?;
            heap
        }
        "f32" => {
            let typed: Vec<f32> = values.iter().map(|v| as_f64(v) as f32).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 4);
            heap.pwrite_f32_array(0, &typed).map_err(to_error)?;
            heap
        }
        "f64" => {
            let typed: Vec<f64> = values.iter().map(as_f64).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * 8);
            heap.pwrite_f64_array(0, &typed).map_err(to_error)?;
            heap
        }
        other => {
            return Err(to_error(format!(
                "unknown dtype '{other}': expected one of {DTYPE_TOKENS}"
            )))
        }
    };

    Ok(Heap { inner })
}

/// Builds the **best available device buffer** — the generic, hardware-inferring device-memory
/// front door. Returns a `gpu.AmdBuffer` when a real GPU is present (any non-CPU device in
/// `gpu.availableDevices()`) **or** `device` names `"amd"`, else a `memory.Heap` (the CPU
/// device-memory type — the core aliases `CpuHeap == Heap`). When `data` (a `Buffer` /
/// `Uint8Array`) is given it seeds the buffer (uploaded to the device for a GPU buffer, copied
/// for a heap). Throws a guided `Error` for an unknown `device` token.
#[napi(
    ts_args_type = "data?: Uint8Array, device?: string",
    ts_return_type = "memory.Heap | gpu.AmdBuffer"
)]
pub fn device_buffer(
    data: Option<Uint8Array>,
    device: Option<String>,
) -> napi::Result<Either<Heap, AmdBuffer>> {
    let use_gpu = match device.as_deref() {
        Some("amd") => true,
        Some("cpu") => false,
        Some(other) => {
            return Err(to_error(format!(
                "unknown device '{other}': expected 'cpu' or 'amd', or omit it to pick the best \
                 available device"
            )))
        }
        // No preference: prefer a real GPU when the probe finds one, else the CPU heap.
        None => gpu_core::available_devices()
            .iter()
            .any(|device| !device.is_cpu()),
    };

    if use_gpu {
        let inner = match &data {
            Some(bytes) => gpu_core::AmdBuffer::from_host(bytes.as_ref()),
            None => gpu_core::AmdBuffer::new(),
        };
        Ok(Either::B(AmdBuffer { inner }))
    } else {
        let inner = match &data {
            Some(bytes) => mem_core::Heap::from_slice(bytes.as_ref()),
            None => mem_core::Heap::new(),
        };
        Ok(Either::A(Heap { inner }))
    }
}
