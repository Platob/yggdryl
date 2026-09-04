//! Time to first bytes and full-drain cost for the lazy byte stream.
//!
//! The fixture is identical after decoding in every case. `first` includes
//! constructing the stream and requesting its first item; `drain` constructs
//! and consumes a fresh stream. Reporting decoded bytes keeps gzip, zlib, and
//! zstd comparable with the plain handle instead of rewarding compression
//! ratio. `read_all` is the whole-value API baseline. `pread` measures one
//! bounded positional decode; repeated positional reads show the cost of
//! rebuilding a compression decoder instead of retaining one stream.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::IOBase;
use yggdryl::coding::gzip::Gzip;
use yggdryl::coding::zlib::Zlib;
use yggdryl::coding::zstd::Zstd;
use yggdryl::holder::Buffer;
use yggdryl::holder::buffered::BufferedOptions;

const FIXTURE: usize = crate::bench_profile::corpus(8 * 1024 * 1024, 512 * 1024);
const BATCHES: [(&str, usize); 3] = [
    ("4kib", 4 * 1024),
    ("64kib", 64 * 1024),
    ("1mib", 1024 * 1024),
];

fn fixture() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FIXTURE);
    let mut row = 0_u64;
    while bytes.len() < FIXTURE {
        bytes.extend_from_slice(
            format!("{row:08},AAPL,{:.2},XNAS,executed\n", row as f64 * 0.01).as_bytes(),
        );
        row += 1;
    }
    bytes.truncate(FIXTURE);
    bytes
}

fn cases(payload: &[u8]) -> Vec<(&'static str, Box<dyn IOBase>)> {
    vec![
        (
            "plain",
            Box::new(Buffer::from_bytes(payload.to_vec())) as Box<dyn IOBase>,
        ),
        (
            "gzip",
            Box::new(Gzip::new(Buffer::from_bytes(
                yggdryl::coding::gzip::dump(payload).expect("a gzip fixture"),
            ))),
        ),
        (
            "zlib",
            Box::new(Zlib::new(Buffer::from_bytes(
                yggdryl::coding::zlib::dump(payload).expect("a zlib fixture"),
            ))),
        ),
        (
            "zstd",
            Box::new(Zstd::new(Buffer::from_bytes(
                yggdryl::coding::zstd::dump(payload).expect("a zstd fixture"),
            ))),
        ),
    ]
}

fn first(handle: &dyn IOBase, batch_size: usize) -> usize {
    handle
        .pstream_bytes(0, batch_size)
        .expect("a byte stream")
        .next()
        .transpose()
        .expect("a readable first chunk")
        .map_or(0, |bytes| bytes.len())
}

fn drain(handle: &dyn IOBase, batch_size: usize) -> usize {
    handle
        .pstream_bytes(0, batch_size)
        .expect("a byte stream")
        .map(|bytes| bytes.expect("a readable chunk").len())
        .sum()
}

fn positional_drain(handle: &dyn IOBase, batch_size: usize, limit: usize) -> usize {
    let mut target = vec![0_u8; batch_size];
    let mut offset = 0_u64;
    while offset < limit as u64 {
        let wanted = target.len().min(limit - offset as usize);
        let read = handle
            .pread(offset, &mut target[..wanted])
            .expect("a positional byte range");
        if read == 0 {
            break;
        }
        offset += read as u64;
    }
    offset as usize
}

/// Verify the benchmark's retention claim with an observable cache counter.
///
/// Timing cannot prove that pages were not retained. A buffered coded handle
/// therefore performs one complete drain before the timer starts and reports
/// zero cached pages afterwards. The stream delegates through the wrapper;
/// callers that need a cache still get it through positional reads.
fn zero_cache_holds(payload: &[u8]) {
    let encoded = yggdryl::coding::gzip::dump(payload).expect("a gzip fixture");
    let handle = Gzip::new(Buffer::from_bytes(encoded)).buffered(BufferedOptions::default());
    assert_eq!(drain(&handle, 64 * 1024), payload.len());
    assert_eq!(handle.cached_pages(), 0);
    assert!(!handle.opened());
}

pub(crate) fn byte_stream_benchmarks(criterion: &mut Criterion) {
    let payload = fixture();
    let cases = cases(&payload);

    for (name, handle) in &cases {
        assert_eq!(drain(handle.as_ref(), 64 * 1024), payload.len(), "{name}");
        assert!(!handle.opened(), "{name} must stay closed after a drain");
    }
    zero_cache_holds(&payload);

    let mut first_group = criterion.benchmark_group("io_pstream_first");
    first_group.sample_size(10);
    for (batch_name, batch_size) in BATCHES {
        first_group.throughput(Throughput::Bytes(batch_size as u64));
        for (name, handle) in &cases {
            first_group.bench_with_input(
                BenchmarkId::new(*name, batch_name),
                &batch_size,
                |bencher, batch_size| {
                    bencher.iter(|| first(black_box(handle.as_ref()), *batch_size));
                },
            );
        }
    }

    // One positional read is the random-access baseline. A closed coded handle
    // decodes only far enough to fill it, but cannot retain that decoder for a
    // later positional call.
    let pread_size = 64 * 1024;
    first_group.throughput(Throughput::Bytes(pread_size as u64));
    for (name, handle) in &cases {
        let mut target = vec![0_u8; pread_size];
        first_group.bench_function(
            BenchmarkId::new(format!("{name}_pread"), "64kib"),
            |bencher| {
                bencher.iter(|| {
                    handle
                        .pread(0, black_box(&mut target))
                        .expect("a positional first read")
                });
            },
        );
    }
    first_group.finish();

    // Repeating positional reads makes the decoder restart at every offset.
    // One MiB keeps that deliberately quadratic coded baseline bounded while
    // still spanning sixteen 64-KiB calls.
    let positional_limit = 1024 * 1024;
    let mut positional_group = criterion.benchmark_group("io_pstream_repeated_pread");
    positional_group.sample_size(10);
    positional_group.throughput(Throughput::Bytes(positional_limit as u64));
    for (name, handle) in &cases {
        positional_group.bench_function(BenchmarkId::new(*name, "64kib_to_1mib"), |bencher| {
            bencher
                .iter(|| positional_drain(black_box(handle.as_ref()), 64 * 1024, positional_limit));
        });
    }
    positional_group.finish();

    let mut drain_group = criterion.benchmark_group("io_pstream_drain");
    drain_group.sample_size(10);
    drain_group.throughput(Throughput::Bytes(payload.len() as u64));
    for (batch_name, batch_size) in BATCHES {
        for (name, handle) in &cases {
            drain_group.bench_with_input(
                BenchmarkId::new(*name, batch_name),
                &batch_size,
                |bencher, batch_size| {
                    bencher.iter(|| drain(black_box(handle.as_ref()), *batch_size));
                },
            );
        }
    }

    // This baseline keeps the same decoded result but elects to retain the
    // complete value in one Vec, making the cost of that convenience visible.
    for (name, handle) in &cases {
        drain_group.bench_function(
            BenchmarkId::new(format!("{name}_read_all"), "whole"),
            |bencher| {
                bencher.iter(|| {
                    black_box(handle.as_ref())
                        .read_all_bytes()
                        .expect("a whole-value read")
                        .len()
                });
            },
        );
    }
    drain_group.finish();
}
