//! Time **and** memory benchmark for the `io::amd` device-memory family (feature `amd`) — the
//! [`Aggregate`](yggdryl_core::io::memory::Aggregate) reductions / filter and the host↔device
//! transfer, over an [`AmdHeap`](yggdryl_core::io::amd::AmdHeap). The reductions stream the typed
//! data through a fixed stack chunk, so the **allocs/op** column proves they run with zero heap
//! allocation in the hot loop.
//!
//! Dependency-free (`harness = false`, a plain `main`) with the same counting allocator as the
//! other benches. Run with `cargo bench -p yggdryl-core --features amd --bench io_amd`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Relaxed);
            BYTES.fetch_add(layout.size(), Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn measure(items: usize, iters: u32, mut op: impl FnMut()) -> (f64, f64, f64) {
    op();
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    let (a1, b1) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let total = items as f64 * f64::from(iters);
    (
        total / secs / 1_000_000.0,
        (a1 - a0) as f64 / total,
        (b1 - b0) as f64 / total,
    )
}

fn row(name: &str, (mops, allocs, bytes): (f64, f64, f64)) {
    println!("  {name:<36} {mops:8.2}      {allocs:6.2}      {bytes:7.1}");
}

fn main() {
    use yggdryl_core::io::amd::{AmdHeap, AmdMemory};
    use yggdryl_core::io::memory::{Aggregate, IOBase};

    let iters = 2_000;
    let n = 1 << 16; // 65 536 elements — crosses the GPU dispatch threshold + many stack chunks

    // An AMD device heap holding N i32s and N f64s.
    let ints: Vec<i32> = (0..n as i32).collect();
    let floats: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let mut ibuf = AmdHeap::with_capacity(n * 4);
    ibuf.pwrite_i32_array(0, &ints).unwrap();
    let mut fbuf = AmdHeap::with_capacity(n * 8);
    fbuf.pwrite_f64_array(0, &floats).unwrap();

    println!("amd device compute — time & memory ({iters} iters over {n} elements)\n");
    println!(
        "  {:<36} {:>8}   {:>10}   {:>9}",
        "op", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(74));

    row(
        "sum_i32 (reduce)",
        measure(n, iters, || {
            black_box(ibuf.sum_i32(0, n).unwrap());
        }),
    );
    row(
        "min_i32 (reduce)",
        measure(n, iters, || {
            black_box(ibuf.min_i32(0, n).unwrap());
        }),
    );
    row(
        "max_i32 (reduce)",
        measure(n, iters, || {
            black_box(ibuf.max_i32(0, n).unwrap());
        }),
    );
    row(
        "count_ge_i32 (filter)",
        measure(n, iters, || {
            black_box(ibuf.count_ge_i32(0, n, black_box(n as i32 / 2)).unwrap());
        }),
    );
    row(
        "sum_f64 (reduce)",
        measure(n, iters, || {
            black_box(fbuf.sum_f64(0, n).unwrap());
        }),
    );
    row(
        "mean_f64 (reduce)",
        measure(n, iters, || {
            black_box(fbuf.mean_f64(0, n).unwrap());
        }),
    );

    // Host <-> device transfer: upload replaces the content, download_vec owns one Vec.
    let payload = vec![0xABu8; n];
    let mut xfer = AmdHeap::with_capacity(n);
    row(
        "upload (host -> device)",
        measure(n, iters, || {
            xfer.upload(black_box(&payload)).unwrap();
        }),
    );
    row(
        "download_vec (device -> host)",
        measure(n, iters, || {
            black_box(xfer.download_vec());
        }),
    );
}
