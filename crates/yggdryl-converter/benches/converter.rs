//! Throughput benchmark for the `codec::converter` family — the dtype-keyed numeric
//! cast and the flexible string parse / render.
//!
//! Dependency-free (`harness = false`, a plain `main`). Run with
//! `cargo bench -p yggdryl-core --bench converter`.

use std::time::Instant;

use yggdryl_converter::PrimitiveType;

/// Runs `op` `iters` times, returning MB/s over `bytes` processed per iteration.
fn throughput_mb_s(bytes: usize, iters: u32, mut op: impl FnMut()) -> f64 {
    op(); // warm up
    let start = Instant::now();
    for _ in 0..iters {
        op();
    }
    let secs = start.elapsed().as_secs_f64();
    (bytes as f64 * f64::from(iters)) / secs / (1024.0 * 1024.0)
}

fn main() {
    cast_bench();
    parse_bench();
    format_bench();
}

/// Bulk numeric cast `i32 -> i64` over a 1 MiB payload (the source byte size is the
/// throughput denominator).
fn cast_bench() {
    let size = 1 << 20; // 1 MiB of i32
    let iters = 200;
    let count = size / 4;
    let bytes: Vec<u8> = (0..count as i32).flat_map(i32::to_le_bytes).collect();

    let cast = throughput_mb_s(size, iters, || {
        let _ = PrimitiveType::I32
            .cast_bytes(PrimitiveType::I64, &bytes)
            .unwrap();
    });
    println!("converter over 1 MiB / 100k values:");
    println!("  cast i32->i64          {cast:9.1} MB/s");
}

/// Flexible parse of decimal strings into `i32` (throughput over the input text bytes).
fn parse_bench() {
    let n = 100_000_i32;
    let iters = 50;
    let strings: Vec<String> = (0..n).map(|i| i.to_string()).collect();
    let total: usize = strings.iter().map(String::len).sum();

    let parse = throughput_mb_s(total, iters, || {
        for s in &strings {
            let _ = PrimitiveType::I32.parse_bytes(s).unwrap();
        }
    });
    println!("  parse string->i32      {parse:9.1} MB/s");
}

/// Render `i32` values back to strings (throughput over the output text bytes).
fn format_bench() {
    let n = 100_000_i32;
    let iters = 50;
    let le: Vec<[u8; 4]> = (0..n).map(i32::to_le_bytes).collect();
    let total: usize = (0..n).map(|i| i.to_string().len()).sum();

    let format = throughput_mb_s(total, iters, || {
        for bytes in &le {
            let _ = PrimitiveType::I32.format_bytes(bytes).unwrap();
        }
    });
    println!("  format i32->string     {format:9.1} MB/s");
}
