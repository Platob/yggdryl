//! [`Compute`] — the **GPU-vs-CPU dispatch decision** and the device-aware copy for [`GpuMemory`]
//! buffers.
//!
//! The statistical aggregations themselves (`sum` / `min` / `max` / `mean` / `std` / `first` /
//! `last` / `count_ge`, every numeric width) live on
//! [`Aggregate`](crate::io::memory::Aggregate) — a blanket trait over **every** [`IOBase`], so a
//! device buffer runs them exactly like a `Heap`. What is GPU-specific is *where* they should run:
//! [`compute_backend`](Compute::compute_backend) picks the **GPU** when the buffer is on a real
//! device *and* the workload is big enough to amortize the host↔device transfer, else the **CPU**.
//! It is the optimization seam — a device-backed source overrides the `Aggregate` methods with
//! device kernels and consults this to decide when to use them. [`compute_copy_into`](Compute::compute_copy_into)
//! is the matching device-aware transfer.

use super::{GpuBackend, GpuMemory};
use crate::io::memory::IoError;

/// Elements at or above this count make a GPU run worth the host↔device transfer — the
/// conservative default threshold [`Compute::compute_backend`] uses. Tunable as real kernels land.
pub const GPU_ELEMENT_THRESHOLD: usize = 1 << 16; // 65 536

/// The backend a compute op should run on — chosen per call by [`Compute::compute_backend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComputeBackend {
    /// The dense, LLVM-vectorized CPU reduction (the [`Aggregate`](crate::io::memory::Aggregate) path).
    Cpu,
    /// The device kernel (uploaded + run on the GPU) — the accelerated path.
    Gpu,
}

impl ComputeBackend {
    /// Whether this is the GPU (device-kernel) backend.
    pub fn is_gpu(&self) -> bool {
        matches!(self, ComputeBackend::Gpu)
    }

    /// The short lowercase token (`"gpu"` / `"cpu"`) — the stable name the bindings surface, matching
    /// [`GpuBackend::as_str`](super::GpuBackend::as_str).
    pub fn as_str(&self) -> &'static str {
        match self {
            ComputeBackend::Cpu => "cpu",
            ComputeBackend::Gpu => "gpu",
        }
    }
}

/// **GPU-vs-CPU dispatch + device-aware transfer** for device memory. Every [`GpuMemory`] buffer
/// gets it (a blanket impl); the aggregations come from
/// [`Aggregate`](crate::io::memory::Aggregate) (shared with every source).
pub trait Compute: GpuMemory {
    /// The backend an op over `elements` values would run on: **GPU** when this buffer is on a real
    /// device *and* `elements >= `[`GPU_ELEMENT_THRESHOLD`], else **CPU**. The dispatch a
    /// device-backed source consults before choosing a kernel; exposed so a caller can see it.
    fn compute_backend(&self, elements: usize) -> ComputeBackend {
        if self.device().backend() != GpuBackend::Cpu && elements >= GPU_ELEMENT_THRESHOLD {
            ComputeBackend::Gpu
        } else {
            ComputeBackend::Cpu
        }
    }

    /// **Device-aware copy** — copies this buffer's whole content into `dst`, auto-selecting the
    /// path: a same-device GPU→GPU copy would run as a device-to-device DMA (the marked seam),
    /// otherwise the zero-copy host copy ([`copy_from`](crate::io::memory::IOBase::copy_from)).
    /// Returns the byte count.
    fn compute_copy_into<D: GpuMemory>(&self, dst: &mut D) -> Result<u64, IoError> {
        let same_gpu = self.device().backend() != GpuBackend::Cpu && self.device() == dst.device();
        if same_gpu {
            // GPU seam: a device-to-device DMA runs here once the hardware queue is wired; until
            // then the host copy below is used (correct on every platform).
        }
        dst.copy_from(self)
    }
}

impl<T: GpuMemory> Compute for T {}
