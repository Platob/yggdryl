//! Throughput benchmark for the gzip codec.
//!
//! Dependency-free (`harness = false`, a plain `main`) so it adds no dev-deps: it
//! times [`Gzip`](yggdryl_core::Gzip) encode/decode over a fixed corpus and prints
//! MB/s, in the same shape as the Python and Node comparison scripts
//! (`bindings/*/bench*`), so the Rust core, `flate2`-in-Python, and `zlib`-in-Node
//! numbers line up side by side.
//!
//! Run with `cargo bench -p yggdryl-core`. The gzip-dependent body is compiled only
//! when the `gzip` feature is on (it is by default).

#[cfg(feature = "gzip")]
fn main() {
    use std::time::Instant;

    use yggdryl_core::{Decoder, Encoder, Gzip};

    /// A ~1 MiB, moderately compressible corpus (repeated English-like text).
    fn corpus() -> Vec<u8> {
        let unit = b"the quick brown fox jumps over the lazy dog. ";
        let mut data = Vec::with_capacity(1 << 20);
        while data.len() < (1 << 20) {
            data.extend_from_slice(unit);
        }
        data
    }

    /// Runs `op` `iters` times, returning MB/s over `bytes` processed per iteration.
    fn throughput_mb_s(bytes: usize, iters: u32, mut op: impl FnMut()) -> f64 {
        // Warm up once so the first-call allocation is not billed to the timed run.
        op();
        let start = Instant::now();
        for _ in 0..iters {
            op();
        }
        let secs = start.elapsed().as_secs_f64();
        (bytes as f64 * f64::from(iters)) / secs / (1024.0 * 1024.0)
    }

    let data = corpus();
    let iters = 200;

    println!(
        "gzip throughput over {} KiB, {iters} iters:",
        data.len() / 1024
    );
    for level in [1_u32, 6, 9] {
        let gzip = Gzip::new(level).unwrap();
        let compressed = gzip.encode_byte_array(&data).unwrap();
        let ratio = data.len() as f64 / compressed.len() as f64;

        let enc = throughput_mb_s(data.len(), iters, || {
            gzip.encode_byte_array(&data).unwrap();
        });
        let dec = throughput_mb_s(data.len(), iters, || {
            gzip.decode_byte_array(&compressed).unwrap();
        });

        println!(
            "  level {level}: encode {enc:7.1} MB/s  decode {dec:7.1} MB/s  ratio {ratio:.2}x"
        );
    }

    #[cfg(feature = "zstd")]
    {
        use yggdryl_core::Zstd;

        println!(
            "\nzstd throughput over {} KiB, {iters} iters:",
            data.len() / 1024
        );
        for level in [1_i32, 3, 19] {
            let zstd = Zstd::new(level).unwrap();
            let compressed = zstd.encode_byte_array(&data).unwrap();
            let ratio = data.len() as f64 / compressed.len() as f64;
            let enc = throughput_mb_s(data.len(), iters, || {
                zstd.encode_byte_array(&data).unwrap();
            });
            let dec = throughput_mb_s(data.len(), iters, || {
                zstd.decode_byte_array(&compressed).unwrap();
            });
            println!(
                "  level {level}: encode {enc:7.1} MB/s  decode {dec:7.1} MB/s  ratio {ratio:.2}x"
            );
        }
    }
}

#[cfg(not(feature = "gzip"))]
fn main() {
    eprintln!("the `gzip` feature is disabled; nothing to benchmark");
}
