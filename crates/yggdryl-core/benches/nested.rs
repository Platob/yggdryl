//! Time **and** memory benchmark for the `io::nested` struct layer: erasing a typed column into a
//! [`Column`], assembling a [`StructSerie`], reading rows, and the byte-codec round-trip. Nested
//! columns copy their leaf bytes into the erased carrier (the erased column owns its buffers), so
//! the allocations/op column is the story — navigation is free, construction is a bounded copy.
//!
//! Dependency-free harness (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench nested`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::boxed;
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::nested::StructSerie;
use yggdryl_core::io::var::Utf8Serie;

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

fn build_table(n: usize) -> StructSerie {
    let ids = boxed(Serie::from_values(&(0..n as i64).collect::<Vec<_>>()));
    let names = boxed(Utf8Serie::from_strs(
        &(0..n).map(|_| Some("value")).collect::<Vec<_>>(),
    ));
    StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap()
}

fn main() {
    let iters = 20_000;
    let n = 1024usize;
    let ids: Vec<i64> = (0..n as i64).collect();
    let names: Vec<Option<&str>> = (0..n).map(|_| Some("value")).collect();
    let table = build_table(n);
    let frame = table.serialize_bytes();

    println!("Nested struct — time & memory ({iters} iters, {n} rows × 2 cols)\n");
    println!(
        "  {:<40} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(76));

    row(
        "boxed(Serie<i64>) (erase)",
        measure(1, iters, || {
            let _ = boxed(Serie::from_values(&ids));
        }),
    );
    row(
        "boxed(Utf8Serie) (erase)",
        measure(1, iters, || {
            let _ = boxed(Utf8Serie::from_strs(&names));
        }),
    );
    row(
        "StructSerie::from_named (2 cols)",
        measure(1, iters, || {
            let _ = build_table(n);
        }),
    );
    row(
        "StructSerie::column + type_id (navigate)",
        measure(2, iters, || {
            let _ = table.column(0).map(|c| c.type_id());
            let _ = table.column_named("name").map(|c| c.type_id());
        }),
    );
    row(
        "StructSerie::row",
        measure(1, iters, || {
            let _ = table.row(n / 2);
        }),
    );
    row(
        "StructSerie::serialize_bytes",
        measure(1, iters, || {
            let _ = table.serialize_bytes();
        }),
    );
    row(
        "StructSerie::deserialize_bytes",
        measure(1, iters, || {
            let _ = StructSerie::deserialize_bytes(&frame).unwrap();
        }),
    );
}
