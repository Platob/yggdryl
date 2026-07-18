//! Functional tests for the `io::gpu` device-memory layer (feature `gpu`) — the by-architecture
//! device probe, the CPU backend unified with `Heap` ([`CpuHeap`]), the [`MemoryInfo`] snapshot,
//! and (under `gpu-amd`) the AMD [`AmdBuffer`]. Compiles only under `--features gpu`.
#![cfg(feature = "gpu")]

use yggdryl_core::io::gpu::{available_devices, default_device, CpuHeap, GpuBackend, GpuMemory};
use yggdryl_core::io::memory::{Heap, IOBase};
use yggdryl_core::io::MemoryInfo;

#[test]
fn probe_always_offers_the_cpu_device() {
    let devices = available_devices();
    assert!(
        !devices.is_empty(),
        "the probe is never empty (cpu is always present)"
    );
    assert!(devices.iter().any(|d| d.backend() == GpuBackend::Cpu));
    let dev = default_device();
    assert!(dev.total_memory() > 0 || dev.is_cpu());
    assert_eq!(GpuBackend::Cpu.as_str(), "cpu");
}

#[test]
fn cpu_heap_is_our_heap_and_a_gpu_memory() {
    // CpuHeap is a type alias for Heap — the unification.
    let mut dev: CpuHeap = Heap::new();
    assert!(dev.device().is_cpu());
    dev.upload(b"device payload").unwrap();
    assert_eq!(dev.byte_size(), 14);
    assert_eq!(dev.download_vec(), b"device payload");

    // Oversized download returns a short count (the whole content).
    let mut out = [0u8; 32];
    assert_eq!(dev.download(&mut out), 14);
    // Download from an empty buffer copies nothing.
    assert_eq!(Heap::new().download(&mut out), 0);

    // Re-upload replaces the content and syncs size.
    dev.upload(b"tiny").unwrap();
    assert_eq!(dev.as_slice(), b"tiny");
}

#[test]
fn device_memory_runs_the_full_iobase_simd_surface() {
    let mut dev = CpuHeap::with_capacity(256);
    dev.pwrite_i32_array(0, &[1, -2, 3, -4]).unwrap();
    dev.pwrite_f64_array(16, &[1.5, 2.5]).unwrap();
    dev.pwrite_u128(32, u128::MAX).unwrap();
    let mut ints = [0i32; 4];
    let mut floats = [0f64; 2];
    dev.pread_i32_array(0, &mut ints).unwrap();
    dev.pread_f64_array(16, &mut floats).unwrap();
    assert_eq!(ints, [1, -2, 3, -4]);
    assert_eq!(floats, [1.5, 2.5]);
    assert_eq!(dev.pread_u128(32).unwrap(), u128::MAX);
}

#[test]
fn device_memory_info_is_a_capacity_snapshot() {
    let info = default_device().memory_info();
    assert!(info.total() >= info.available());
    assert!(info.usage_ratio() >= 0.0 && info.usage_ratio() <= 1.0);
    // The unknown sentinel is distinguishable.
    assert!(MemoryInfo::unknown().is_unknown());
    assert!(!MemoryInfo::system().is_unknown() || cfg!(not(any(windows, target_os = "linux"))));
}

#[cfg(feature = "gpu-amd")]
#[test]
fn amd_buffer_is_a_gpu_memory_over_the_detected_device() {
    use yggdryl_core::io::gpu::AmdBuffer;

    let mut buf = AmdBuffer::from_host(&[0xAB; 16]);
    assert_eq!(buf.byte_size(), 16);
    // The device backend is either the detected AMD device or the CPU fallback.
    let backend = buf.device().backend();
    assert!(backend == GpuBackend::Amd || backend == GpuBackend::Cpu);

    // Full IOBase + bulk surface on the AMD buffer.
    buf.pwrite_i64_array(16, &[10, -20, 30]).unwrap();
    let mut back = [0i64; 3];
    buf.pread_i64_array(16, &mut back).unwrap();
    assert_eq!(back, [10, -20, 30]);
    assert_eq!(&buf.download_vec()[..16], &[0xAB; 16]);
}

// -------------------------------------------------------------------------------------
// Compute — auto-dispatched aggregations / filters / copy
// -------------------------------------------------------------------------------------

#[test]
fn compute_aggregations_and_filter_on_device_memory() {
    use yggdryl_core::io::gpu::{Compute, ComputeBackend, GPU_ELEMENT_THRESHOLD};

    let mut buf = CpuHeap::new();
    buf.pwrite_i32_array(0, &[4, 8, 15, 16, 23, 42]).unwrap();
    assert_eq!(buf.sum_i32(0, 6).unwrap(), 108);
    assert_eq!(buf.min_i32(0, 6).unwrap(), Some(4));
    assert_eq!(buf.max_i32(0, 6).unwrap(), Some(42));
    assert_eq!(buf.mean_i32(0, 6).unwrap(), Some(18.0));
    assert_eq!(buf.count_ge_i32(0, 6, 16).unwrap(), 3); // filter: 16, 23, 42
                                                        // Empty range: min/max/mean are None, sum is 0.
    assert_eq!(buf.min_i32(0, 0).unwrap(), None);
    assert_eq!(buf.sum_i32(0, 0).unwrap(), 0);

    // Floats, across the chunk boundary (> COMPUTE_CHUNK = 1024 elements) to exercise streaming.
    let data: Vec<f64> = (0..5000).map(|i| i as f64).collect();
    let mut fbuf = CpuHeap::new();
    fbuf.pwrite_f64_array(0, &data).unwrap();
    assert_eq!(
        fbuf.sum_f64(0, 5000).unwrap(),
        (0..5000).sum::<i64>() as f64
    );
    assert_eq!(fbuf.max_f64(0, 5000).unwrap(), Some(4999.0));
    assert_eq!(fbuf.count_ge_f64(0, 5000, 2500.0).unwrap(), 2500);

    // Dispatch policy: the CPU device always resolves to the CPU backend (no real GPU here for a
    // CpuHeap), regardless of size; the threshold constant is exposed and sane.
    assert_eq!(buf.compute_backend(6), ComputeBackend::Cpu);
    assert_eq!(
        buf.compute_backend(GPU_ELEMENT_THRESHOLD * 2),
        ComputeBackend::Cpu
    );
    assert!(!ComputeBackend::Cpu.is_gpu());
}

#[test]
fn compute_copy_into_transfers_between_device_buffers() {
    use yggdryl_core::io::gpu::Compute;

    let src = CpuHeap::from_slice(b"compute copy");
    let mut dst = CpuHeap::new();
    assert_eq!(src.compute_copy_into(&mut dst).unwrap(), 12);
    assert_eq!(dst.as_slice(), b"compute copy");
}

#[cfg(feature = "gpu-amd")]
#[test]
fn compute_backend_selects_gpu_on_a_large_device_workload() {
    use yggdryl_core::io::gpu::{AmdBuffer, Compute, ComputeBackend, GPU_ELEMENT_THRESHOLD};

    let buf = AmdBuffer::new();
    // Only a real (non-cpu) detected device dispatches to GPU for a large workload; on a machine
    // whose AMD adapter is detected the large-workload backend is Gpu, else Cpu (fallback).
    let big = buf.compute_backend(GPU_ELEMENT_THRESHOLD * 4);
    if buf.device().backend().as_str() == "amd" {
        assert_eq!(big, ComputeBackend::Gpu);
    } else {
        assert_eq!(big, ComputeBackend::Cpu);
    }
    // A small workload always stays on the CPU (transfer would not amortize).
    assert_eq!(buf.compute_backend(8), ComputeBackend::Cpu);
}
