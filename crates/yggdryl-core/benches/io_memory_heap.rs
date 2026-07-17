//! Time **and** memory benchmark for the in-heap [`Heap`](yggdryl_core::io::memory::Heap) source and
//! the byte I/O trait surface: the byte-array primitives, the typed `byte` / `bit` / `i32` /
//! `i64` accessors, the allocation-reusing `pread_into` transfer versus the owning `pread_vec`,
//! cursor streaming, and slicing.
//!
//! Dependency-free (`harness = false`, a plain `main`). A counting global allocator makes every
//! measurement report **allocations/op** and **bytes/op** next to throughput — allocation counts
//! are build-independent and deterministic, so they validate the zero-alloc-accessor and
//! buffer-reuse rules directly. Runs in well under a second.
//!
//! Run with `cargo bench -p yggdryl-core --bench heap` (build release for real throughput
//! numbers; the allocation numbers are the same in debug and release).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::memory::{Heap, IOBase};

// -------------------------------------------------------------------------------------
// Counting allocator — every alloc (a `Vec` growth realloc counts as one) is tallied.
// -------------------------------------------------------------------------------------

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

/// Runs `op` once to warm up, then `iters` times over `items` inputs each, returning
/// `(millions of ops/second, allocations per op, bytes allocated per op)`.
fn measure(items: usize, iters: u32, mut op: impl FnMut()) -> (f64, f64, f64) {
    op(); // warm up any one-time initialization out of the measured window
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
    println!("  {name:<34} {mops:8.2}      {allocs:6.2}      {bytes:7.1}");
}

fn main() {
    let iters = 20_000;
    // A 4 KiB page of data — representative of a block read from a source.
    let page: Vec<u8> = (0..4096u32).map(|i| i as u8).collect();
    let src = Heap::from_slice(&page);

    println!("Heap — time & memory ({iters} iters)\n");
    println!(
        "  {:<34} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(70));

    // Typed positioned reads (256 elements per call = 256 ops): stack buffers, zero allocation.
    row(
        "pread_byte",
        measure(256, iters, || {
            for i in 0..256u64 {
                let _ = src.pread_byte(i).unwrap();
            }
        }),
    );
    row(
        "pread_i32",
        measure(1, iters, || {
            let _ = src.pread_i32(0).unwrap();
        }),
    );
    row(
        "pread_i64",
        measure(1, iters, || {
            let _ = src.pread_i64(0).unwrap();
        }),
    );
    row(
        "pread_bit",
        measure(1, iters, || {
            let _ = src.pread_bit(17).unwrap();
        }),
    );

    // Transfer a 4 KiB page: pread_into reuses one warm buffer, pread_vec allocates each call.
    let mut scratch = Vec::with_capacity(page.len());
    row(
        "pread_into (4 KiB, reused buf)",
        measure(1, iters, || {
            let _ = src.pread_into(0, page.len(), &mut scratch);
        }),
    );
    row(
        "pread_vec (4 KiB, fresh Vec)",
        measure(1, iters, || {
            let _ = src.pread_vec(0, page.len());
        }),
    );

    // Cursor streaming write of a mixed record into a sized, reused buffer.
    let mut sink = Heap::from_slice(&[0u8; 13]);
    row(
        "cursor write byte+i32+i64",
        measure(1, iters, || {
            sink.rewind();
            sink.write_byte(1).unwrap();
            sink.write_i32(-1).unwrap();
            sink.write_i64(1).unwrap();
        }),
    );

    // Slicing owns a copy of the window: one allocation.
    row(
        "slice (1 KiB window)",
        measure(1, iters, || {
            let _ = src.slice(1024, 1024).unwrap();
        }),
    );

    // from_slice ingests external bytes: one owned copy.
    row(
        "from_slice (4 KiB ingest)",
        measure(1, iters, || {
            let _ = Heap::from_slice(&page);
        }),
    );

    // Bulk typed arrays: stack-staged, vectorized dense conversion — zero heap allocation.
    let bulk_values = vec![7i32; 1024];
    let mut bulk_back = vec![0i32; 1024];
    let mut bulk_sink = Heap::with_capacity(4096);
    bulk_sink.pwrite_i32_array(0, &bulk_values).unwrap();
    row(
        "pwrite_i32_array (1024 elems)",
        measure(1024, iters, || {
            bulk_sink.pwrite_i32_array(0, &bulk_values).unwrap();
        }),
    );
    row(
        "pread_i32_array (1024 elems)",
        measure(1024, iters, || {
            bulk_sink.pread_i32_array(0, &mut bulk_back).unwrap();
        }),
    );

    // Repeated-value fill: never materializes the array (vs building a Vec then writing it).
    let mut fill_sink = Heap::with_capacity(4096);
    fill_sink.pwrite_i32_repeat(0, -1, 1024).unwrap();
    row(
        "pwrite_i32_repeat (1024 elems)",
        measure(1024, iters, || {
            fill_sink.pwrite_i32_repeat(0, -1, 1024).unwrap();
        }),
    );
    row(
        "repeat via full Vec (compare)",
        measure(1024, iters, || {
            let all = vec![-1i32; 1024];
            fill_sink.pwrite_i32_array(0, &all).unwrap();
        }),
    );

    // UTF-8 text over the byte layer: the read owns exactly its String.
    let mut text = Heap::with_capacity(64);
    text.pwrite_utf8(0, "héllo wörld — text!");
    let text_len = text.byte_size() as usize;
    row(
        "pread_utf8 (short text)",
        measure(1, iters, || {
            let _ = text.pread_utf8(0, text_len).unwrap();
        }),
    );
}
