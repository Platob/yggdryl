//! The **NVIDIA CUDA** architecture (feature `gpu-cuda`) — *reserved*.
//!
//! CUDA comes **after** AMD in the roadmap. This module fixes the seam so the probe and feature
//! wiring are stable: [`detect`] enumerates CUDA devices (returning `None` until the CUDA runtime
//! route is linked), and a `CudaBuffer` implementing [`GpuMemory`](super::GpuMemory) over the NVML
//! / CUDA driver API drops in here — mirroring [`amd`](super::amd) — without touching the layer's
//! public shape.

use super::GpuDevice;

/// Probes for an NVIDIA **CUDA** device, returning its [`GpuDevice`] when found, else `None`.
/// Reserved: returns `None` until the CUDA driver route is wired (the caller falls back to the CPU
/// device meanwhile). Defensive — never panics.
pub fn detect() -> Option<GpuDevice> {
    // DESIGN: real CUDA enumeration (cuDeviceGetCount / cuDeviceTotalMem, or NVML) lands here
    // behind this feature; the CPU fallback covers callers until then.
    None
}
