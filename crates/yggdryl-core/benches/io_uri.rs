//! Time **and** memory benchmark for the URI base types ([`Uri`](yggdryl_core::io::uri::Uri) /
//! [`Url`](yggdryl_core::io::uri::Url)): `Uri::parse` over a URL corpus, `Uri::from_path` over a
//! Windows-path corpus, the `serialize_bytes` / `deserialize_bytes` byte round-trip, the
//! zero-copy accessors, `Display`, and `Uri` as a `HashMap` key.
//!
//! Dependency-free (`harness = false`, a plain `main`). A counting global allocator makes
//! every measurement report **allocations/op** and **bytes/op** next to throughput —
//! allocation counts are build-independent and deterministic, so they validate the
//! at-most-one-copy / zero-copy-accessor rules directly. Runs in well under a second.
//!
//! Run with `cargo bench -p yggdryl-core --bench uri` (build release for real throughput
//! numbers; the allocation numbers are the same in debug and release).

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use yggdryl_core::io::uri::Uri;

// -------------------------------------------------------------------------------------
// Counting allocator — every alloc (a `String`/`Vec` growth realloc counts as one) is
// tallied, so a measurement can report how many allocations an operation makes.
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
    println!("  {name:<32} {mops:8.2}      {allocs:6.2}      {bytes:7.1}");
}

/// A representative spread of absolute URLs, plus one long URL to expose `String`-growth
/// reallocations in the byte codec.
const URLS: &[&str] = &[
    "https://user:pw@example.com:8080/a/b/c.txt?q=1&x=2#frag",
    "http://example.com/",
    "https://example.com/path/to/archive.tar.gz",
    "ftp://files.example.org:21/pub/readme",
    "http://[::1]:8080/v1/status",
    "postgres://svc:secret@db.internal:5432/app?sslmode=require",
    "s3://bucket-name/keys/2026/07/13/object.parquet",
    "mailto:person@example.com",
    "file:///etc/hosts",
    "wss://stream.example.com/socket?token=abcdef#live",
    "https://user:password@very.long.subdomain.example.com:8443\
     /deeply/nested/path/segment/tree/archive.backup.tar.gz?a=1&b=2&c=3&d=4#section-final",
];

/// A representative spread of filesystem paths: Windows drive, UNC, back-slashed
/// relative, POSIX absolute, and a multi-extension file.
const PATHS: &[&str] = &[
    r"C:\Users\alice\Documents\report.final.docx",
    r"D:\data\2026\input\records.tar.gz",
    r"\\server\share\team\notes.txt",
    r"src\bindings\python\lib.rs",
    "/usr/local/share/data/set.csv",
    "/var/log/app/service.log.1",
    r"E:\media\video\clip.mp4",
    "relative/dir/without/leading/slash",
];

fn main() {
    let iters = 20_000;

    println!("Uri — time & memory ({iters} iters over each corpus)\n");
    println!(
        "  {:<32} {:>8}   {:>10}   {:>9}",
        "op", "Mops/s", "allocs/op", "bytes/op"
    );
    println!("  {}", "-".repeat(66));

    row(
        "Uri::parse (URL corpus)",
        measure(URLS.len(), iters, || {
            for &s in URLS {
                let _ = Uri::parse_str(s).unwrap();
            }
        }),
    );

    row(
        "Uri::from_path (Windows corpus)",
        measure(PATHS.len(), iters, || {
            for &p in PATHS {
                let _ = Uri::from_path(p);
            }
        }),
    );

    let uris: Vec<Uri> = URLS.iter().map(|s| Uri::parse_str(s).unwrap()).collect();

    row(
        "serialize_bytes",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = uri.serialize_bytes();
            }
        }),
    );

    let encoded: Vec<Vec<u8>> = uris.iter().map(Uri::serialize_bytes).collect();
    row(
        "deserialize_bytes",
        measure(encoded.len(), iters, || {
            for bytes in &encoded {
                let _ = Uri::deserialize_bytes(bytes).unwrap();
            }
        }),
    );

    row(
        "serialize + deserialize",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap();
            }
        }),
    );

    // Zero-copy accessors: these borrow from the `Uri`, so they must allocate nothing.
    row(
        "accessors (scheme/host/path/name)",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = (
                    uri.scheme(),
                    uri.host(),
                    uri.path(),
                    uri.name(),
                    uri.extension(),
                );
            }
        }),
    );

    // Effective-endpoint accessors (default-port fallback + IPv6 host unbracketing): a table
    // scan and a couple of borrows, so — like the plain accessors — zero allocations.
    row(
        "endpoint (port_or_default/host)",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = (
                    uri.default_port(),
                    uri.port_or_default(),
                    uri.host_is_ipv6(),
                    uri.host_unbracketed(),
                );
            }
        }),
    );

    // Combinators. `copy` is a clone; `joinpath` adds one pre-sized path allocation over that
    // clone (no re-parse); `merge_with` overlays components, again without re-parsing.
    row(
        "copy (clone)",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = uri.copy();
            }
        }),
    );
    row(
        "joinpath (append segment)",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = uri.joinpath("segment");
            }
        }),
    );
    row(
        "merge_with (overlay)",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = uri.merge_with(uri);
            }
        }),
    );

    row(
        "to_string (Display)",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = uri.to_string();
            }
        }),
    );

    // `Uri` as a HashMap key exercises `Hash` (and `Eq` on collisions) in the hot path.
    let map: HashMap<Uri, usize> = uris.iter().cloned().zip(0..).collect();
    row(
        "HashMap lookup (Uri key)",
        measure(uris.len(), iters, || {
            for uri in &uris {
                let _ = map.get(uri);
            }
        }),
    );

    // Query-parameter map access + CRUD. Reads borrow (0 allocs); the map view and a write
    // each allocate once.
    let q = Uri::parse_str("http://h/p?a=1&b=2&c=3&d=4&a=9").unwrap();
    row(
        "query_param (read, first)",
        measure(1, iters, || {
            let _ = q.query_param("c");
        }),
    );
    row(
        "query_params (map view)",
        measure(1, iters, || {
            let _ = q.query_params();
        }),
    );
    row(
        "query_param_decoded (clean)",
        measure(1, iters, || {
            let _ = q.query_param_decoded("c");
        }),
    );
    let mut q_set = q.clone();
    row(
        "set_query_param (update)",
        measure(1, iters, || {
            q_set.set_query_param("b", "9");
        }),
    );
    let mut q_bulk = q.clone();
    row(
        "set_query_params (bulk x3)",
        measure(1, iters, || {
            q_bulk.set_query_params(&[("b", "9"), ("e", "5"), ("a", "0")]);
        }),
    );
    let mut q_norm = q.clone();
    row(
        "normalize_query (sort+clean)",
        measure(1, iters, || {
            q_norm.normalize_query();
        }),
    );
}
