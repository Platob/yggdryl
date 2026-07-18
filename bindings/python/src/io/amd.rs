//! The `yggdryl.amd` submodule — the **AMD Radeon device-memory** family.
//!
//! Mirrors `yggdryl_core::io::amd`. [`detect`] **adapts to the hardware present** — it returns an
//! [`AmdDevice`] only on a real Radeon adapter, else `None`. An [`AmdDevice`] is a value
//! description of the AMD compute device (its name, total VRAM, and whether a real adapter backs
//! it), and an [`AmdHeap`] is device memory over the detected adapter (or the host-memory fallback
//! when none is present) that **is an `IOBase`** — it reads, writes, and runs the vectorized bulk
//! numeric kernels exactly as a `yggdryl.memory.Heap` does, plus the host↔device
//! [`upload`](AmdHeap::upload) / [`download`](AmdHeap::download) transfer and the GPU-vs-CPU
//! [`compute_backend`](AmdHeap::compute_backend) dispatch.
//!
//! Every method is one or two lines over `yggdryl_core`; a read with a hard length requirement
//! that runs off the end raises a guided `ValueError` carrying the core error text unchanged.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedBytes;
use pyo3::types::PyBytes;

use crate::io::meminfo::MemoryInfo;
use crate::io::memory::bulk_eof;
use yggdryl_core::io::amd::{self as core, AmdMemory};
// The statistical aggregations live on the `Aggregate` blanket trait over any `IOBase`; import it
// so `sum_i32` / `std_i32` / … resolve on `AmdHeap`, and `IOBase` for the byte surface.
use yggdryl_core::io::memory::{Aggregate, IOBase, IoError};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Probes for an **AMD Radeon** device, returning its [`AmdDevice`] (name + VRAM) when a real
/// adapter is present, else `None`. Defensive — any platform-query failure yields `None`.
#[pyfunction]
fn detect() -> Option<AmdDevice> {
    core::detect().map(|inner| AmdDevice { inner })
}

/// A **value description of the AMD compute device** — its human name and total VRAM, plus whether
/// a real Radeon adapter backs it ([`is_present`](AmdDevice::is_present)) or it is the host-memory
/// fallback. A plain value: equal, hashable, and keys a map / sits in a set. Live free-memory is a
/// fresh [`memory_info`](AmdDevice::memory_info) query, not baked into the descriptor.
#[pyclass(module = "yggdryl.amd")]
#[derive(Clone)]
pub struct AmdDevice {
    pub(crate) inner: core::AmdDevice,
}

#[pymethods]
impl AmdDevice {
    /// The human-readable device name (the driver description for a real adapter).
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The total device memory in bytes (VRAM for a real adapter, host RAM for the fallback).
    fn total_memory(&self) -> u64 {
        self.inner.total_memory()
    }

    /// Whether a **real AMD Radeon adapter** backs this device (vs the host-memory fallback).
    fn is_present(&self) -> bool {
        self.inner.is_present()
    }

    /// A **live capacity snapshot** for this device — a present adapter reports its total VRAM;
    /// the fallback queries host RAM fresh.
    fn memory_info(&self) -> MemoryInfo {
        MemoryInfo {
            inner: self.inner.memory_info(),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    fn __repr__(&self) -> String {
        format!(
            "AmdDevice(name={:?}, total_memory={}, present={})",
            self.inner.name(),
            self.inner.total_memory(),
            if self.inner.is_present() {
                "True"
            } else {
                "False"
            }
        )
    }
}

/// An **AMD Radeon device-memory heap** — device memory over the detected AMD adapter (or the
/// host-memory fallback when none is present) that **is an `IOBase`**: it carries the full
/// positioned / bulk byte surface (`pread_byte_array` / `pwrite_byte_array`, the vectorized
/// `pwrite_i32_array` / `pread_i32_array` / `pwrite_i64_array` / `pread_i64_array`), plus the
/// host↔device [`upload`](AmdHeap::upload) / [`download`](AmdHeap::download) transfer.
#[pyclass(module = "yggdryl.amd")]
#[derive(Clone)]
pub struct AmdHeap {
    pub(crate) inner: core::AmdHeap,
}

#[pymethods]
impl AmdHeap {
    /// An empty AMD device heap on the detected AMD device (or the host fallback when none).
    #[new]
    fn new() -> Self {
        Self {
            inner: core::AmdHeap::new(),
        }
    }

    /// An empty heap with room for `capacity` bytes before reallocating.
    #[staticmethod]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: core::AmdHeap::with_capacity(capacity),
        }
    }

