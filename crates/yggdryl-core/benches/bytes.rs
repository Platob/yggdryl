//! Time **and** memory benchmark for the byte-I/O type [`Bytes`](yggdryl_core::io::Bytes):
//! positioned (`pread`/`pwrite`) and cursor (`read`/`write`) access, zero-copy `slice`, and
//! the copy-on-write write path — each reported with allocations/op and bytes/op next to
//! throughput, so the zero-copy-read / zero-copy-slice / reuse-in-place claims are visible.
//!
//! Dependency-free (`harness = false`, a plain `main`), with a counting global allocator.
//! Run with `cargo bench -p yggdryl-core --bench bytes` (build release for real throughput;
//! the allocation numbers are the same in debug and release).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::{Bytes, IOBase, IOCursor, IOSlice, Whence};

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
    let iters = 50_000;
    let block = 4096usize; // a representative 4 KiB I/O block
    let payload = vec![0xabu8; block];
    let data = Bytes::from_vec(vec![0x5au8; block]);

    println!("Bytes — time & memory ({iters} iters, {block}-byte block)\n");
    println!(
        "  {:<34} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(70));

    // Positioned read into a caller buffer — zero-copy.
    let mut scratch = vec![0u8; block];
    row(
        "pread (positioned, into buf)",
        measure(1, iters, || {
            let _ = data.pread(0, &mut scratch);
        }),
    );

    // Cursor read — rewind then read the whole block.
    let mut reader = data.clone();
    row(
        "read (cursor, into buf)",
        measure(1, iters, || {
            reader.set_position(0);
            let _ = reader.read(&mut scratch);
        }),
    );

    // Zero-copy slice — an Arc bump, no heap.
    row(
        "slice (zero-copy window)",
        measure(1, iters, || {
            let _ = data.slice(0, block as u64).unwrap();
        }),
    );

    // In-place positioned write to a uniquely-owned buffer — reuses the payload allocation.
    let mut owned = Bytes::from_vec(vec![0u8; block]);
    row(
        "pwrite (in-place overwrite)",
        measure(1, iters, || {
            let _ = owned.pwrite(0, &payload);
        }),
    );

    // Cursor write that grows from empty — the append path.
    row(
        "write (grow from empty)",
        measure(1, iters, || {
            let mut buf = Bytes::with_capacity(block);
            let _ = buf.write(&payload);
        }),
    );

    // Copy-on-write: write to a slice still sharing the parent's allocation.
    let shared = Bytes::from_vec(vec![0u8; block]);
    row(
        "pwrite (copy-on-write, shared)",
        measure(1, iters, || {
            let mut window = shared.slice(0, block as u64).unwrap();
            window.pwrite(0, &payload);
        }),
    );

    // Owning read of the whole block to a fresh Vec.
    row(
        "read_to_end (owning)",
        measure(1, iters, || {
            reader.set_position(0);
            let _ = reader.read_to_end();
        }),
    );

    // A realistic mixed cursor session: seek around and read exact spans.
    let session = data.clone();
    let mut small = [0u8; 64];
    row(
        "seek + read_exact (session)",
        measure(1, iters, || {
            let mut cursor = session.clone();
            cursor.seek(Whence::End, -64).unwrap();
            let _ = cursor.read_exact(&mut small);
        }),
    );
}
