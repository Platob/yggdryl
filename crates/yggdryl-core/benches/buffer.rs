//! Throughput benchmark for the typed buffers: `I32Buffer` construction, byte
//! round-trips, and — under `arrow` — zero-copy `from_arrow` versus a copy.
//!
//! Dependency-free (`harness = false`, a plain `main`). Run with
//! `cargo bench -p yggdryl-core --bench buffer`.

use std::time::Instant;

use yggdryl_core::I32Buffer;

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
    let count = (1 << 20) / 4; // 256 Ki i32 == 1 MiB
    let size = count * 4;
    let iters = 200;
    let values: Vec<i32> = (0..count as i32).collect();

    println!(
        "I32Buffer over {} KiB ({count} i32), {iters} iters:",
        size / 1024
    );

    let construct = throughput_mb_s(size, iters, || {
        let _ = I32Buffer::from_slice(&values);
    });

    let buffer = I32Buffer::from_slice(&values);
    let serialize = throughput_mb_s(size, iters, || {
        let _ = buffer.serialize_bytes();
    });

    let bytes = buffer.serialize_bytes();
    let deserialize = throughput_mb_s(size, iters, || {
        let _ = I32Buffer::deserialize_bytes(&bytes).unwrap();
    });

    println!(
        "  from_slice {construct:9.1} MB/s   serialize {serialize:9.1} MB/s   \
         deserialize {deserialize:9.1} MB/s"
    );

    arrow_bench(&values, size, iters);
}

/// Zero-copy Arrow `ScalarBuffer` wrap vs a copying construction.
fn arrow_bench(values: &[i32], size: usize, iters: u32) {
    use yggdryl_core::arrow_buffer::ScalarBuffer;

    let scalar = ScalarBuffer::<i32>::from(values.to_vec());
    let wrap = throughput_mb_s(size, iters, || {
        let _ = I32Buffer::from_arrow(scalar.clone());
    });
    let copy = throughput_mb_s(size, iters, || {
        let _ = I32Buffer::from_slice(values);
    });
    println!("  arrow from_arrow (zero-copy) {wrap:9.1} MB/s   from_slice (copy) {copy:9.1} MB/s");
}
