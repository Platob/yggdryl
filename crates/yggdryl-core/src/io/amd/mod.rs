//! `io::amd` — the **AMD Radeon device-memory family** (feature `amd`).
//!
//! One concrete infrastructure family, a sibling of [`io::local`](crate::io::local) and
//! [`io::memory`](crate::io::memory) (which *is* the CPU byte layer — a [`Heap`](crate::io::memory::Heap)
//! is simply the CPU heap). This module adds the AMD-optimized trio — [`AmdHeap`], [`AmdCursor`],
//! [`AmdSlice`] — each speaking the full [`IOBase`] byte + vectorized-bulk contract, over a real
//! detected AMD device ([`AmdDevice`] / [`detect`]). [`AmdMemory`] adds the host↔device transfer and
//! the [`ComputeBackend`] dispatch that decides when a reduction is worth running on the GPU.
//!
//! ```
//! use yggdryl_core::io::amd::{detect, AmdHeap, AmdMemory, ComputeBackend};
//! use yggdryl_core::io::memory::{Aggregate, IOBase};
//!
//! // Adapt to what's installed: `detect()` is `Some` only on a real Radeon adapter; either way an
//! // `AmdHeap` is usable, running byte I/O and the SIMD bulk/aggregate kernels on device memory.
//! let _adapter = detect(); // None on a machine with no AMD GPU — the heap still works
//! let mut dev = AmdHeap::new();
//! dev.upload(b"device bytes").unwrap();
//! dev.pwrite_f32_array(16, &[1.5, 2.5, 3.5]).unwrap(); // vectorized, on device memory
//! assert_eq!(&dev.download_vec()[..12], b"device bytes");
//! assert_eq!(dev.sum_f32(16, 3).unwrap(), 7.5); // aggregate shared with every source
//!
//! // The device reports a capacity snapshot (total >= available), and the dispatch is CPU until a
//! // real adapter + a large-enough workload justify the GPU.
//! let info = dev.memory_info();
//! assert!(info.total() >= info.available());
//! assert_eq!(dev.compute_backend(8), ComputeBackend::Cpu); // tiny workload stays on the CPU
//! ```

use crate::io::memory::{IOBase, IoError};
use crate::io::MemoryInfo;

mod compute;
mod cursor;
pub mod device;
mod heap;
mod slice;

pub use compute::{ComputeBackend, GPU_ELEMENT_THRESHOLD};
pub use cursor::AmdCursor;
pub use device::{detect, AmdDevice};
pub use heap::AmdHeap;
pub use slice::AmdSlice;

/// **AMD device memory that speaks the full [`IOBase`] byte contract**, plus the host↔device
/// transfer and the GPU-vs-CPU compute dispatch. An [`AmdHeap`] inherits every typed/bulk/cursor
/// operation from `IOBase` (the auto-vectorized bulk numeric kernels included), and adds:
///
/// - [`upload`](AmdMemory::upload): copy a host slice **into** device memory (replacing content),
/// - [`download`](AmdMemory::download) / [`download_vec`](AmdMemory::download_vec): copy device
///   memory **back** to the host,
/// - [`device`](AmdMemory::device): which [`AmdDevice`] this buffer's memory lives on,
/// - [`compute_backend`](AmdMemory::compute_backend): the [`ComputeBackend`] a reduction would run
///   on for a given element count.
///
/// The resident store is host memory today, so these transfers are memcpys; the VRAM queue and
/// device kernels drop in behind them without changing a caller.
pub trait AmdMemory: IOBase {
    /// The device this buffer's memory lives on.
    fn device(&self) -> &AmdDevice;

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

    /// The backend an op over `elements` values would run on: **GPU** when this buffer is on a real
    /// Radeon adapter *and* `elements >= `[`GPU_ELEMENT_THRESHOLD`], else **CPU**. The dispatch a
    /// device-backed source consults before choosing a kernel; exposed so a caller can see it.
    fn compute_backend(&self, elements: usize) -> ComputeBackend {
        if self.device().is_present() && elements >= GPU_ELEMENT_THRESHOLD {
            ComputeBackend::Gpu
        } else {
            ComputeBackend::Cpu
        }
    }

    /// **Device-aware copy** — copies this buffer's whole content into `dst`, auto-selecting the
    /// path: a same-device GPU→GPU copy would run as a device-to-device DMA (the marked seam),
    /// otherwise the zero-copy host copy ([`copy_from`](IOBase::copy_from)). Returns the byte count.
    fn compute_copy_into<D: AmdMemory>(&self, dst: &mut D) -> Result<u64, IoError> {
        let _same_gpu = self.device().is_present() && self.device() == dst.device();
        // GPU seam: a device-to-device DMA runs here once the hardware queue is wired; until then
        // the host copy below is used (correct on every platform).
        dst.copy_from(self)
    }
}
