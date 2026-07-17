//! Time **and** memory benchmark for the [`Headers`](yggdryl_core::headers::Headers) media-type
//! and mtime accessors — the centralized `Content-Type` / `Content-Encoding` reads/writes and
//! the epoch-microseconds `mtime` codec — plus the plain get/insert baseline.
//!
//! Dependency-free (`harness = false`, a plain `main`) with the same counting allocator as the
//! other benches, so every row reports **allocations/op** and **bytes/op** next to throughput.
//! The `mtime` rows show the allocation-free integer render (`set_mtime` writes the decimal
//! straight into a stack buffer; only the entry storage allocates).
//!
//! Run with `cargo bench -p yggdryl-core --bench headers`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::headers::Headers;

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
    println!("  {name:<34} {mops:9.2}     {allocs:6.2}     {bytes:7.1}");
}

fn main() {
    let iters = 100_000;

    println!("Headers media type + mtime — time & memory ({iters} iters)\n");
    println!(
        "  {:<34} {:>9}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(70));

    // A realistic small header set, with a declared media type + mtime.
    let mut declared = Headers::new();
    declared.set_content_type("application/x-tar");
    declared.set_content_encoding("gzip");
    declared.set_mtime(1_600_000_000_000_000);
    declared.insert("Host", "example.com");
    declared.insert(Headers::CONTENT_LENGTH, "4096");

    row(
        "content_type (get)",
        measure(1, iters, || {
            let _ = declared.content_type();
        }),
    );
    row(
        "mime_type (parse primary)",
        measure(1, iters, || {
            let _ = declared.mime_type().unwrap();
        }),
    );
    row(
        "media_type (type + encoding fold)",
        measure(1, iters, || {
            let _ = declared.media_type().unwrap();
        }),
    );

    let mut sink = declared.clone();
    row(
        "set_mime_type (replace)",
        measure(1, iters, || {
            sink.set_content_type("application/json");
        }),
    );

    // mtime: the epoch-microseconds codec. The setter renders the decimal into a stack buffer
    // (no format!/String), so only the entry storage allocates.
    row(
        "mtime (get, parse decimal)",
        measure(1, iters, || {
            let _ = declared.mtime().unwrap();
        }),
    );
    let mut mt = declared.clone();
    row(
        "set_mtime (render decimal)",
        measure(1, iters, || {
            mt.set_mtime(1_600_000_000_123_456);
        }),
    );

    // Baseline: a plain get + a plain insert over the same set.
    row(
        "get (plain, present)",
        measure(1, iters, || {
            let _ = declared.get("host");
        }),
    );
    let mut ins = declared.clone();
    row(
        "insert (replace, present)",
        measure(1, iters, || {
            ins.insert("Host", "example.org");
        }),
    );
}
