//! Time **and** memory benchmark for the `io::var` variable-length layer over `Utf8`:
//! `Utf8Serie` build (all-valid and with nulls), the **zero-copy** `get_str` access path, the
//! `push_str` append path, and the `Scalar` / `Serie` serialization round-trip through a byte
//! sink. Allocations/op and bytes/op sit next to throughput so the zero-copy-read claim shows
//! (`get_str` returns a borrowed `&str` — 0 allocs/op).
//!
//! Dependency-free (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench var`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::var::{Utf8Scalar, Utf8Serie};
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
    println!("  {name:<36} {mops:8.2}      {allocs:6.2}      {bytes:8.1}");
}

fn main() {
    let iters = 20_000;
    let n = 1024usize;
    let owned: Vec<String> = (0..n).map(|i| format!("value-{i}")).collect();
    let strs: Vec<Option<&str>> = owned.iter().map(|s| Some(s.as_str())).collect();
    let serie = Utf8Serie::from_strs(&strs);

    println!("io::var (utf8) — time & memory ({iters} iters, {n} elements)\n");
    println!(
        "  {:<36} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(72));

    // Column build, all valid — offsets + data grow once each (amortized).
    row(
        "Utf8Serie::from_strs (1024)",
        measure(1, iters, || {
            let _ = Utf8Serie::from_strs(&strs);
        }),
    );

    // Column build with a validity mask (every 4th element null).
    let with_nulls: Vec<Option<&str>> = strs
        .iter()
        .enumerate()
        .map(|(i, s)| if i % 4 == 0 { None } else { *s })
        .collect();
    row(
        "Utf8Serie::from_strs (1/4 null)",
        measure(1, iters, || {
            let _ = Utf8Serie::from_strs(&with_nulls);
        }),
    );

    // Zero-copy element read — a borrowed &str, no heap.
    row(
        "Utf8Serie::get_str (one element)",
        measure(1, iters, || {
            let _ = serie.get_str(512);
        }),
    );

    // Zero-copy scan over every element — sum of lengths, still 0 allocs/op.
    row(
        "Utf8Serie::get_str scan (1024)",
        measure(n, iters, || {
            let total: usize = (0..serie.len())
                .filter_map(|i| serie.get_str(i))
                .map(str::len)
                .sum();
            let _ = total;
        }),
    );

    // Append growth from empty (with capacity) — the write path.
    row(
        "Utf8Serie::push_str (1024, prealloc)",
        measure(n, iters, || {
            let mut col = Utf8Serie::with_capacity(n);
            for s in &strs {
                col.push_str(*s);
            }
        }),
    );

    // Scalar round-trip through a byte sink.
    let scalar = Utf8Scalar::of("value-512");
    row(
        "Scalar write+read round-trip",
        measure(1, iters, || {
            let mut sink = Bytes::new();
            scalar.write_to(&mut sink).unwrap();
            sink.rewind();
            let _ = Utf8Scalar::read_from(&mut sink).unwrap();
        }),
    );

    // Serie round-trip through a byte sink.
    row(
        "Serie write+read round-trip (1024)",
        measure(1, iters, || {
            let mut sink = Bytes::new();
            serie.write_to(&mut sink).unwrap();
            sink.rewind();
            let _ = Utf8Serie::read_from(&mut sink).unwrap();
        }),
    );
}
