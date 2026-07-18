//! Time **and** memory benchmark for the [`Compression`](yggdryl_core::compression) codecs and
//! their zero-copy [`IOBase`] integration. It measures each native codec's compress /
//! decompress throughput and — the point of the yggdryl path — shows the **zero-copy read**
//! (`decompressed_with` handing the codec a source's borrowed bytes) beats a **naive** pipeline
//! that copies the source into a `Vec` first, by exactly that extra copy/allocation.
//!
//! Feature-gated: `cargo bench -p yggdryl-core --features compression --bench compression`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::compression::{Compression, Gzip, Lzma, Zlib, Zstd};
use yggdryl_core::io::memory::{Heap, IOBase};

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

/// `(MiB/s over `bytes`, allocs/op, bytes-alloc/op)`.
fn measure(bytes: usize, iters: u32, mut op: impl FnMut()) -> (f64, f64, f64) {
    op();
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    let (a1, b1) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let total = bytes as f64 * f64::from(iters);
    (
        total / secs / (1024.0 * 1024.0),
        (a1 - a0) as f64 / f64::from(iters),
        (b1 - b0) as f64 / f64::from(iters),
    )
}

fn row(name: &str, (mibs, allocs, bytes): (f64, f64, f64)) {
    println!("  {name:<36} {mibs:9.1}     {allocs:6.1}     {bytes:11.0}");
}

fn main() {
    // A ~1 MiB semi-repetitive corpus (compresses well, but not trivially).
    let mut data = Vec::new();
    for i in 0..24_000u32 {
        data.extend_from_slice(
            format!(
                "row {i:08} | the quick brown fox jumps over {} lazy dogs\n",
                i % 97
            )
            .as_bytes(),
        );
    }
    let iters = 40;

    println!(
        "Compression — time & memory ({} KiB corpus, {iters} iters)\n",
        data.len() / 1024
    );
    println!(
        "  {:<36} {:>9}   {:>10}   {:>12}",
        "op", "MiB/s", "allocs/op", "bytes-alloc/op"
    );
    println!("  {}", "-".repeat(78));

    let codecs: Vec<(&str, Box<dyn Compression>)> = vec![
        ("gzip", Box::new(Gzip::new())),
        ("zlib", Box::new(Zlib::new())),
        ("zstd", Box::new(Zstd::new())),
        ("xz", Box::new(Lzma::new())),
    ];

    for (name, codec) in &codecs {
        let packed = codec.compress(&data).unwrap();
        let ratio = data.len() as f64 / packed.len() as f64;
        println!("  -- {name} (ratio {ratio:.1}x) --");
        row(
            &format!("{name} compress"),
            measure(data.len(), iters, || {
                let _ = codec.compress(&data).unwrap();
            }),
        );
        row(
            &format!("{name} decompress"),
            measure(data.len(), iters, || {
                let _ = codec.decompress(&packed).unwrap();
            }),
        );
    }

    // --- The zero-copy IOBase path vs a naive copy-first pipeline. Gzip decompress: the
    // yggdryl path hands the codec the source's borrowed bytes; the naive path copies the
    // compressed source into a Vec first. The saved copy is exactly the input size. ---
    println!("\n  -- IOBase read path (gzip decompress a mapped/heap source) --");
    let codec = Gzip::new();
    let packed = codec.compress(&data).unwrap();
    let src = Heap::from_slice(&packed);
    assert!(src.as_bytes().is_some()); // contiguous -> zero-copy eligible

    row(
        "yggdryl decompressed_with (zero-copy)",
        measure(packed.len(), iters, || {
            let _ = src.decompressed_with(&codec).unwrap();
        }),
    );
    row(
        "naive copy-then-decompress",
        measure(packed.len(), iters, || {
            let buf = src.pread_vec(0, src.byte_size() as usize); // the extra copy
            let _ = codec.decompress(&buf).unwrap();
        }),
    );
}
