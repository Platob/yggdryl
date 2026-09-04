//! Throughput per algorithm and per size, and what this module costs over the
//! protocol it wraps.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::DigestAlgorithm;
use yggdryl::xxhash::{self, Xxh3_64, Xxh3_128, Xxh32, Xxh64};

use super::{SIZES, label, payload};

/// Every algorithm at every size, reported in bytes per second.
pub(crate) fn size_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("xxhash_size");
    for length in SIZES {
        let data = payload(length);
        group.throughput(Throughput::Bytes(length as u64));
        let size = label(length);
        group.bench_function(format!("xxh32/{size}"), |bencher| {
            bencher.iter(|| xxhash::xxh32(black_box(&data)));
        });
        group.bench_function(format!("xxh64/{size}"), |bencher| {
            bencher.iter(|| xxhash::xxh64(black_box(&data)));
        });
        group.bench_function(format!("xxh3_64/{size}"), |bencher| {
            bencher.iter(|| xxhash::xxh3_64(black_box(&data)));
        });
        group.bench_function(format!("xxh3_128/{size}"), |bencher| {
            bencher.iter(|| xxhash::xxh3_128(black_box(&data)));
        });
    }
    group.finish();
}

/// This module's one-shot against a direct call into the protocol crate.
///
/// The number being measured is wrapper overhead and nothing else: both rows
/// hash the same bytes with the same implementation, so the difference is the
/// argument normalization this module does at its boundary. The small sizes
/// are where a fixed per-call cost still shows; from a few kilobytes up the
/// two rows should sit inside each other's noise.
pub(crate) fn wrapper_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("xxhash_wrapper");
    for length in [
        1_usize,
        64,
        240,
        4096,
        crate::bench_profile::corpus(1024 * 1024, 256 * 1024),
    ] {
        let data = payload(length);
        group.throughput(Throughput::Bytes(length as u64));
        let size = label(length);
        group.bench_function(format!("yggdryl/{size}"), |bencher| {
            bencher.iter(|| xxhash::xxh3_64(black_box(&data)));
        });
        group.bench_function(format!("twox_hash/{size}"), |bencher| {
            bencher.iter(|| twox_hash::xxhash3_64::Hasher::oneshot(black_box(&data)));
        });
        group.bench_function(format!("digest_value/{size}"), |bencher| {
            bencher.iter(|| DigestAlgorithm::Xxh3_64.digest(black_box(&data)));
        });
    }
    group.finish();
}

/// Streaming in stream-sized chunks against hashing the payload whole.
///
/// The chunk is the same bounded window `pstream_bytes` yields, so this is the
/// cost a handle digest pays for never holding the object.
pub(crate) fn streaming_benchmarks(criterion: &mut Criterion) {
    const WINDOW: usize = 64 * 1024;
    let mut group = criterion.benchmark_group("xxhash_streaming");
    for length in [
        crate::bench_profile::corpus(1024 * 1024, 256 * 1024),
        crate::bench_profile::corpus(64 * 1024 * 1024, 1024 * 1024),
    ] {
        let data = payload(length);
        group.throughput(Throughput::Bytes(length as u64));
        let size = label(length);
        group.bench_function(format!("one_shot/{size}"), |bencher| {
            bencher.iter(|| xxhash::xxh3_64(black_box(&data)));
        });
        group.bench_function(format!("streamed/{size}"), |bencher| {
            bencher.iter(|| {
                let mut state = Xxh3_64::new();
                for chunk in black_box(&data).chunks(WINDOW) {
                    state.write_bytes(chunk);
                }
                state.as_u64()
            });
        });
        group.bench_function(format!("streamed_dispatched/{size}"), |bencher| {
            bencher.iter(|| {
                let mut digester = DigestAlgorithm::Xxh3_64.digester();
                for chunk in black_box(&data).chunks(WINDOW) {
                    digester.write_bytes(chunk);
                }
                digester.as_digest()
            });
        });
    }

    // The four states side by side at one size, so the widths compare.
    let data = payload(crate::bench_profile::corpus(1024 * 1024, 256 * 1024));
    group.throughput(Throughput::Bytes(data.len() as u64));
    group.bench_function("state/xxh32", |bencher| {
        bencher.iter(|| {
            let mut state = Xxh32::new();
            state.write_bytes(black_box(&data));
            state.as_u32()
        });
    });
    group.bench_function("state/xxh64", |bencher| {
        bencher.iter(|| {
            let mut state = Xxh64::new();
            state.write_bytes(black_box(&data));
            state.as_u64()
        });
    });
    group.bench_function("state/xxh3_64", |bencher| {
        bencher.iter(|| {
            let mut state = Xxh3_64::new();
            state.write_bytes(black_box(&data));
            state.as_u64()
        });
    });
    group.bench_function("state/xxh3_128", |bencher| {
        bencher.iter(|| {
            let mut state = Xxh3_128::new();
            state.write_bytes(black_box(&data));
            state.as_u128()
        });
    });
    group.finish();
}