    /// A heap initialized by **uploading** `data` (bytes / bytearray) — host → device.
    #[staticmethod]
    fn from_host(data: PyBackedBytes) -> Self {
        Self {
            inner: core::AmdHeap::from_host(&data),
        }
    }

    // ---- host <-> device transfer ------------------------------------------------------

    /// **Uploads** `host` (bytes / bytearray) into device memory, replacing the whole content
    /// (and syncing the size headers). The "copy this array to the GPU" entry point.
    fn upload(&mut self, host: PyBackedBytes) -> PyResult<()> {
        self.inner.upload(&host).map_err(ioerr)
    }

    /// **Downloads** up to `length` bytes of device memory (from the start) into a fresh
    /// `bytes` — short when `length` exceeds the buffer.
    fn download<'py>(&self, py: Python<'py>, length: usize) -> PyResult<Bound<'py, PyBytes>> {
        let n = self.inner.byte_size().min(length as u64) as usize;
        PyBytes::new_bound_with(py, n, |dst| {
            self.inner.download(dst);
            Ok(())
        })
    }

    /// **Downloads** the whole device buffer into a fresh `bytes` — one pre-sized allocation.
    fn download_vec<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.download_vec())
    }

    /// The whole device buffer as a `bytes` copy — an alias of
    /// [`download_vec`](AmdHeap::download_vec) (so `to_bytes()` reads naturally).
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.download_vec())
    }

    /// The whole device buffer as a `bytes` copy (so `bytes(buffer)` works).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.download_vec())
    }

    /// The [`AmdDevice`] this heap's memory lives on (a clone of the heap's device).
    fn device(&self) -> AmdDevice {
        AmdDevice {
            inner: self.inner.device().clone(),
        }
    }

    /// This device's live capacity snapshot — a convenience for `device().memory_info()`.
    fn memory_info(&self) -> MemoryInfo {
        MemoryInfo {
            inner: self.inner.memory_info(),
        }
    }

    // ---- size --------------------------------------------------------------------------

    /// The total length in bytes.
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The total length in bytes (so `len(buffer)` works).
    fn __len__(&self) -> usize {
        self.inner.byte_size() as usize
    }

    /// Whether the buffer holds no bytes (`byte_size() == 0`).
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Truthiness — `True` when the buffer holds at least one byte.
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    // ---- positioned byte-array ---------------------------------------------------------

    /// **Positioned read.** Returns up to `length` bytes starting at `offset` as `bytes` —
    /// short near the end, empty at or past it. Reads directly into the `bytes` allocation.
    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let n = self
            .inner
            .byte_size()
            .saturating_sub(offset)
            .min(length as u64) as usize;
        PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(offset, dst);
            Ok(())
        })
    }

    /// **Positioned write.** Copies `data` (bytes / bytearray) in at `offset`, growing the
    /// buffer and zero-filling any gap; returns the number of bytes written.
    fn pwrite_byte_array(&mut self, offset: u64, data: PyBackedBytes) -> usize {
        self.inner.pwrite_byte_array(offset, &data)
    }

    // ---- bulk typed arrays (i32 / i64) -------------------------------------------------

    /// **Bulk typed read.** Returns `count` little-endian `i32`s starting at `offset` as a
    /// list, raising `ValueError` if fewer bytes remain — checked **before** the result is
    /// allocated, so a hostile `count` fails fast instead of allocating.
    fn pread_i32_array(&self, offset: u64, count: usize) -> PyResult<Vec<i32>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 4) {
            return Err(ioerr(e));
        }
        let mut values = vec![0i32; count];
        self.inner
            .pread_i32_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write.** Writes all of `values` as little-endian `i32`s at `offset`,
    /// growing as needed — a vectorized bulk op on device memory.
    fn pwrite_i32_array(&mut self, offset: u64, values: Vec<i32>) -> PyResult<()> {
        self.inner.pwrite_i32_array(offset, &values).map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `i64`s — the wide counterpart of
    /// [`pread_i32_array`](AmdHeap::pread_i32_array), with the same fail-fast bounds check.
    fn pread_i64_array(&self, offset: u64, count: usize) -> PyResult<Vec<i64>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 8) {
            return Err(ioerr(e));
        }
        let mut values = vec![0i64; count];
        self.inner
            .pread_i64_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `i64`s — the wide counterpart of
    /// [`pwrite_i32_array`](AmdHeap::pwrite_i32_array).
    fn pwrite_i64_array(&mut self, offset: u64, values: Vec<i64>) -> PyResult<()> {
        self.inner.pwrite_i64_array(offset, &values).map_err(ioerr)
    }

    // ---- compute: auto-dispatched aggregations, filter, device-aware copy --------------

    /// **Sum** of `count` little-endian `i32`s at `offset` (accumulated as a 64-bit int).
    fn sum_i32(&self, offset: u64, count: usize) -> PyResult<i64> {
        self.inner.sum_i32(offset, count).map_err(ioerr)
    }

    /// **Sum** of `count` little-endian `i64`s at `offset` (accumulated as a 128-bit int).
    fn sum_i64(&self, offset: u64, count: usize) -> PyResult<i128> {
        self.inner.sum_i64(offset, count).map_err(ioerr)
    }

    /// **Sum** of `count` little-endian `f32`s at `offset` (accumulated as `f64`).
    fn sum_f32(&self, offset: u64, count: usize) -> PyResult<f64> {
        self.inner.sum_f32(offset, count).map_err(ioerr)
    }

    /// **Sum** of `count` little-endian `f64`s at `offset`.
    fn sum_f64(&self, offset: u64, count: usize) -> PyResult<f64> {
        self.inner.sum_f64(offset, count).map_err(ioerr)
    }

    /// **Minimum** of `count` `i32`s at `offset`, or `None` when `count == 0`.
    fn min_i32(&self, offset: u64, count: usize) -> PyResult<Option<i32>> {
        self.inner.min_i32(offset, count).map_err(ioerr)
    }

    /// **Minimum** of `count` `i64`s at `offset`, or `None` when `count == 0`.
    fn min_i64(&self, offset: u64, count: usize) -> PyResult<Option<i64>> {
        self.inner.min_i64(offset, count).map_err(ioerr)
    }

    /// **Minimum** of `count` `f32`s at `offset`, or `None` when `count == 0`.
    fn min_f32(&self, offset: u64, count: usize) -> PyResult<Option<f32>> {
        self.inner.min_f32(offset, count).map_err(ioerr)
    }

    /// **Minimum** of `count` `f64`s at `offset`, or `None` when `count == 0`.
    fn min_f64(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.min_f64(offset, count).map_err(ioerr)
    }

    /// **Maximum** of `count` `i32`s at `offset`, or `None` when `count == 0`.
    fn max_i32(&self, offset: u64, count: usize) -> PyResult<Option<i32>> {
        self.inner.max_i32(offset, count).map_err(ioerr)
    }

    /// **Maximum** of `count` `i64`s at `offset`, or `None` when `count == 0`.
    fn max_i64(&self, offset: u64, count: usize) -> PyResult<Option<i64>> {
        self.inner.max_i64(offset, count).map_err(ioerr)
    }

    /// **Maximum** of `count` `f32`s at `offset`, or `None` when `count == 0`.
    fn max_f32(&self, offset: u64, count: usize) -> PyResult<Option<f32>> {
        self.inner.max_f32(offset, count).map_err(ioerr)
    }

    /// **Maximum** of `count` `f64`s at `offset`, or `None` when `count == 0`.
    fn max_f64(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.max_f64(offset, count).map_err(ioerr)
    }

    /// **Mean** of `count` `i32`s at `offset` as `float`, or `None` when `count == 0`.
    fn mean_i32(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.mean_i32(offset, count).map_err(ioerr)
    }

    /// **Mean** of `count` `i64`s at `offset` as `float`, or `None` when `count == 0`.
    fn mean_i64(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.mean_i64(offset, count).map_err(ioerr)
    }

    /// **Mean** of `count` `f32`s at `offset` as `float`, or `None` when `count == 0`.
    fn mean_f32(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.mean_f32(offset, count).map_err(ioerr)
    }

    /// **Mean** of `count` `f64`s at `offset` as `float`, or `None` when `count == 0`.
    fn mean_f64(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.mean_f64(offset, count).map_err(ioerr)
    }

    /// **Filter count** — how many of `count` `i32`s at `offset` are `>= threshold`.
    fn count_ge_i32(&self, offset: u64, count: usize, threshold: i32) -> PyResult<usize> {
        self.inner
            .count_ge_i32(offset, count, threshold)
            .map_err(ioerr)
    }

    /// **Filter count** — how many of `count` `i64`s at `offset` are `>= threshold`.
    fn count_ge_i64(&self, offset: u64, count: usize, threshold: i64) -> PyResult<usize> {
        self.inner
            .count_ge_i64(offset, count, threshold)
            .map_err(ioerr)
    }

    /// **Filter count** — how many of `count` `f32`s at `offset` are `>= threshold`.
    fn count_ge_f32(&self, offset: u64, count: usize, threshold: f32) -> PyResult<usize> {
        self.inner
            .count_ge_f32(offset, count, threshold)
            .map_err(ioerr)
    }

    /// **Filter count** — how many of `count` `f64`s at `offset` are `>= threshold`.
    fn count_ge_f64(&self, offset: u64, count: usize, threshold: f64) -> PyResult<usize> {
        self.inner
            .count_ge_f64(offset, count, threshold)
            .map_err(ioerr)
    }

    /// **Population standard deviation** of `count` `i32`s at `offset` as `float`, or `None` when
    /// `count == 0`.
    fn std_i32(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.std_i32(offset, count).map_err(ioerr)
    }

    /// **Population standard deviation** of `count` `i64`s at `offset` as `float`, or `None` when
    /// `count == 0`.
    fn std_i64(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.std_i64(offset, count).map_err(ioerr)
    }

    /// **Population standard deviation** of `count` `f32`s at `offset` as `float`, or `None` when
    /// `count == 0`.
    fn std_f32(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.std_f32(offset, count).map_err(ioerr)
    }

    /// **Population standard deviation** of `count` `f64`s at `offset` as `float`, or `None` when
    /// `count == 0`.
    fn std_f64(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.std_f64(offset, count).map_err(ioerr)
    }

    /// The **first** `i32` at `offset`, or `None` when `count == 0`.
    fn first_i32(&self, offset: u64, count: usize) -> PyResult<Option<i32>> {
        self.inner.first_i32(offset, count).map_err(ioerr)
    }

    /// The **first** `i64` at `offset`, or `None` when `count == 0`.
    fn first_i64(&self, offset: u64, count: usize) -> PyResult<Option<i64>> {
        self.inner.first_i64(offset, count).map_err(ioerr)
    }

    /// The **first** `f32` at `offset`, or `None` when `count == 0`.
    fn first_f32(&self, offset: u64, count: usize) -> PyResult<Option<f32>> {
        self.inner.first_f32(offset, count).map_err(ioerr)
    }

    /// The **first** `f64` at `offset`, or `None` when `count == 0`.
    fn first_f64(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.first_f64(offset, count).map_err(ioerr)
    }

    /// The **last** `i32` of the `count` at `offset`, or `None` when `count == 0`.
    fn last_i32(&self, offset: u64, count: usize) -> PyResult<Option<i32>> {
        self.inner.last_i32(offset, count).map_err(ioerr)
    }

    /// The **last** `i64` of the `count` at `offset`, or `None` when `count == 0`.
    fn last_i64(&self, offset: u64, count: usize) -> PyResult<Option<i64>> {
        self.inner.last_i64(offset, count).map_err(ioerr)
    }

    /// The **last** `f32` of the `count` at `offset`, or `None` when `count == 0`.
    fn last_f32(&self, offset: u64, count: usize) -> PyResult<Option<f32>> {
        self.inner.last_f32(offset, count).map_err(ioerr)
    }

    /// The **last** `f64` of the `count` at `offset`, or `None` when `count == 0`.
    fn last_f64(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
        self.inner.last_f64(offset, count).map_err(ioerr)
    }

    /// The backend the next op over `elements` values runs on — the token `"gpu"` (device
    /// kernel, when on a real adapter and the workload amortizes the transfer) or `"cpu"` (the
    /// vectorized host reduction).
    fn compute_backend(&self, elements: usize) -> String {
        self.inner.compute_backend(elements).as_str().to_string()
    }

    /// **Device-aware copy** of this buffer's whole content into `dst`; returns the byte count.
    fn compute_copy_into(&self, dst: &mut AmdHeap) -> PyResult<u64> {
        self.inner.compute_copy_into(&mut dst.inner).map_err(ioerr)
    }

    // ---- context manager + repr --------------------------------------------------------

    /// Context-manager entry — returns the heap itself, so `with AmdHeap() as buf:` binds it.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit — a no-op for the host-staged device heap (nothing to release);
    /// returns `False` so exceptions propagate.
    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        false
    }

    fn __repr__(&self) -> String {
        format!(
            "AmdHeap(<{} bytes on {}>)",
            self.inner.byte_size(),
            self.inner.device().name()
        )
    }
}

/// Populates the `amd` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(detect, module)?)?;
    module.add_class::<AmdDevice>()?;
    module.add_class::<AmdHeap>()?;
    Ok(())
}
