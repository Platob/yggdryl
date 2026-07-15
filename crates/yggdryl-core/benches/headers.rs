//! Time **and** memory benchmark for [`Headers`](yggdryl_core::io::Headers) over a realistic
//! ~16-entry header set: case-insensitive lookup (hit + miss), the HTTP text render/parse,
//! and the binary round-trip through a byte sink. Allocations/op sit next to throughput so
//! the zero-allocation lookup shows.
//!
//! Dependency-free (`harness = false`), counting global allocator. Run with
//! `cargo bench -p yggdryl-core --bench headers`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::{Bytes, Headers, IOCursor};

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

/// A representative request header set (~16 entries).
const HEADERS: &[(&str, &str)] = &[
    ("Host", "api.example.com"),
    ("User-Agent", "yggdryl/0.1"),
    ("Accept", "application/json"),
    ("Accept-Encoding", "gzip, deflate, br"),
    ("Accept-Language", "en-US,en;q=0.9"),
    ("Authorization", "Bearer abcdef0123456789"),
    ("Cache-Control", "no-cache"),
    ("Connection", "keep-alive"),
    ("Content-Type", "application/json; charset=utf-8"),
    ("Content-Length", "1024"),
    ("Cookie", "session=xyz; theme=dark"),
    ("Origin", "https://example.com"),
    ("Referer", "https://example.com/page"),
    ("X-Request-Id", "8f3a2b1c-0000-4a5b-9c8d-1e2f3a4b5c6d"),
    ("X-Forwarded-For", "203.0.113.7"),
    ("If-None-Match", "\"abc123\""),
];

fn build() -> Headers {
    let mut headers = Headers::with_capacity(HEADERS.len());
    for &(name, value) in HEADERS {
        headers.append(name, value);
    }
    headers
}

fn main() {
    let iters = 50_000;
    let headers = build();
    let http = headers.to_http_bytes();

    println!(
        "Headers — time & memory ({iters} iters, {} entries)\n",
        HEADERS.len()
    );
    println!(
        "  {:<34} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(70));

    // Case-insensitive lookup of a present header near the end — the linear scan, zero heap.
    row(
        "get (hit, case-insensitive)",
        measure(1, iters, || {
            let _ = headers.get("if-none-match");
        }),
    );

    // Lookup of an absent header scans the whole set — still zero heap.
    row(
        "get (miss)",
        measure(1, iters, || {
            let _ = headers.get("x-does-not-exist");
        }),
    );

    // Typed helper: parse Content-Length.
    row(
        "content_length (parse)",
        measure(1, iters, || {
            let _ = headers.content_length();
        }),
    );

    // Build the whole set from pairs.
    row(
        "build (16 appends)",
        measure(1, iters, || {
            let _ = build();
        }),
    );

    // Render to the HTTP wire form (one pre-sized allocation).
    row(
        "to_http_bytes (render)",
        measure(1, iters, || {
            let _ = headers.to_http_bytes();
        }),
    );

    // Parse the HTTP wire form.
    row(
        "parse_http",
        measure(1, iters, || {
            let _ = Headers::parse_http(&http);
        }),
    );

    // Binary write + read round-trip through a byte sink.
    row(
        "binary write+read round-trip",
        measure(1, iters, || {
            let mut sink = Bytes::new();
            headers.write_to(&mut sink).unwrap();
            sink.rewind();
            let _ = Headers::read_from(&mut sink).unwrap();
        }),
    );
}
