//! Lightweight timing benchmarks for the hot byte-IO paths.
//!
//! Run with `cargo bench -p yggdryl-core --bench io`. Uses a plain `main` (the crate sets
//! `harness = false`) so there is no benchmark-framework dependency; it reports
//! nanoseconds per iteration and, for the transfer paths, MiB/s.

use std::hint::black_box;
use std::time::Instant;

use yggdryl_core::{copy, BytesIO, Codec, Frames, Io, IoError, IoStats, Url, Whence};

/// A streamed-only [`Io`] (no `as_slice`) wrapping a `BytesIO`, to force the
/// chunked-loop fallbacks in `copy_to` / `read_to_end` rather than the zero-copy
/// memory fast path. Forwards full reads so the transfer measures the loop.
#[derive(Debug)]
struct Streamed(BytesIO);

impl Io for Streamed {
    fn url(&self) -> Url {
        self.0.url()
    }
    fn stats(&self) -> Result<IoStats, IoError> {
        self.0.stats()
    }
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        Io::read(&mut self.0, buf)
    }
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        Io::seek(&mut self.0, offset, whence)
    }
    fn stream_position(&self) -> u64 {
        Io::stream_position(&self.0)
    }
    // No `as_slice`: forces the streamed copy_to / read_to_end paths.
}

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
    bench("Io::pread (memory, positional)", 5_000_000, || {
        black_box(io.pread(&mut buf, black_box(4096), Whence::Start).unwrap());
    });
    // Positional footer read: Whence::End leaves the cursor untouched (the path a
    // Parquet footer read takes). Distinct from the Whence::Start pread above.
    bench("Io::pread (memory, footer, End)", 5_000_000, || {
        black_box(io.pread(&mut buf, black_box(-256), Whence::End).unwrap());
    });

    println!("\n== streamed read ==");
    bench("Io::read (4 KiB)", 2_000_000, || {
        io.seek(0, Whence::Start).unwrap();
        let mut chunk = [0u8; 4096];
        black_box(Io::read(&mut io, &mut chunk).unwrap());
    });

    // Append-heavy writes: build a 1 MiB buffer from 4 KiB chunks, reusing the
    // sink's capacity each round (clear keeps it). This exercises the amortized
    // grow path — the appended region is written once, never zero-filled first.
    println!("\n== streamed write (append, 1 MiB in 4 KiB chunks) ==");
    let chunk = vec![9u8; 4096];
    let mut sink = BytesIO::with_capacity(1024 * 1024);
    bench_throughput("Io::write append loop", 4000, 1024 * 1024, || {
        sink.clear();
        for _ in 0..256 {
            black_box(Io::write(&mut sink, black_box(&chunk)).unwrap());
        }
    });

    // Reuse the source (rewind with seek) and the destination (clear keeps its
    // capacity) so the timed work is the transfer itself, not a 4 MiB clone.
    println!("\n== transfer (4 MiB, source reused) ==");
    let payload = vec![7u8; 4 * 1024 * 1024];
    let mut src = BytesIO::from_bytes(payload.clone());
    let mut dst = BytesIO::with_capacity(payload.len());
    bench_throughput(
        "copy: BytesIO -> BytesIO (zero-copy)",
        2000,
        payload.len(),
        || {
            src.seek(0, Whence::Start).unwrap();
            dst.clear();
            black_box(copy(&mut src, &mut dst).unwrap());
        },
    );
    let mut drained: Vec<u8> = Vec::with_capacity(payload.len());
    bench_throughput(
        "read_to_end: BytesIO -> Vec (chunked)",
        2000,
        payload.len(),
        || {
            src.seek(0, Whence::Start).unwrap();
            drained.clear();
            black_box(src.read_to_end(&mut drained).unwrap());
        },
    );

    println!("\n== transfer: streamed (no fast path, 4 MiB) ==");
    let mut streamed = Streamed(BytesIO::from_bytes(payload.clone()));
    bench_throughput(
        "copy_to: streamed chunked loop",
        2000,
        payload.len(),
        || {
            streamed.seek(0, Whence::Start).unwrap();
            dst.clear();
            black_box(streamed.copy_to(&mut dst).unwrap());
        },
    );
    bench_throughput("read_to_end: streamed -> Vec", 2000, payload.len(), || {
        streamed.seek(0, Whence::Start).unwrap();
        drained.clear();
        black_box(streamed.read_to_end(&mut drained).unwrap());
    });

    println!("\n== codec ==");
    let frame = vec![3u8; 256];
    bench("Frames::write (256 B)", 2_000_000, || {
        let mut sink = BytesIO::with_capacity(260);
        Frames.write(&mut sink, &frame).unwrap();
        black_box(sink.len());
    });
    let mut encoded = BytesIO::new();
    for _ in 0..1024 {
        Frames.write(&mut encoded, &frame).unwrap();
    }
    bench("Frames::stream (1024 frames)", 20_000, || {
        encoded.seek(0, Whence::Start).unwrap();
        let count = Frames.stream(&mut encoded).filter(|r| r.is_ok()).count();
        black_box(count);
    });
    // Write one frame then read it straight back — the full encode+decode the
    // write-only / stream-only cases above don't measure together.
    let mut roundtrip = BytesIO::with_capacity(260);
    bench("Frames::write + read (256 B)", 1_000_000, || {
        roundtrip.clear();
        Frames.write(&mut roundtrip, black_box(&frame)).unwrap();
        roundtrip.seek(0, Whence::Start).unwrap();
        black_box(Frames.read(&mut roundtrip).unwrap());
    });
}
