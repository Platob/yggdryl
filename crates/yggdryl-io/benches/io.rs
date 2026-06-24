//! Lightweight timing benchmarks for the hot byte-IO paths.
//!
//! Run with `cargo bench -p yggdryl-io`. Uses a plain `main` (the crate sets
//! `harness = false`) so there is no benchmark-framework dependency; it reports
//! nanoseconds per iteration and, for the transfer paths, MiB/s.

use std::hint::black_box;
use std::time::Instant;

use yggdryl_io::{copy, BytesIO, Codec, Frames, Io, ReadBytes, Whence};

/// Times `f` over `iters` iterations (after a short warm-up) and prints ns/iter.
fn bench(name: &str, iters: u64, mut f: impl FnMut()) {
    for _ in 0..iters / 10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let per = start.elapsed().as_nanos() as f64 / iters as f64;
    println!("{name:<38} {per:>9.1} ns/iter");
}

/// Times a transfer of `bytes` per iteration and reports throughput in MiB/s.
fn bench_throughput(name: &str, iters: u64, bytes: usize, mut f: impl FnMut()) {
    for _ in 0..iters / 10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let secs = start.elapsed().as_secs_f64();
    let mib = (bytes as f64 * iters as f64) / (1024.0 * 1024.0);
    println!("{name:<38} {:>9.1} MiB/s", mib / secs);
}

fn main() {
    println!("== cursor / random access ==");
    let data = vec![0u8; 64 * 1024];
    let mut io = BytesIO::from_bytes(data.clone());
    bench("BytesIO::seek", 5_000_000, || {
        black_box(io.seek(black_box(1024), Whence::Start).unwrap());
    });
    let mut buf = [0u8; 256];
    bench("Io::read_at (memory)", 5_000_000, || {
        black_box(io.read_at(black_box(4096), &mut buf).unwrap());
    });

    println!("\n== streamed read ==");
    bench("ReadBytes::read_bytes (4 KiB)", 2_000_000, || {
        io.seek(0, Whence::Start).unwrap();
        let mut chunk = [0u8; 4096];
        black_box(io.read_bytes(&mut chunk).unwrap());
    });

    // Reuse the source (rewind with seek) and the destination (clear keeps its
    // capacity) so the timed work is the transfer itself, not a 4 MiB clone.
    println!("\n== transfer (4 MiB, source reused) ==");
    let payload = vec![7u8; 4 * 1024 * 1024];
    let mut src = BytesIO::from_bytes(payload.clone());
    let mut dst: Vec<u8> = Vec::with_capacity(payload.len());
    bench_throughput(
        "copy: BytesIO -> Vec (zero-copy)",
        2000,
        payload.len(),
        || {
            src.seek(0, Whence::Start).unwrap();
            dst.clear();
            black_box(copy(&mut src, &mut dst).unwrap());
        },
    );
    bench_throughput(
        "read_to_end: BytesIO -> Vec (chunked)",
        2000,
        payload.len(),
        || {
            src.seek(0, Whence::Start).unwrap();
            dst.clear();
            black_box(src.read_to_end(&mut dst).unwrap());
        },
    );

    println!("\n== codec ==");
    let frame = vec![3u8; 256];
    bench("Frames::write (256 B)", 2_000_000, || {
        let mut sink: Vec<u8> = Vec::with_capacity(260);
        Frames.write(&mut sink, &frame).unwrap();
        black_box(&sink);
    });
    let mut encoded: Vec<u8> = Vec::new();
    for _ in 0..1024 {
        Frames.write(&mut encoded, &frame).unwrap();
    }
    bench("Frames::stream (1024 frames)", 20_000, || {
        let count = Frames.stream(&encoded[..]).filter(|r| r.is_ok()).count();
        black_box(count);
    });
}
