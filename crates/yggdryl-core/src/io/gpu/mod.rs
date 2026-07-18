//! `io::gpu` — the **device-memory** layer (feature `gpu`), organized **by GPU architecture**.
//!
//! Device memory that **is an [`IOBase`]**: a [`GpuMemory`] buffer reads, writes, and runs the
//! vectorized bulk numeric kernels exactly as a `Heap` does, plus a host↔device
//! [`upload`](GpuMemory::upload) / [`download`](GpuMemory::download) transfer. The layer is split
//! by architecture — [`cpu`] (the portable backend, where device memory **is our
//! [`Heap`](crate::io::memory::Heap)**, aliased [`CpuHeap`]), `amd` (AMD Radeon, feature
//! `gpu-amd`), `cuda` (NVIDIA, feature `gpu-cuda`) — and [`available_devices`] **adapts to the
//! hardware present**. Every device reports its capacity as a
//! [`MemoryInfo`](crate::io::MemoryInfo), the same value type [`LocalIO`](crate::io::local::LocalIO)
//! reports for disk, so "how much room is there?" is answered uniformly across backends.
//!
//! ```
//! use yggdryl_core::io::gpu::{available_devices, default_device, CpuHeap, GpuMemory};
//! use yggdryl_core::io::memory::IOBase;
//!
//! // Adapt to what's available, allocate device memory (a CpuHeap == our Heap on the CPU
//! // device), and run IO + SIMD bulk ops on it.
//! assert!(!available_devices().is_empty());
//! let mut dev = CpuHeap::new();
//! dev.upload(b"device bytes").unwrap();
//! dev.pwrite_f32_array(16, &[1.5, 2.5, 3.5]).unwrap(); // vectorized, on device memory
//! assert_eq!(&dev.download_vec()[..12], b"device bytes");
//! assert!(dev.device().is_cpu());
//! // Every device reports a capacity snapshot (total >= available within a device).
//! let info = default_device().memory_info();
//! assert!(info.total() >= info.available());
//! ```

use super::MemoryInfo;
use crate::io::memory::{IOBase, IoError};

pub mod cpu;
mod device;

#[cfg(feature = "gpu-amd")]
pub mod amd;
#[cfg(feature = "gpu-cuda")]
pub mod cuda;

pub use cpu::CpuHeap;
pub use device::{available_devices, default_device, GpuBackend, GpuDevice};

#[cfg(feature = "gpu-amd")]
pub use amd::AmdBuffer;

/// **Device memory that speaks the full [`IOBase`] byte contract**, plus the host↔device
/// transfer. A `GpuMemory` buffer inherits every typed/bulk/cursor operation from `IOBase` (the
/// auto-vectorized bulk numeric kernels included), and adds:
///
/// - [`upload`](GpuMemory::upload): copy a host slice **into** device memory (replacing content),
/// - [`download`](GpuMemory::download) / [`download_vec`](GpuMemory::download_vec): copy device
///   memory **back** to the host,
/// - [`device`](GpuMemory::device): which [`GpuDevice`] this buffer's memory lives on.
///
/// For the CPU backend ([`CpuHeap`]) these transfers are memcpys; a real GPU backend
/// ([`AmdBuffer`](amd::AmdBuffer)) maps them onto the device queue.
pub trait GpuMemory: IOBase {
    /// The device this buffer's memory lives on.
    fn device(&self) -> &GpuDevice;

    /// This device's live capacity snapshot — a convenience for `self.device().memory_info()`.
    fn memory_info(&self) -> MemoryInfo {
        self.device().memory_info()
    }

    /// **Uploads** `host` into device memory, replacing the whole content (and syncing the size
    /// headers, via [`overwrite_with`](IOBase::overwrite_with)). The "copy this array to the GPU"
    /// entry point; positioned partial updates use the ordinary `pwrite_*` surface.
    fn upload(&mut self, host: &[u8]) -> Result<(), IoError> {
        self.overwrite_with(host)
    }

    /// **Downloads** device memory into `out`, returning the number of bytes copied (short when
    /// `out` is larger than the buffer). Positioned, from the start.
    fn download(&self, out: &mut [u8]) -> usize {
        self.pread_byte_array(0, out)
    }

    /// **Downloads** the whole device buffer into a fresh host `Vec` — one pre-sized allocation.
    fn download_vec(&self) -> Vec<u8> {
        self.pread_vec(0, self.byte_size() as usize)
    }
}
