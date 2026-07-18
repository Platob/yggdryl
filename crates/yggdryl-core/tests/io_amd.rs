//! Functional tests for the `io::amd` AMD Radeon device-memory family (feature `amd`) — the live
//! device probe ([`detect`] / [`AmdDevice`]), the [`AmdHeap`] over the full [`IOBase`] byte + SIMD
//! surface, its [`AmdMemory`] host↔device transfer, the [`ComputeBackend`] dispatch, the shared
//! [`AmdCursor`] / [`AmdSlice`], and the [`MemoryInfo`] snapshot. Compiles only under `--features amd`.
#![cfg(feature = "amd")]

use yggdryl_core::io::amd::{
    detect, AmdCursor, AmdHeap, AmdMemory, AmdSlice, ComputeBackend, GPU_ELEMENT_THRESHOLD,
};
use yggdryl_core::io::memory::{Aggregate, IOBase};
use yggdryl_core::io::MemoryInfo;

#[test]
fn probe_adapts_to_the_hardware_and_the_heap_always_works() {
    // `detect()` is `Some` only on a machine with a real AMD Radeon adapter; either way an
    // `AmdHeap` is usable, reporting a device whose `is_present` matches whether one was found.
    let adapter = detect();
    let dev = AmdHeap::new();
    assert_eq!(dev.device().is_present(), adapter.is_some());
    // The device name is stable and non-empty; the fallback names itself explicitly.
    assert!(!dev.device().name().is_empty());
    assert_eq!(
        dev.device().is_present(),
        dev.device().name() != "no AMD device (host memory)"
    );
}

#[test]
fn amd_heap_is_amd_memory_upload_download() {
    let mut dev = AmdHeap::new();
    dev.upload(b"device payload").unwrap();
    assert_eq!(dev.byte_size(), 14);
    assert_eq!(dev.download_vec(), b"device payload");

    // Oversized download returns a short count (the whole content).
    let mut out = [0u8; 32];
    assert_eq!(dev.download(&mut out), 14);
    // Download from an empty buffer copies nothing.
    assert_eq!(AmdHeap::new().download(&mut out), 0);

    // Re-upload replaces the content and syncs size.
    dev.upload(b"tiny").unwrap();
    assert_eq!(dev.as_slice(), b"tiny");
}

#[test]
fn device_memory_runs_the_full_iobase_simd_surface() {
    let mut dev = AmdHeap::with_capacity(256);
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
fn amd_cursor_and_slice_share_the_zero_copy_fast_path() {
    // The AMD family reuses the crate's one cursor/slice, instantiated over `AmdHeap`.
    let mut cur = AmdCursor::new(AmdHeap::from_host(b"radeon payload"));
    let mut head = [0u8; 6];
    assert_eq!(cur.read(&mut head), 6);
    assert_eq!(&head, b"radeon");
    cur.seek(yggdryl_core::io::Whence::Start, 7).unwrap();
    let mut tail = [0u8; 7];
    assert_eq!(cur.read(&mut tail), 7);
    assert_eq!(&tail, b"payload");

    let win = AmdSlice::new(AmdHeap::from_host(b"radeon payload"), 7, 7).unwrap();
    assert_eq!(win.byte_size(), 7);
    assert_eq!(win.pread_vec(0, 7), b"payload");
}

#[test]
fn device_memory_info_is_a_capacity_snapshot() {
    let info = AmdHeap::new().memory_info();
    assert!(info.total() >= info.available());
    assert!(info.usage_ratio() >= 0.0 && info.usage_ratio() <= 1.0);
    // The unknown sentinel is distinguishable.
    assert!(MemoryInfo::unknown().is_unknown());
    assert!(!MemoryInfo::system().is_unknown() || cfg!(not(any(windows, target_os = "linux"))));
}

// -------------------------------------------------------------------------------------
// Aggregations (shared with every source) + the AMD compute dispatch
// -------------------------------------------------------------------------------------

#[test]
fn aggregations_and_filter_on_device_memory() {
    let mut buf = AmdHeap::new();
    buf.pwrite_i32_array(0, &[4, 8, 15, 16, 23, 42]).unwrap();
    assert_eq!(buf.sum_i32(0, 6).unwrap(), 108);
    assert_eq!(buf.min_i32(0, 6).unwrap(), Some(4));
    assert_eq!(buf.max_i32(0, 6).unwrap(), Some(42));
    assert_eq!(buf.mean_i32(0, 6).unwrap(), Some(18.0));
    assert_eq!(buf.count_ge_i32(0, 6, 16).unwrap(), 3); // filter: 16, 23, 42
                                                        // Empty range: min/max/mean are None, sum is 0.
    assert_eq!(buf.min_i32(0, 0).unwrap(), None);
    assert_eq!(buf.sum_i32(0, 0).unwrap(), 0);

    // Floats, across the chunk boundary (> AGG_CHUNK = 1024 elements) to exercise streaming.
    let data: Vec<f64> = (0..5000).map(|i| i as f64).collect();
    let mut fbuf = AmdHeap::new();
    fbuf.pwrite_f64_array(0, &data).unwrap();
    assert_eq!(
        fbuf.sum_f64(0, 5000).unwrap(),
        (0..5000).sum::<i64>() as f64
    );
    assert_eq!(fbuf.max_f64(0, 5000).unwrap(), Some(4999.0));
    assert_eq!(fbuf.count_ge_f64(0, 5000, 2500.0).unwrap(), 2500);
}

#[test]
fn compute_backend_dispatch_matches_the_detected_device() {
    let buf = AmdHeap::new();
    // A small workload always stays on the CPU (a transfer would not amortize).
    assert_eq!(buf.compute_backend(8), ComputeBackend::Cpu);
    // A large workload dispatches to the GPU only when a real adapter backs the device.
    let big = buf.compute_backend(GPU_ELEMENT_THRESHOLD * 4);
    if buf.device().is_present() {
        assert_eq!(big, ComputeBackend::Gpu);
    } else {
        assert_eq!(big, ComputeBackend::Cpu);
    }
    assert!(!ComputeBackend::Cpu.is_gpu());
    assert_eq!(ComputeBackend::Gpu.as_str(), "gpu");
}

#[test]
fn compute_copy_into_transfers_between_device_buffers() {
    let src = AmdHeap::from_host(b"compute copy");
    let mut dst = AmdHeap::new();
    assert_eq!(src.compute_copy_into(&mut dst).unwrap(), 12);
    assert_eq!(dst.as_slice(), b"compute copy");
}

#[test]
fn compute_min_max_ignore_nan_regardless_of_order() {
    // A NaN must never poison min/max, whether it leads or trails (order-independent).
    let mut lead = AmdHeap::new();
    lead.pwrite_f64_array(0, &[f64::NAN, 1.0, 2.0]).unwrap();
    assert_eq!(lead.min_f64(0, 3).unwrap(), Some(1.0));
    assert_eq!(lead.max_f64(0, 3).unwrap(), Some(2.0));
    let mut trail = AmdHeap::new();
    trail.pwrite_f64_array(0, &[1.0, 2.0, f64::NAN]).unwrap();
    assert_eq!(trail.min_f64(0, 3).unwrap(), Some(1.0));
    assert_eq!(trail.max_f64(0, 3).unwrap(), Some(2.0));
}
