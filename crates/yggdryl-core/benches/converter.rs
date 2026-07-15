//! Time **and** memory for the type converter: the numeric `cast` (scalar / serie), the
//! same-type no-copy fast path, and the UTF-8 / binary bridges. Focus: that a same-type serie cast
//! is allocation-free (it shares the buffer), a cross-width cast allocates once, and the string
//! bridge's cost is the (expected) formatting/parsing allocation.
//!
//! Dependency-free (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench converter`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::{Scalar, Serie};
use yggdryl_core::io::var::{BinaryScalar, Utf8Scalar};

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

fn measure(iters: u32, mut op: impl FnMut()) -> (f64, f64, f64) {
    op();
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    let (a1, b1) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let total = f64::from(iters);
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
    let iters = 100_000;
    println!("io::converter — time & memory ({iters} iters)\n");
    println!(
        "  {:<44} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(82));

    let scalar = Scalar::of(1234i32);
    row(
        "Scalar<i32>::cast::<i64> (value)",
        measure(iters, || {
            let _ = scalar.cast::<i64>().unwrap();
        }),
    );
    row(
        "Scalar<i32>::cast::<f64> (value)",
        measure(iters, || {
            let _ = scalar.cast::<f64>().unwrap();
        }),
    );

    let col: Serie<i32> = (0..1024).map(Some).collect();
    row(
        "Serie<i32>::cast::<i32> (same type, no copy)",
        measure(iters, || {
            let _ = col.cast::<i32>().unwrap();
        }),
    );
    row(
        "Serie<i32>::cast::<i64> (1024, cross-width)",
        measure(iters / 100, || {
            let _ = col.cast::<i64>().unwrap();
        }),
    );

    // UTF-8 bridge (formatting + parsing allocate, as expected).
    row(
        "Scalar<i32>::to_utf8 (format)",
        measure(iters, || {
            let _ = scalar.to_utf8();
        }),
    );
    let text = Utf8Scalar::of("1234");
    row(
        "Utf8Scalar::parse_to::<i32>",
        measure(iters, || {
            let _ = text.parse_to::<i32>().unwrap();
        }),
    );

    // Binary bridge (canonical LE bytes; one small allocation for the boxed slice).
    row(
        "Scalar<i32>::to_binary",
        measure(iters, || {
            let _ = scalar.to_binary();
        }),
    );
    let bin = BinaryScalar::of(&1234i32.to_le_bytes());
    row(
        "BinaryScalar::read_to::<i32>",
        measure(iters, || {
            let _ = bin.read_to::<i32>().unwrap();
        }),
    );
}
