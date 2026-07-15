//! Time **and** memory for the column **access** surface: element/scalar `get`, single `set`, and
//! the bulk set (`set_range` from another column). Covers the fixed numeric `Serie`, the
//! fixed-size byte `FixedBinarySerie`, the variable-length `Utf8Serie` (whose `set` rewrites
//! offsets — deliberately expensive), and the `DecimalSerie`. Focus: that `get`/`get_scalar` are
//! zero-copy, single `set` is O(1), and bulk `set_range` materializes the values in **one** COW
//! rather than re-sealing the buffer per element.
//!
//! Dependency-free (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench access`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::{D128Serie, FixedBinarySerie, Serie, D128};
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
    println!("  {name:<44} {mops:8.2}      {allocs:7.3}      {bytes:8.1}");
}

fn main() {
    let iters = 20_000;
    let n = 256usize;

    println!("io column access — time & memory ({iters} iters, {n} elements)\n");
    println!(
        "  {:<44} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(82));

    // ---- fixed numeric Serie<i32> --------------------------------------------------------
    let base: Vec<i32> = (0..n as i32).collect();
    let patch = Serie::from_values(&base);
    let mut col = Serie::from_values(&base);

    row(
        "Serie::<i32> get_scalar (one)",
        measure(1, iters, || {
            let _ = col.get_scalar(128);
        }),
    );
    row(
        "Serie::<i32> set (one)",
        measure(1, iters, || {
            col.set(128, Some(7)).unwrap();
        }),
    );
    row(
        "Serie::<i32> set_range (256, from Serie)",
        measure(1, iters, || {
            col.set_range(0, &patch).unwrap();
        }),
    );
    row(
        "Serie::<i32> set_values (256, native)",
        measure(1, iters, || {
            col.set_values(0, &base).unwrap();
        }),
    );

    // ---- fixed-size binary (N=16) --------------------------------------------------------
    let blob = [7u8; 16];
    let mut fixed = FixedBinarySerie::new(16);
    for _ in 0..n {
        fixed.push(Some(&blob)).unwrap();
    }
    row(
        "FixedBinarySerie set (one, N=16)",
        measure(1, iters, || {
            fixed.set(128, Some(&blob)).unwrap();
        }),
    );

    // ---- variable-length Utf8Serie (offset rewrite) --------------------------------------
    let strings: Vec<Option<&str>> = (0..n).map(|_| Some("abcd")).collect();
    let mut text = Utf8Serie::from_strs(&strings);
    row(
        "Utf8Serie set_str same-length (one)",
        measure(1, iters, || {
            text.set_str(128, Some("wxyz")).unwrap();
        }),
    );
    row(
        "Utf8Serie set_str grow (one, offset rewrite)",
        measure(1, iters, || {
            // Alternate lengths so every set triggers an offset shift.
            text.set_str(128, Some("longer-value")).unwrap();
            text.set_str(128, Some("ab")).unwrap();
        }),
    );

    // ---- decimal Serie -------------------------------------------------------------------
    let decimals: Vec<D128> = (0..n)
        .map(|i| D128::new(i as i128 * 100 + 45, 2).unwrap())
        .collect();
    let dpatch = D128Serie::from_values(20, 2, &decimals).unwrap();
    let mut dcol = D128Serie::from_values(20, 2, &decimals).unwrap();
    row(
        "D128Serie set (one)",
        measure(1, iters, || {
            dcol.set(128, Some(D128::new(999, 2).unwrap())).unwrap();
        }),
    );
    row(
        "D128Serie set_range (256, from Serie)",
        measure(1, iters, || {
            dcol.set_range(0, &dpatch).unwrap();
        }),
    );
}
