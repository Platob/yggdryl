//! Lightweight throughput benchmarks for the compression codecs.
//!
//! Run with `cargo bench -p yggdryl-compression --all-features` (a codec whose
//! feature is off is skipped). Uses a plain `main` (the crate sets
//! `harness = false`) so there is no benchmark-framework dependency; it reports
//! MiB/s over the *uncompressed* size for both one-shot and streamed paths.

use std::hint::black_box;
use std::time::Instant;

use yggdryl_compression::{CompressIo, Compression};
use yggdryl_io::BytesIO;

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

fn main() {
    // A semi-compressible 1 MiB payload: repetitive text with some variation, so
    // the ratios and speeds resemble real columnar data rather than random noise.
    const SIZE: usize = 1 << 20;
    let payload: Vec<u8> = (0..SIZE)
        .map(|i| b"yggdryl compresses bytes "[i % 25])
        .collect();

    println!("== compression ({} KiB payload) ==", SIZE / 1024);
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

        // Streamed compress / decompress over an `Io` handle (CompressIo).
        bench(&format!("{codec} compress (Io stream)"), 200, SIZE, || {
            let mut src = BytesIO::from_bytes(payload.clone());
            black_box(src.compress(codec).unwrap());
        });
        bench(
            &format!("{codec} decompress (Io stream)"),
            200,
            SIZE,
            || {
                let mut src = BytesIO::from_bytes(packed.clone());
                black_box(src.decompress(Some(codec)).unwrap());
            },
        );
    }
}
