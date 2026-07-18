//! Top-level **builder** functions — the ergonomic, type-inferring front doors that hide the
//! concrete binding classes behind one call, mirroring the spirit of the top-level [`open`].
//!
//! Each function dispatches on the runtime type / arguments and redirects to the matching
//! `memory.Heap` / `amd.AmdHeap` wrapper — JS-level convenience glue over the existing classes,
//! with no logic beyond the dispatch:
//!
//! - [`buffer`] builds a `memory.Heap` from optional bytes plus an options object
//!   (`{ capacity, headers, mode }`);
//! - [`array`] builds a `memory.Heap` from a numeric array, inferring (or taking) an element
//!   `dtype` and writing it through the matching vectorized bulk kernel;
//! - [`device_buffer`] returns the **best available** device buffer — an `amd.AmdHeap` when a
//!   real AMD adapter is present (or `"amd"` is requested), else a `memory.Heap` (the CPU device
//!   memory).
//!
//! Every failing build surfaces as a thrown `Error` carrying a guided message.

use napi::bindgen_prelude::{BigInt, Either, Object, Uint8Array};
use napi_derive::napi;

use crate::headers::Headers;
use crate::io::amd::AmdHeap;
use crate::io::memory::{to_error, Heap};
use yggdryl_core::io::amd as amd_core;
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
/// - `capacity` pre-sizes the heap so appends do not reallocate — an **empty** heap of that
///   capacity when `data` is omitted (`Heap.withCapacity`), or **reserved headroom** past the
///   copied bytes when `data` is given (`ensureCapacity`);
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

    // The bytes decide the base: a copy of `data` (reserving headroom to `capacity` when
    // given), else a capacity-sized (or empty) heap.
    let mut inner = match (&data, capacity) {
        (Some(bytes), capacity) => {
            let mut inner = mem_core::Heap::from_slice(bytes.as_ref());
            if let Some(capacity) = capacity {
                inner.ensure_capacity(capacity as u64);
            }
            inner
        }
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

    // One arm per element type, collapsed by a local macro that expands to the shared
    // sequence: collect `values` into a pre-sized typed vector, build a capacity-sized heap
    // (`width` bytes per element), and stream them through the matching vectorized bulk
    // kernel. (The `u8` arm stays explicit — the byte surface *is* the `u8` array, and its
    // `pwriteByteArray` is non-fallible, so it has no `map_err` to share.)
    macro_rules! arm {
        ($convert:expr, $width:expr, $write:ident) => {{
            let typed: Vec<_> = values.iter().map($convert).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len() * $width);
            heap.$write(0, &typed).map_err(to_error)?;
            heap
        }};
    }

    let inner = match dtype.as_str() {
        "i8" => arm!(|v| as_i128(v) as i8, 1, pwrite_i8_array),
        "u8" => {
            let typed: Vec<u8> = values.iter().map(|v| as_u128(v) as u8).collect();
            let mut heap = mem_core::Heap::with_capacity(typed.len());
            heap.pwrite_byte_array(0, &typed);
            heap
        }
        "i16" => arm!(|v| as_i128(v) as i16, 2, pwrite_i16_array),
        "u16" => arm!(|v| as_u128(v) as u16, 2, pwrite_u16_array),
        "i32" => arm!(|v| as_i128(v) as i32, 4, pwrite_i32_array),
        "u32" => arm!(|v| as_u128(v) as u32, 4, pwrite_u32_array),
        "i64" => arm!(|v| as_i128(v) as i64, 8, pwrite_i64_array),
        "u64" => arm!(|v| as_u128(v) as u64, 8, pwrite_u64_array),
        "i128" => arm!(as_i128, 16, pwrite_i128_array),
        "u128" => arm!(as_u128, 16, pwrite_u128_array),
        "f32" => arm!(|v| as_f64(v) as f32, 4, pwrite_f32_array),
        "f64" => arm!(as_f64, 8, pwrite_f64_array),
        other => {
            return Err(to_error(format!(
                "unknown dtype '{other}': expected one of {DTYPE_TOKENS}"
            )))
        }
    };

    Ok(Heap { inner })
}

/// Builds the **best available device buffer** — the generic, hardware-inferring device-memory
/// front door. Returns an `amd.AmdHeap` when a real AMD adapter is present (`amd.detect()` finds
/// one) **or** `device` names `"amd"`, else a `memory.Heap` (the CPU device-memory type — a
/// `Heap` is simply the CPU heap). When `data` (a `Buffer` / `Uint8Array`) is given it seeds the
/// buffer (uploaded to the device for a device heap, copied for a heap). Throws a guided `Error`
/// for an unknown `device` token.
#[napi(
    ts_args_type = "data?: Uint8Array, device?: string",
    ts_return_type = "memory.Heap | amd.AmdHeap"
)]
pub fn device_buffer(
    data: Option<Uint8Array>,
    device: Option<String>,
) -> napi::Result<Either<Heap, AmdHeap>> {
    let use_gpu = match device.as_deref() {
        // Case-insensitive tokens, matching the Python twin: `"cpu"` → the CPU heap,
        // `"amd"` / `"gpu"` / `"cuda"` → a device heap (an `AmdHeap`).
        Some(name) => match name.to_ascii_lowercase().as_str() {
            "amd" | "gpu" | "cuda" => true,
            "cpu" => false,
            _ => {
                return Err(to_error(format!(
                    "unknown device '{name}': expected 'cpu' (the CPU heap) or one of 'amd' / \
                     'gpu' / 'cuda' (a device heap), or omit it to pick the best available device"
                )))
            }
        },
        // No preference: prefer a real AMD adapter when the probe finds one, else the CPU heap.
        None => amd_core::detect().is_some(),
    };

    if use_gpu {
        let inner = match &data {
            Some(bytes) => amd_core::AmdHeap::from_host(bytes.as_ref()),
            None => amd_core::AmdHeap::new(),
        };
        Ok(Either::B(AmdHeap { inner }))
    } else {
        let inner = match &data {
            Some(bytes) => mem_core::Heap::from_slice(bytes.as_ref()),
            None => mem_core::Heap::new(),
        };
        Ok(Either::A(Heap { inner }))
    }
}
