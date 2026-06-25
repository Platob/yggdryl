//! Lightweight throughput benchmarks for the compression codecs.
//!
//! Run with `cargo bench -p yggdryl-compression --all-features` (a codec whose
//! feature is off is skipped). Uses a plain `main` (the crate sets
//! `harness = false`) so there is no benchmark-framework dependency; it reports
//! MiB/s over the *uncompressed* size for both one-shot and streamed paths.
//!
//! The payload is a deterministic, **semi-compressible** CSV-like stream (a few
//! repeated tokens mixed with pseudo-random numbers), so the ratios and speeds
//! resemble real columnar/log data rather than a trivially compressible cycle.
//! The streamed (`Io`) benches reuse one handle and `seek(0)` between iterations
//! so the timing measures the codec, not a per-iteration buffer copy.

use std::hint::black_box;
use std::time::Instant;

use yggdryl_compression::{CompressIo, Compression};
use yggdryl_io::{BytesIO, Whence};

/// Times `f` over `iters` iterations (after a short warm-up) and reports
/// throughput in MiB/s relative to `bytes` processed per iteration.
fn bench(name: &str, iters: u64, bytes: usize, mut f: impl FnMut()) {
    for _ in 0..iters / 10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let secs = start.elapsed().as_secs_f64();
    let mib = (bytes as f64 * iters as f64) / (1024.0 * 1024.0);
    println!("{name:<40} {:>9.1} MiB/s", mib / secs);
}

/// A deterministic ~1 MiB CSV-like payload with moderate (~3-5x) compressibility.
fn payload() -> Vec<u8> {
    const SIZE: usize = 1 << 20;
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut out = Vec::with_capacity(SIZE + 64);
    let mut row = 0u64;
    while out.len() < SIZE {
        // A linear-congruential step keeps it deterministic without `rand`.
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let value = (state >> 40) % 100_000;
        out.extend_from_slice(format!("row,{row},region,eu-west,measure,{value}\n").as_bytes());
        row += 1;
    }
    out.truncate(SIZE);
    out
}

fn main() {
    const SIZE: usize = 1 << 20;
    let payload = payload();

    println!("== compression ({} KiB CSV-like payload) ==", SIZE / 1024);
    for codec in [Compression::Gzip, Compression::Zstd, Compression::Snappy] {
        if !codec.is_available() {
            println!("{:<40} (feature off)", codec.as_str());
            continue;
        }
        let packed = codec.compress(&payload).unwrap();
        let ratio = SIZE as f64 / packed.len() as f64;
        println!("{:<40} ratio {ratio:>6.2}x", format!("{codec} size"));

        // One-shot compress / decompress over an in-memory slice.
        bench(&format!("{codec} compress (one-shot)"), 200, SIZE, || {
            black_box(codec.compress(black_box(&payload)).unwrap());
        });
        bench(&format!("{codec} decompress (one-shot)"), 200, SIZE, || {
            black_box(codec.decompress(black_box(&packed)).unwrap());
        });

        // Streamed compress / decompress over a reused `Io` handle (CompressIo):
        // `seek(0)` resets the cursor each iteration without copying the buffer.
        let mut source = BytesIO::from_bytes(payload.clone());
        bench(&format!("{codec} compress (Io stream)"), 200, SIZE, || {
            source.seek(0, Whence::Start).unwrap();
            black_box(source.compress(codec).unwrap());
        });
        let mut compressed = BytesIO::from_bytes(packed.clone());
        bench(
            &format!("{codec} decompress (Io stream)"),
            200,
            SIZE,
            || {
                compressed.seek(0, Whence::Start).unwrap();
                black_box(compressed.decompress(black_box(Some(codec))).unwrap());
            },
        );
    }
}
