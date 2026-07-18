//! [`GpuBackend`] / [`GpuDevice`] and the **by-architecture** device probe.
//!
//! The gpu layer is organized **by GPU architecture** — `cpu` (the portable [`Heap`] backend),
//! `amd` (AMD Radeon, feature `gpu-amd`), `cuda` (NVIDIA, feature `gpu-cuda`). The probe
//! **adapts to the hardware present**: [`available_devices`] enumerates whatever the enabled
//! architecture modules detect, always ending with the CPU device so a target is never missing.

use super::MemoryInfo;

/// The GPU **architecture** a device belongs to. `#[non_exhaustive]` — more architectures (Intel,
/// Apple, …) slot in without breaking a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GpuBackend {
    /// The portable CPU backend — device memory is host RAM, realized by
    /// [`CpuHeap`](super::CpuHeap) (our [`Heap`](crate::io::memory::Heap)). Always available.
    Cpu,
    /// An AMD Radeon device (module `amd`, feature `gpu-amd`).
    Amd,
    /// An NVIDIA CUDA device (module `cuda`, feature `gpu-cuda`).
    Cuda,
}

impl GpuBackend {
    /// The short lowercase architecture token (`"cpu"`, `"amd"`, `"cuda"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            GpuBackend::Cpu => "cpu",
            GpuBackend::Amd => "amd",
            GpuBackend::Cuda => "cuda",
        }
    }

    /// Whether this is the portable CPU backend.
    pub fn is_cpu(&self) -> bool {
        matches!(self, GpuBackend::Cpu)
    }
}

/// A **value description of one compute device** — its architecture, human name, and total memory
/// (VRAM for a GPU, host RAM for the CPU). A plain value (`Clone`/`Eq`/`Hash`) that keys a map,
/// sits in a set, and travels over a wire. Live free-memory is a fresh [`memory_info`](GpuDevice::memory_info)
/// query, not baked into the descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuDevice {
    backend: GpuBackend,
    name: String,
    total_memory: u64,
}

impl GpuDevice {
    /// A device description from its parts.
    pub fn new(backend: GpuBackend, name: impl Into<String>, total_memory: u64) -> GpuDevice {
        GpuDevice {
            backend,
            name: name.into(),
            total_memory,
        }
    }

    /// The always-available **CPU** device — memory is host RAM, sized from
    /// [`MemoryInfo::system`](crate::io::MemoryInfo::system).
    pub fn cpu() -> GpuDevice {
        GpuDevice::new(
            GpuBackend::Cpu,
            "cpu (host memory)",
            MemoryInfo::system().total(),
        )
    }

    /// The device architecture.
    pub fn backend(&self) -> GpuBackend {
        self.backend
    }

    /// The human-readable device name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The total device memory in bytes (VRAM, or host RAM for the CPU device).
    pub fn total_memory(&self) -> u64 {
        self.total_memory
    }

    /// Whether this is the CPU (host-memory) device.
    pub fn is_cpu(&self) -> bool {
        self.backend.is_cpu()
    }

    /// A **live capacity snapshot** for this device — the CPU device queries host RAM fresh
    /// ([`MemoryInfo::system`](crate::io::MemoryInfo::system)); a GPU device reports its total VRAM
    /// (live free-VRAM query lands with the hardware backend).
    pub fn memory_info(&self) -> MemoryInfo {
        if self.is_cpu() {
            MemoryInfo::system()
        } else {
            MemoryInfo::new(self.total_memory, self.total_memory)
        }
    }
}

/// Enumerates the compute devices this build can allocate on — **adapting to the hardware
/// present**. Each enabled architecture module contributes what it detects (AMD via `gpu-amd`,
/// NVIDIA via `gpu-cuda`), and the portable CPU device is always appended last, so the result is
/// never empty.
///
/// ```
/// use yggdryl_core::io::gpu::{available_devices, GpuBackend};
///
/// let devices = available_devices();
/// assert!(!devices.is_empty());
/// assert!(devices.iter().any(|d| d.backend() == GpuBackend::Cpu)); // CPU is always present
/// ```
pub fn available_devices() -> Vec<GpuDevice> {
    let mut devices = Vec::new();
    #[cfg(feature = "gpu-amd")]
    {
        devices.extend(super::amd::detect());
    }
    #[cfg(feature = "gpu-cuda")]
    {
        devices.extend(super::cuda::detect());
    }
    devices.push(GpuDevice::cpu()); // the CPU fallback is always the last resort
    devices
}

/// The **default** device — the first detected hardware GPU, else the CPU fallback.
pub fn default_device() -> GpuDevice {
    available_devices()
        .into_iter()
        .find(|d| !d.is_cpu())
        .unwrap_or_else(GpuDevice::cpu)
}
