//! The `yggdryl.gpu` namespace — the **device-memory** layer (feature `gpu-amd`), organized
//! by GPU architecture.
//!
//! Mirrors `yggdryl_core::io::gpu`: the by-architecture device probe (`availableDevices` /
//! `defaultDevice`), the [`GpuDevice`] value descriptor, and the [`AmdBuffer`] AMD device-memory
//! buffer — a full `GpuMemory` that speaks the whole `IOBase` byte + vectorized-bulk surface plus
//! host↔device `upload` / `download`. The **CPU** device-memory type is `memory.Heap` itself
//! (the core aliases `CpuHeap = Heap`), so there is deliberately **no** separate CPU class here —
//! allocate a `memory.Heap` for CPU device memory. Every method is a thin one- or two-line
//! delegation to `yggdryl_core`; every failing byte op surfaces as a thrown `Error` carrying the
//! core's guided text unchanged.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{BigInt, Buffer};
use napi_derive::napi;

use crate::io::meminfo::MemoryInfo;
use crate::io::memory::{check_bulk_read, to_error};
use yggdryl_core::io::gpu::{self as core, Compute, GpuMemory};
use yggdryl_core::io::memory::Aggregate;
use yggdryl_core::io::memory::IOBase;

/// A Java-style `i32` content hash of a value, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// Enumerates the compute devices this build can allocate on — **adapting to the hardware
/// present**. Each enabled architecture (AMD via `gpu-amd`) contributes what it detects, and the
/// portable CPU device is always appended last, so the result is never empty.
#[napi(namespace = "gpu")]
pub fn available_devices() -> Vec<GpuDevice> {
    core::available_devices()
        .into_iter()
        .map(|inner| GpuDevice { inner })
        .collect()
}

/// The **default** device — the first detected hardware GPU, else the CPU fallback.
#[napi(namespace = "gpu")]
pub fn default_device() -> GpuDevice {
    GpuDevice {
        inner: core::default_device(),
    }
}

/// A **value description of one compute device** — its architecture, human name, and total
/// memory (VRAM for a GPU, host RAM for the CPU). A plain value (equatable); the live free-memory
/// is a fresh `memoryInfo()` query, not baked into the descriptor.
#[napi(namespace = "gpu")]
pub struct GpuDevice {
    pub(crate) inner: core::GpuDevice,
}

#[napi(namespace = "gpu")]
impl GpuDevice {
    /// The short lowercase architecture token — `"cpu"`, `"amd"`, or `"cuda"`.
    #[napi]
    pub fn backend(&self) -> String {
        self.inner.backend().as_str().to_string()
    }

    /// The human-readable device name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The total device memory in bytes (VRAM, or host RAM for the CPU device) — an `i64`
    /// (a JS number, exact to 2^53).
    #[napi]
    pub fn total_memory(&self) -> i64 {
        self.inner.total_memory() as i64
    }

    /// Whether this is the CPU (host-memory) device.
    #[napi]
    pub fn is_cpu(&self) -> bool {
        self.inner.is_cpu()
    }

    /// A **live capacity snapshot** for this device — the CPU device queries host RAM fresh; a
    /// GPU device reports its total VRAM.
    #[napi]
    pub fn memory_info(&self) -> MemoryInfo {
        MemoryInfo {
            inner: self.inner.memory_info(),
        }
    }

    /// Content equality — equal iff the backend, name, and total memory all match.
    #[napi]
    pub fn equals(&self, other: &GpuDevice) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash — equal devices hash equal.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        java_hash(&self.inner)
    }

    /// A short debug string of the form `GpuDevice(<backend>, <name>)`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "GpuDevice({}, {})",
            self.inner.backend().as_str(),
            self.inner.name()
        )
    }
}

/// An **AMD Radeon device-memory buffer** — a full `GpuMemory` over the detected AMD device (or
/// the CPU fallback when none), implementing the whole `IOBase` byte + vectorized-bulk surface
/// plus host↔device `upload` / `download`. (The resident store stages through host memory for
/// now; the API is stable so wiring the VRAM queue does not change a caller.)
#[napi(namespace = "gpu")]
#[derive(Default)]
pub struct AmdBuffer {
    pub(crate) inner: core::AmdBuffer,
}

