//! Time **and** memory benchmark for the `io::fixed` typed layer over `i32`:
//! `Buffer<i32>` construction / element access / append, `Serie<i32>` build + null handling,
//! and the `Scalar` / `Serie` serialization round-trip through a byte sink. Allocations/op
//! and bytes/op sit next to throughput so the zero-copy-read and payload-reuse claims show.
//!
//! Dependency-free (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench fixed`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::fixed::{Buffer, I32Scalar, I32Serie};
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
    println!("  {name:<34} {mops:8.2}      {allocs:6.2}      {bytes:8.1}");
}

fn main() {
    let iters = 20_000;
    let n = 1024usize;
    let values: Vec<i32> = (0..n as i32).collect();
    let buffer = Buffer::<i32>::from_vec(values.clone());

    println!("io::fixed (i32) — time & memory ({iters} iters, {n} elements)\n");
    println!(
        "  {:<34} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(70));

    // Construction from a Vec<T> moves the payload — bytes/op is the small Arc box, not 4 KiB.
    row(
        "Buffer::from_vec (1024 i32)",
        measure(1, iters, || {
            let _ = Buffer::<i32>::from_vec(values.clone());
        }),
    );

    // Element read — decode from borrowed bytes, zero heap.
    row(
        "Buffer::get (one element)",
        measure(1, iters, || {
            let _ = buffer.get(512);
        }),
    );

    // Sum over the zero-copy typed view.
    row(
        "Buffer::as_slice sum (1024)",
        measure(n, iters, || {
            let _: i64 = buffer.as_slice().iter().map(|&v| v as i64).sum();
        }),
    );

    // Append growth from empty (with capacity) — the write path.
    row(
        "Buffer::push (1024, prealloc)",
        measure(n, iters, || {
            let mut b = Buffer::<i32>::with_capacity(n);
            for &v in &values {
                b.push(v);
            }
        }),
    );

    // Column build, all valid.
    row(
        "Serie::from_values (1024)",
        measure(1, iters, || {
            let _ = I32Serie::from_values(&values);
        }),
    );

    // Column build with a validity mask (every 4th element null).
    let options: Vec<Option<i32>> = (0..n as i32).map(|v| (v % 4 != 0).then_some(v)).collect();
    row(
        "Serie::from_options (1/4 null)",
        measure(1, iters, || {
            let _ = I32Serie::from_options(&options);
        }),
    );

    // Scalar round-trip through a byte sink.
    let mut scalar_sink = Bytes::with_capacity(I32Scalar::serialized_width());
    row(
        "Scalar write+read round-trip",
        measure(1, iters, || {
            scalar_sink.rewind();
            I32Scalar::of(12345).write_to(&mut scalar_sink).unwrap();
            scalar_sink.rewind();
            let _ = I32Scalar::read_from(&mut scalar_sink).unwrap();
        }),
    );

    // Serie round-trip through a byte sink.
    let serie = I32Serie::from_values(&values);
    row(
        "Serie write+read round-trip (1024)",
        measure(1, iters, || {
            let mut sink = Bytes::new();
            serie.write_to(&mut sink).unwrap();
            sink.rewind();
            let _ = I32Serie::read_from(&mut sink).unwrap();
        }),
    );
}
