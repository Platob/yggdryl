//! [`ComputeBackend`] — the **GPU-vs-CPU dispatch decision** for AMD device memory.
//!
//! The statistical aggregations themselves (`sum` / `min` / `max` / `mean` / `std` / `first` /
//! `last` / `count_ge`, every numeric width) live on [`Aggregate`](crate::io::memory::Aggregate) —
//! a blanket trait over **every** [`IOBase`](crate::io::memory::IOBase), so an [`AmdHeap`](super::AmdHeap)
//! runs them exactly like a `Heap`. What is AMD-specific is *where* they should run:
//! [`compute_backend`](super::AmdMemory::compute_backend) picks the **GPU** when the buffer is on a
//! real Radeon adapter *and* the workload is big enough to amortize the host↔device transfer, else
//! the **CPU**. It is the optimization seam the device kernels drop into.

/// Elements at or above this count make a GPU run worth the host↔device transfer — the
/// conservative default threshold [`compute_backend`](super::AmdMemory::compute_backend) uses.
/// Tunable as real kernels land.
pub const GPU_ELEMENT_THRESHOLD: usize = 1 << 16; // 65 536

/// The backend a compute op should run on — chosen per call by
/// [`compute_backend`](super::AmdMemory::compute_backend).
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

    /// The short lowercase token (`"gpu"` / `"cpu"`) — the stable name the bindings surface.
    pub fn as_str(&self) -> &'static str {
        match self {
            ComputeBackend::Cpu => "cpu",
            ComputeBackend::Gpu => "gpu",
        }
    }
}
