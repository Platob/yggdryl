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

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use crate::io::meminfo::MemoryInfo;
use crate::io::memory::{check_bulk_read, to_error};
use yggdryl_core::io::gpu::{self as core, GpuMemory};
use yggdryl_core::io::memory::IOBase;

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
    /// `Buffer` — short when `length` exceeds the buffer.
    #[napi]
    pub fn download(&self, length: u32) -> Buffer {
        let mut out = vec![0u8; length as usize];
        let read = self.inner.download(&mut out);
        out.truncate(read);
        out.into()
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

    /// Releases the device buffer's backing storage — resets it to empty. The JS
    /// explicit-resource-management disposer (an explicit `buf.dispose()` frees the memory).
    #[napi]
    pub fn dispose(&mut self) {
        self.inner = core::AmdBuffer::new();
    }
}
