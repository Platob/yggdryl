//! Time **and** memory for the broadened numeric surface: a wide `[u8; 32]` newtype (`i256`,
//! not Arrow-native) and the runtime-`N` fixed-size byte family (`FixedBinary`). Focus: that
//! the wide newtype's `Serie` build/read is on par with the primitives (its `as_slice` is a
//! total, panic-free reinterpret — align 1), and that fixed-size `get_bytes` is zero-copy.
//!
//! Dependency-free (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench numerics`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::{Buffer, FixedBinarySerie, I256Scalar, I256Serie, I256};
use yggdryl_core::io::{Bytes, IOCursor};

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
    println!("  {name:<38} {mops:8.2}      {allocs:6.2}      {bytes:8.1}");
}

fn main() {
    let iters = 20_000;
    let n = 1024usize;
    let wide: Vec<I256> = (0..n)
        .map(|i| I256::from_le_bytes([(i % 251) as u8; 32]))
        .collect();
    let options: Vec<Option<I256>> = wide.iter().map(|&v| Some(v)).collect();
    let serie = I256Serie::from_options(&options);
    let buffer = Buffer::<I256>::from_slice(&wide);

    // Fixed-size binary column of 16-byte values.
    let blobs: Vec<[u8; 16]> = (0..n).map(|i| [(i % 251) as u8; 16]).collect();

    println!("io::fixed numerics — time & memory ({iters} iters, {n} elements)\n");
    println!(
        "  {:<38} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(74));

    // Wide newtype (i256, 32 bytes) — column build.
    row(
        "I256Serie::from_options (1024)",
        measure(1, iters, || {
            let _ = I256Serie::from_options(&options);
        }),
    );

    // Zero-copy typed view of the align-1 newtype (as_slice is total — never panics).
    row(
        "Buffer::<I256>::as_slice scan (1024)",
        measure(n, iters, || {
            let _ = buffer.as_slice().len();
        }),
    );

    // Element read.
    row(
        "I256Serie::get (one element)",
        measure(1, iters, || {
            let _ = serie.get(512);
        }),
    );

    // Column round-trip through a byte sink.
    row(
        "I256Serie write+read round-trip (1024)",
        measure(1, iters, || {
            let mut sink = Bytes::new();
            serie.write_to(&mut sink).unwrap();
            sink.rewind();
            let _ = I256Serie::read_from(&mut sink).unwrap();
        }),
    );

    // Scalar round-trip.
    let scalar = I256Scalar::of(wide[512]);
    row(
        "I256Scalar write+read round-trip",
        measure(1, iters, || {
            let mut sink = Bytes::new();
            scalar.write_to(&mut sink).unwrap();
            sink.rewind();
            let _ = I256Scalar::read_from(&mut sink).unwrap();
        }),
    );

    // Fixed-size binary (runtime N=16) — column build.
    row(
        "FixedBinarySerie::push (1024, N=16)",
        measure(n, iters, || {
            let mut col = FixedBinarySerie::new(16);
            for blob in &blobs {
                col.push(Some(blob)).unwrap();
            }
        }),
    );

    // Fixed-size zero-copy element read.
    let fixed = {
        let mut col = FixedBinarySerie::new(16);
        for blob in &blobs {
            col.push(Some(blob)).unwrap();
        }
        col
    };
    row(
        "FixedBinarySerie::get_bytes (one)",
        measure(1, iters, || {
            let _ = fixed.get_bytes(512);
        }),
    );
}
