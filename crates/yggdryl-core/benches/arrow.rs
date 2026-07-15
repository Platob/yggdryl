//! Time **and** memory benchmark for the zero-copy Arrow interop (feature `arrow`):
//! `Buffer` / `Serie` ↔ `arrow_array::PrimitiveArray`. The point of the interop is that the
//! value payload is **never copied** — it is an `Arc` bump — so the allocations/op column is
//! the story: it is `0.00` for the buffer/dense paths and tiny (the validity mask) for the
//! nullable path, regardless of the 4096-element payload.
//!
//! Dependency-free harness (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --features arrow --bench arrow`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::{Buffer, Serie};

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
    let iters = 50_000;
    let n = 4096usize;
    let values: Vec<i32> = (0..n as i32).collect();
    let buffer = Buffer::<i32>::from_vec(values.clone());
    let dense = Serie::from_values(&values);
    let nullable = Serie::from_options(
        &(0..n as i32)
            .map(|v| (v % 4 != 0).then_some(v))
            .collect::<Vec<_>>(),
    );
    let arrow_array = buffer.to_arrow_array();

    println!("Arrow interop — time & memory ({iters} iters, {n} × i32)\n");
    println!(
        "  {:<36} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(72));

    row(
        "Buffer::to_arrow_array (zero-copy)",
        measure(1, iters, || {
            let _ = buffer.to_arrow_array();
        }),
    );
    row(
        "Buffer::from_arrow_array (zero-copy)",
        measure(1, iters, || {
            let _ = Buffer::<i32>::from_arrow_array(&arrow_array);
        }),
    );
    row(
        "Serie::to_arrow_array (dense)",
        measure(1, iters, || {
            let _ = dense.to_arrow_array();
        }),
    );
    row(
        "Serie::to_arrow_array (nullable)",
        measure(1, iters, || {
            let _ = nullable.to_arrow_array();
        }),
    );
    let nullable_arrow = nullable.to_arrow_array();
    row(
        "Serie::from_arrow_array (nullable)",
        measure(1, iters, || {
            let _ = Serie::<i32>::from_arrow_array(&nullable_arrow);
        }),
    );
}