#[napi(namespace = "gpu")]
impl AmdBuffer {
    /// An empty AMD device buffer on the detected AMD device (or the CPU fallback when none).
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: core::AmdBuffer::new(),
        }
    }

    /// An empty buffer with room for `capacity` bytes before reallocating.
    #[napi(factory)]
    pub fn with_capacity(capacity: u32) -> AmdBuffer {
        Self {
            inner: core::AmdBuffer::with_capacity(capacity as usize),
        }
    }

    /// A buffer initialized by **uploading** `data` (host → device).
    #[napi(factory)]
    pub fn from_host(data: Buffer) -> AmdBuffer {
        Self {
            inner: core::AmdBuffer::from_host(data.as_ref()),
        }
    }

    // ---- GpuMemory transfer surface ----------------------------------------------------

    /// **Uploads** `host` into device memory, replacing the whole content (and syncing the size
    /// headers) — the "copy this array to the GPU" entry point.
    #[napi]
    pub fn upload(&mut self, host: Buffer) -> napi::Result<()> {
        self.inner.upload(host.as_ref()).map_err(to_error)
    }

    /// **Downloads** up to `length` bytes of device memory (from the start) into a fresh
    /// `Buffer` — short when `length` exceeds the buffer. `length` is clamped to the buffer
    /// size **before** allocating, so an over-long request never over-allocates.
    #[napi]
    pub fn download(&self, length: u32) -> Buffer {
        let n = self.inner.byte_size().min(length as u64) as usize;
        self.inner.pread_vec(0, n).into()
    }

    /// **Downloads** the whole device buffer into a fresh host `Buffer` — one pre-sized copy.
    #[napi]
    pub fn download_vec(&self) -> Buffer {
        self.inner.download_vec().into()
    }

    /// The whole device buffer as a host `Buffer` — the value alias of `downloadVec`.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.download_vec().into()
    }

    /// The [`GpuDevice`] this buffer's memory lives on.
    #[napi]
    pub fn device(&self) -> GpuDevice {
        GpuDevice {
            inner: self.inner.device().clone(),
        }
    }

    /// This buffer's device capacity snapshot — `device().memoryInfo()`.
    #[napi]
    pub fn memory_info(&self) -> MemoryInfo {
        MemoryInfo {
            inner: self.inner.memory_info(),
        }
    }

    // ---- core byte surface (bounded subset, mirroring Heap) ----------------------------

    /// The total length in bytes — an `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn byte_size(&self) -> i64 {
        self.inner.byte_size() as i64
    }

    /// Reads up to `length` bytes at `offset` into a fresh `Buffer` — short (or empty) near the
    /// end. Never moves the cursor.
    #[napi]
    pub fn pread_byte_array(&self, offset: u32, length: u32) -> Buffer {
        self.inner.pread_vec(offset as u64, length as usize).into()
    }

    /// Writes `data` at `offset`, growing the storage (and zero-filling any gap) as needed;
    /// returns the number of bytes written (always `data.length`). Never moves the cursor.
    #[napi]
    pub fn pwrite_byte_array(&mut self, offset: u32, data: Buffer) -> u32 {
        self.inner.pwrite_byte_array(offset as u64, data.as_ref()) as u32
    }

    /// **Bulk typed read** of `count` little-endian `i32`s at `offset` into a fresh array, or
    /// throws if fewer bytes remain — checked before allocating.
    #[napi]
    pub fn pread_i32_array(&self, offset: u32, count: u32) -> napi::Result<Vec<i32>> {
        check_bulk_read(self.inner.byte_size(), offset, count, 4)?;
        let mut values = vec![0i32; count as usize];
        self.inner
            .pread_i32_array(offset as u64, &mut values)
            .map_err(to_error)?;
        Ok(values)
    }

    /// **Bulk typed write** of all of `values` as little-endian `i32`s at `offset`, growing as
    /// needed — the vectorized bulk kernel on device memory.
    #[napi]
    pub fn pwrite_i32_array(&mut self, offset: u32, values: Vec<i32>) -> napi::Result<()> {
        self.inner
            .pwrite_i32_array(offset as u64, &values)
            .map_err(to_error)
    }

    /// **Bulk typed read** of `count` little-endian `i64`s at `offset` into a fresh array, or
    /// throws if fewer bytes remain — checked before allocating. Each JS `number` is exact only
    /// up to ±2^53.
    #[napi]
    pub fn pread_i64_array(&self, offset: u32, count: u32) -> napi::Result<Vec<i64>> {
        check_bulk_read(self.inner.byte_size(), offset, count, 8)?;
        let mut values = vec![0i64; count as usize];
        self.inner
            .pread_i64_array(offset as u64, &mut values)
            .map_err(to_error)?;
        Ok(values)
    }

    /// **Bulk typed write** of all of `values` as little-endian `i64`s at `offset`, growing as
    /// needed. Keep each value below ±2^53 so the JS `number`s stay exact.
    #[napi]
    pub fn pwrite_i64_array(&mut self, offset: u32, values: Vec<i64>) -> napi::Result<()> {
        self.inner
            .pwrite_i64_array(offset as u64, &values)
            .map_err(to_error)
    }

    // ---- aggregations, filter, and device-aware copy -----------------------------------
    //
    // The reductions delegate to the core `Aggregate` blanket trait (in scope via the
    // `Aggregate` import); the device-aware `computeBackend` / `computeCopyInto` stay on the
    // gpu `Compute` trait. Each op runs the dense vectorized reduction streamed through a fixed
    // stack chunk (a GPU-backed source overrides it with a device kernel). `offset` / `count`
    // cross as `u32` like the bulk byte surface; a `count` past the buffer throws the core's
    // guided EOF text. 64-bit crossings follow the buffer's convention — an `i64` accumulator
    // is a JS number (exact to 2^53), an `i128` sum is a `BigInt`, and an `i64` threshold is a
    // `BigInt`; `f32` values widen to `f64` like `preadF32`.

    /// **Sum** of `count` `i32`s at `offset` (accumulated as `i64`) — auto-dispatched (GPU when
    /// large + on a device, else the vectorized CPU reduction). An `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn sum_i32(&self, offset: u32, count: u32) -> napi::Result<i64> {
        self.inner
            .sum_i32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Sum** of `count` `i64`s at `offset` (accumulated as `i128`, a `BigInt`) — auto-dispatched.
    #[napi]
    pub fn sum_i64(&self, offset: u32, count: u32) -> napi::Result<i128> {
        self.inner
            .sum_i64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Sum** of `count` `f32`s at `offset` (accumulated as `f64`) — auto-dispatched.
    #[napi]
    pub fn sum_f32(&self, offset: u32, count: u32) -> napi::Result<f64> {
        self.inner
            .sum_f32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Sum** of `count` `f64`s at `offset` — auto-dispatched.
    #[napi]
    pub fn sum_f64(&self, offset: u32, count: u32) -> napi::Result<f64> {
        self.inner
            .sum_f64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Minimum** of `count` `i32`s at `offset`, or `null` when `count == 0` — auto-dispatched.
    #[napi]
    pub fn min_i32(&self, offset: u32, count: u32) -> napi::Result<Option<i32>> {
        self.inner
            .min_i32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Minimum** of `count` `i64`s at `offset` (a JS number, exact to 2^53), or `null` when
    /// `count == 0` — auto-dispatched.
    #[napi]
    pub fn min_i64(&self, offset: u32, count: u32) -> napi::Result<Option<i64>> {
        self.inner
            .min_i64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Minimum** of `count` `f32`s at `offset` (widened to a JS number), or `null` when
    /// `count == 0` — auto-dispatched.
    #[napi]
    pub fn min_f32(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .min_f32(offset as u64, count as usize)
            .map(|opt| opt.map(|v| v as f64))
            .map_err(to_error)
    }

    /// **Minimum** of `count` `f64`s at `offset`, or `null` when `count == 0` — auto-dispatched.
    #[napi]
    pub fn min_f64(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .min_f64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Maximum** of `count` `i32`s at `offset`, or `null` when `count == 0` — auto-dispatched.
    #[napi]
    pub fn max_i32(&self, offset: u32, count: u32) -> napi::Result<Option<i32>> {
        self.inner
            .max_i32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Maximum** of `count` `i64`s at `offset` (a JS number, exact to 2^53), or `null` when
    /// `count == 0` — auto-dispatched.
    #[napi]
    pub fn max_i64(&self, offset: u32, count: u32) -> napi::Result<Option<i64>> {
        self.inner
            .max_i64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Maximum** of `count` `f32`s at `offset` (widened to a JS number), or `null` when
    /// `count == 0` — auto-dispatched.
    #[napi]
    pub fn max_f32(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .max_f32(offset as u64, count as usize)
            .map(|opt| opt.map(|v| v as f64))
            .map_err(to_error)
    }

    /// **Maximum** of `count` `f64`s at `offset`, or `null` when `count == 0` — auto-dispatched.
    #[napi]
    pub fn max_f64(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .max_f64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Mean** of `count` `i32`s at `offset` as `f64`, or `null` when `count == 0` — auto-dispatched.
    #[napi]
    pub fn mean_i32(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .mean_i32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Mean** of `count` `i64`s at `offset` as `f64`, or `null` when `count == 0` — auto-dispatched.
    #[napi]
    pub fn mean_i64(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .mean_i64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Mean** of `count` `f32`s at `offset` as `f64`, or `null` when `count == 0` — auto-dispatched.
    #[napi]
    pub fn mean_f32(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .mean_f32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Mean** of `count` `f64`s at `offset` as `f64`, or `null` when `count == 0` — auto-dispatched.
    #[napi]
    pub fn mean_f64(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .mean_f64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Population standard deviation** of `count` `i32`s at `offset` as `f64`, or `null` when
    /// `count == 0`.
    #[napi]
    pub fn std_i32(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .std_i32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Population standard deviation** of `count` `i64`s at `offset` as `f64`, or `null` when
    /// `count == 0`.
    #[napi]
    pub fn std_i64(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .std_i64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Population standard deviation** of `count` `f32`s at `offset` as `f64`, or `null` when
    /// `count == 0`.
    #[napi]
    pub fn std_f32(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .std_f32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Population standard deviation** of `count` `f64`s at `offset` as `f64`, or `null` when
    /// `count == 0`.
    #[napi]
    pub fn std_f64(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .std_f64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// The **first** `i32` at `offset`, or `null` when `count == 0`.
    #[napi]
    pub fn first_i32(&self, offset: u32, count: u32) -> napi::Result<Option<i32>> {
        self.inner
            .first_i32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// The **last** `i32` of the `count` at `offset`, or `null` when `count == 0`.
    #[napi]
    pub fn last_i32(&self, offset: u32, count: u32) -> napi::Result<Option<i32>> {
        self.inner
            .last_i32(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// The **first** `i64` at `offset` (a JS number, exact to 2^53), or `null` when `count == 0`.
    #[napi]
    pub fn first_i64(&self, offset: u32, count: u32) -> napi::Result<Option<i64>> {
        self.inner
            .first_i64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// The **last** `i64` of the `count` at `offset` (a JS number), or `null` when `count == 0`.
    #[napi]
    pub fn last_i64(&self, offset: u32, count: u32) -> napi::Result<Option<i64>> {
        self.inner
            .last_i64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// The **first** `f32` at `offset` (widened to a JS number), or `null` when `count == 0`.
    #[napi]
    pub fn first_f32(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .first_f32(offset as u64, count as usize)
            .map(|opt| opt.map(|v| v as f64))
            .map_err(to_error)
    }

    /// The **last** `f32` of the `count` at `offset` (widened to a JS number), or `null` when
    /// `count == 0`.
    #[napi]
    pub fn last_f32(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .last_f32(offset as u64, count as usize)
            .map(|opt| opt.map(|v| v as f64))
            .map_err(to_error)
    }

    /// The **first** `f64` at `offset`, or `null` when `count == 0`.
    #[napi]
    pub fn first_f64(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .first_f64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// The **last** `f64` of the `count` at `offset`, or `null` when `count == 0`.
    #[napi]
    pub fn last_f64(&self, offset: u32, count: u32) -> napi::Result<Option<f64>> {
        self.inner
            .last_f64(offset as u64, count as usize)
            .map_err(to_error)
    }

    /// **Filter count** — how many of `count` `i32`s at `offset` are `>= threshold`. An `i64`
    /// (a JS number, exact to 2^53).
    #[napi]
    pub fn count_ge_i32(&self, offset: u32, count: u32, threshold: i32) -> napi::Result<i64> {
        self.inner
            .count_ge_i32(offset as u64, count as usize, threshold)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// **Filter count** — how many of `count` `i64`s at `offset` are `>= threshold` (a `BigInt`).
    /// An `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn count_ge_i64(&self, offset: u32, count: u32, threshold: BigInt) -> napi::Result<i64> {
        self.inner
            .count_ge_i64(offset as u64, count as usize, threshold.get_i64().0)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// **Filter count** — how many of `count` `f32`s at `offset` are `>= threshold`. An `i64`
    /// (a JS number, exact to 2^53).
    #[napi]
    pub fn count_ge_f32(&self, offset: u32, count: u32, threshold: f64) -> napi::Result<i64> {
        self.inner
            .count_ge_f32(offset as u64, count as usize, threshold as f32)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// **Filter count** — how many of `count` `f64`s at `offset` are `>= threshold`. An `i64`
    /// (a JS number, exact to 2^53).
    #[napi]
    pub fn count_ge_f64(&self, offset: u32, count: u32, threshold: f64) -> napi::Result<i64> {
        self.inner
            .count_ge_f64(offset as u64, count as usize, threshold)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The backend token an op over `elements` values would run on — `"gpu"` when this buffer is
    /// on a real device *and* `elements` clears the transfer threshold, else `"cpu"`.
    #[napi]
    pub fn compute_backend(&self, elements: u32) -> String {
        self.inner
            .compute_backend(elements as usize)
            .as_str()
            .to_string()
    }

    /// **Device-aware copy** — copies this buffer's whole content into `dst` (a same-device
    /// GPU→GPU copy would run as a device DMA; else the zero-copy host copy) and returns the
    /// byte count. An `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn compute_copy_into(&self, dst: &mut AmdBuffer) -> napi::Result<i64> {
        self.inner
            .compute_copy_into(&mut dst.inner)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Releases the device buffer's backing storage — resets it to empty. The JS
    /// explicit-resource-management disposer (an explicit `buf.dispose()` frees the memory).
    #[napi]
    pub fn dispose(&mut self) {
        self.inner = core::AmdBuffer::new();
    }
}
