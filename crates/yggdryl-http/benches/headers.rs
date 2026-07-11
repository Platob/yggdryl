//! Throughput benchmark for [`Headers`](yggdryl_http::Headers): serialize / deserialize
//! (MB/s over the payload) and the get / set / zero-copy-mutate hot paths (Mops/s).
//!
//! Dependency-free (`harness = false`, a plain `main`). Run with
//! `cargo bench -p yggdryl-http --bench headers`.

use std::hint::black_box;
use std::time::Instant;

use yggdryl_http::Headers;

/// Runs `op` `iters` times, returning MB/s over `bytes` processed per iteration.
fn throughput_mb_s(bytes: usize, iters: u32, mut op: impl FnMut()) -> f64 {
    op(); // warm up
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    (bytes as f64 * iters as f64) / secs / (1024.0 * 1024.0)
}

/// Runs `op` `iters` times, returning millions of operations per second.
fn mops_s(iters: u32, mut op: impl FnMut()) -> f64 {
    op();
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    iters as f64 / secs / 1_000_000.0
}

fn main() {
    // A realistic header block: 16 keys of ~16-byte keys / ~32-byte values.
    const ENTRIES: usize = 16;
    let headers = Headers::from_pairs((0..ENTRIES).map(|i| {
        (
            format!("x-header-name-{i:02}").into_bytes(),
            format!("value-{i}-{}", "payload".repeat(3)).into_bytes(),
        )
    }));
    let bytes = headers.serialize_bytes();
    let size = bytes.len();
    let iters = 200_000;

    println!("Headers ({ENTRIES} entries, {size} bytes/block, {iters} iters):");

    let ser = throughput_mb_s(size, iters, || {
        black_box(black_box(&headers).serialize_bytes());
    });
    let de = throughput_mb_s(size, iters, || {
        black_box(Headers::deserialize_bytes(black_box(&bytes)).unwrap());
    });
    println!("  serialize {ser:9.1} MB/s   deserialize {de:9.1} MB/s");

    let key = b"x-header-name-07";
    let get = mops_s(iters * 10, || {
        black_box(black_box(&headers).get(black_box(key)));
    });

    // set (add/update) into a scratch map, and zero-copy in-place value extend.
    let mut scratch = headers.clone();
    let set = mops_s(iters, || {
        black_box(scratch.insert(b"scratch".to_vec(), b"v".to_vec()));
    });
    scratch.set_content_type("text/plain");
    let mutate = mops_s(iters, || {
        black_box(scratch.get_mut(Headers::CONTENT_TYPE).unwrap()).push(b'x');
    });
    println!(
        "  get {get:9.2} Mops/s   set {set:9.2} Mops/s   mutate(in-place) {mutate:9.2} Mops/s"
    );
}
