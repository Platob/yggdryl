//! Time **and** memory benchmark for the Phase 8b vectorized arithmetic: the typed fast path
//! (`Serie::add_unchecked` / the scalar broadcast) against the erased, checking + casting base ops
//! (`dyn AnySerie::add`), and — the headline — the **cast cost** of a cross-type add (`i32 + i64`,
//! whose right operand is range-checked into the left's `i32`) over the same-type add.
//!
//! Dependency-free harness (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench ops`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::{Field, Serie};
use yggdryl_core::io::{boxed, AnyScalar, DataTypeId};

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
    println!("  {name:<44} {mops:8.1}      {allocs:6.2}      {bytes:9.1}");
}

fn header(title: &str) {
    println!("\n{title}\n");
    println!(
        "  {:<44} {:>8}   {:>10}   {:>9}",
        "op", "Melem/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(84));
}

fn main() {
    let iters = 20_000;
    let n = 4096usize;

    let a: Vec<i32> = (0..n as i32).collect();
    let b: Vec<i32> = (1..=n as i32).collect();
    let a32 = Serie::from_values(&a);
    let b32 = Serie::from_values(&b);
    let b64 = Serie::from_values(&(1..=n as i64).collect::<Vec<_>>());

    // Erased forms for the base-op rows.
    let ea = boxed(Serie::from_values(&a));
    let eb32 = boxed(Serie::from_values(&b));
    let eb64 = boxed(Serie::from_values(&(1..=n as i64).collect::<Vec<_>>()));
    let scalar = AnyScalar::leaf(
        Field::of("", DataTypeId::I32, 4, false),
        7i32.to_le_bytes().to_vec(),
    );

    header(&format!(
        "Vectorized arithmetic — time & memory ({iters} iters, {n} elements/op)"
    ));

    // The typed fast path — one tight pass, one result buffer.
    row(
        "Serie::add_unchecked (typed, same T)",
        measure(n, iters, || {
            let _ = a32.add_unchecked(&b32);
        }),
    );
    row(
        "Serie::div_unchecked (typed, zero-check)",
        measure(n, iters, || {
            let _ = a32.div_unchecked(&b32);
        }),
    );
    row(
        "Serie::add_scalar_unchecked (typed)",
        measure(n, iters, || {
            let _ = a32.add_scalar_unchecked(7);
        }),
    );

    // The erased base op — same type (no cast) vs cross-type (the range-checked cast into the left).
    row(
        "dyn AnySerie::add (erased, same T)",
        measure(n, iters, || {
            let _ = ea.add(eb32.as_ref()).unwrap();
        }),
    );
    row(
        "dyn AnySerie::add (erased, i32 + i64 — CAST)",
        measure(n, iters, || {
            let _ = ea.add(eb64.as_ref()).unwrap();
        }),
    );
    row(
        "dyn AnySerie::add_scalar (erased broadcast)",
        measure(n, iters, || {
            let _ = ea.add_scalar(&scalar).unwrap();
        }),
    );

    // The typed cast in isolation, to attribute the cross-type overhead above.
    row(
        "Serie::<i64>::cast::<i32> (the cast alone)",
        measure(n, iters, || {
            let _ = b64.cast::<i32>().unwrap();
        }),
    );

    println!(
        "\n  Cross-type add pays one range-checked cast of the right operand into the left's type;\n  \
         the same-type add and the typed `*_unchecked` skip it (a single fused pass)."
    );
}
