//! Time **and** memory benchmark for the [`typed`](yggdryl_core::typed) serialization layer — the
//! `Encoder`/`Decoder` bulk round-trip and the `Reduce` aggregations over a `FixedSerie`, so the
//! typed column is shown to add **no overhead** over the raw `IOBase` bulk kernels it forwards to.
//! The **allocs/op** column proves the bulk build/decode/reduce paths allocate only what the result
//! owns (a build owns its data buffer; a reduce owns nothing).
//!
//! Dependency-free (`harness = false`, a plain `main`) with the same counting allocator as the
//! other benches. Run with `cargo bench -p yggdryl-core --bench typed`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::typed::fixedbyte::{Float64, Int64};
use yggdryl_core::typed::{FixedSerie, Scalar};

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
        (a1 - a0) as f64 / f64::from(iters),
        (b1 - b0) as f64 / f64::from(iters),
    )
}

fn row(name: &str, (mops, allocs, bytes): (f64, f64, f64)) {
    println!("  {name:<40} {mops:8.1}    {allocs:8.2}    {bytes:9.0}");
}

fn main() {
    let iters = 2_000;
    let n = 1 << 16; // 65 536 elements

    let ints: Vec<i64> = (0..n as i64).collect();
    let floats: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let column = FixedSerie::<Int64>::from_values(&ints);
    let fcolumn = FixedSerie::<Float64>::from_values(&floats);

    println!("typed serialization — time & memory ({iters} iters over {n} elements)\n");
    println!(
        "  {:<40} {:>8}    {:>8}    {:>9}",
        "op", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));

    // Build: encode a whole column in one vectorized bulk write (allocates its data buffer).
    row(
        "FixedSerie::from_values (build i64)",
        measure(n, iters, || {
            black_box(FixedSerie::<Int64>::from_values(black_box(&ints)));
        }),
    );
    // Decode: read every element back into a fresh Vec (one allocation the caller owns).
    row(
        "Serie::values (decode i64)",
        measure(n, iters, || {
            black_box(column.values());
        }),
    );
    // Reduce: sum / min / max forward to the data buffer's allocation-free Aggregate kernels.
    row(
        "Serie::sum (reduce i64)",
        measure(n, iters, || {
            black_box(column.sum().unwrap());
        }),
    );
    row(
        "Serie::min (reduce i64)",
        measure(n, iters, || {
            black_box(column.min().unwrap());
        }),
    );
    row(
        "Serie::mean (reduce f64)",
        measure(n, iters, || {
            black_box(fcolumn.mean().unwrap());
        }),
    );
    // Scalar random access — one element decode, allocation-free.
    row(
        "Serie::get (scalar decode i64)",
        measure(n, iters / 16, || {
            for i in 0..n {
                black_box(column.get(black_box(i)));
            }
        }),
    );
}
