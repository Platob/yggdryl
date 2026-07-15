//! Time **and** memory for the scaled-decimal family: the self-describing value type's checked
//! arithmetic (`d128`/`d256`), value identity, and the columnar `DecimalSerie` build / element
//! read / round-trip. Focus: that value arithmetic is stack-only (no per-op allocation) and that
//! a column element read decodes from borrowed bytes (zero-copy).
//!
//! Dependency-free (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench decimal`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::{D128Serie, D128, D256};
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
    println!("  {name:<40} {mops:8.2}      {allocs:6.2}      {bytes:8.1}");
}

fn main() {
    let iters = 20_000;
    let n = 1024usize;

    let values: Vec<D128> = (0..n)
        .map(|i| D128::new(i as i128 * 100 + 45, 2).unwrap())
        .collect();
    let options: Vec<Option<D128>> = values.iter().map(|&v| Some(v)).collect();
    let serie = D128Serie::from_options(30, 2, &options).unwrap();

    let a = D128::new(123_456_789, 4).unwrap();
    let b = D128::new(987_654, 2).unwrap();
    let wide_a = D256::new(123_456_789_012_345, 6).unwrap();
    let wide_b = D256::new(987_654_321, 3).unwrap();

    println!("io::fixed decimal — time & memory ({iters} iters, {n} elements)\n");
    println!(
        "  {:<40} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));

    // Value arithmetic (stack-only) — d128 and d256.
    row(
        "D128 checked_add (aligned scales)",
        measure(1, iters, || {
            let _ = a.checked_add(&b).unwrap();
        }),
    );
    row(
        "D128 checked_mul",
        measure(1, iters, || {
            let _ = a.checked_mul(&b).unwrap();
        }),
    );
    row(
        "D256 checked_add",
        measure(1, iters, || {
            let _ = wide_a.checked_add(&wide_b).unwrap();
        }),
    );

    // Value identity (normalize on the stack).
    let x = D128::new(25, 1).unwrap();
    let y = D128::new(250, 2).unwrap();
    row(
        "D128 cmp (cross-scale)",
        measure(1, iters, || {
            let _ = x.cmp(&y);
        }),
    );

    // Column build.
    row(
        "D128Serie::from_options (1024)",
        measure(1, iters, || {
            let _ = D128Serie::from_options(30, 2, &options).unwrap();
        }),
    );

    // Element read (zero-copy decode from borrowed bytes).
    row(
        "D128Serie::get (one element)",
        measure(1, iters, || {
            let _ = serie.get(512);
        }),
    );

    // Column round-trip through a byte sink.
    row(
        "D128Serie write+read round-trip (1024)",
        measure(1, iters, || {
            let mut sink = Bytes::new();
            serie.write_to(&mut sink).unwrap();
            sink.rewind();
            let _ = D128Serie::read_from(&mut sink).unwrap();
        }),
    );

    // Zero-copy Arrow export (feature `arrow`).
    #[cfg(feature = "arrow")]
    row(
        "D128Serie::to_arrow_array (1024)",
        measure(1, iters, || {
            let _ = serie.to_arrow_array();
        }),
    );
}
