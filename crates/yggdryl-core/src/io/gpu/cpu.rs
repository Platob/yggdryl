//! The **CPU** architecture — device memory realized as **host RAM**, which is exactly our
//! [`Heap`](crate::io::memory::Heap): with the `gpu` feature a `Heap` **is** a [`GpuMemory`]
//! buffer on the [`GpuDevice::cpu`] device. [`CpuHeap`] is the name that layer uses for it.
//!
//! Unifying the CPU backend with `Heap` means the always-available fallback needs no wrapper: it
//! is the same in-heap buffer, with the same vectorized bulk kernels and cursor, that the rest of
//! the crate already uses — so `upload`/`download` are memcpys and every `IOBase` op is native
//! (no delegation). A real GPU backend ([`AmdBuffer`](super::amd::AmdBuffer)) implements the same
//! [`GpuMemory`] trait behind its own feature.

use super::{GpuDevice, GpuMemory};
use crate::io::memory::Heap;

/// The **CPU device-memory** type — an alias for [`Heap`](crate::io::memory::Heap): host RAM *is*
/// the CPU backend's memory, so no wrapper is needed. Construct it exactly like a `Heap`
/// (`CpuHeap::new()`, `CpuHeap::with_capacity(n)`, `CpuHeap::from_slice(&data)`), then use the
/// full [`GpuMemory`] surface (`upload` / `download` / `device`) on top of the ordinary byte I/O.
pub type CpuHeap = Heap;

/// The process-wide **CPU device** descriptor — resolved once (host-RAM total sized from
/// [`MemoryInfo::system`](crate::io::MemoryInfo::system) at first use).
fn cpu_device() -> &'static GpuDevice {
    static DEVICE: std::sync::LazyLock<GpuDevice> = std::sync::LazyLock::new(GpuDevice::cpu);
    &DEVICE
}

/// A `Heap` **is** a CPU-backed [`GpuMemory`] buffer — device memory is host RAM, so `upload` /
/// `download` (from the trait defaults) are memcpys and its device is the shared [`cpu_device`].
impl GpuMemory for Heap {
    fn device(&self) -> &GpuDevice {
        cpu_device()
    }
}
