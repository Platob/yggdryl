//! The coupling beside the digest it wraps, the value's projections, and
//! reading an instant out of a value.

use std::hint::black_box;

use criterion::{Criterion, Throughput};

use yggdryl::txhash::{self, TxHash, TxHasher};
use yggdryl::{DigestAlgorithm, Scalar, TimeUnit, Timezone, xxhash};

/// The sizes where a call's fixed cost and the XXH3 size branches show.
const SIZES: [usize; 4] = [16, 240, 4096, 64 * 1024];

/// What coupling an instant costs over the digest alone.
///
/// Both columns hash the same bytes with the same implementation; the
/// difference is laying the instant beside the answer.
pub(crate) fn coupling_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("txhash_coupling");
    for size in SIZES {
        let payload = super::payload(size);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_function(format!("xxh3/{size}"), |bencher| {
            bencher.iter(|| xxhash::xxh3(black_box(&payload)));
        });
        group.bench_function(format!("txh3/{size}"), |bencher| {
            bencher.iter(|| txhash::txh3(black_box(&payload), super::INSTANT));
        });
        group.bench_function(format!("txh128/{size}"), |bencher| {
            bencher.iter(|| txhash::txh128(black_box(&payload), super::INSTANT));
        });
    }
    group.finish();

    let hasher = TxHasher::new(DigestAlgorithm::Xxh3).with_seed(7);
    let row = Scalar::from_sequence([
        Scalar::from(1_234_567_i64),
        Scalar::from("XNAS"),
        Scalar::from(150.25_f64),
        Scalar::from("AAPL"),
    ]);
    let mut group = criterion.benchmark_group("txhash_hasher");
    group.bench_function("digest/240", |bencher| {
        let payload = super::payload(240);
        bencher.iter(|| hasher.digest(black_box(&payload), super::INSTANT));
    });
    group.bench_function("digest_scalar/four_column_row", |bencher| {
        bencher.iter(|| hasher.digest_scalar(black_box(&row), super::INSTANT));
    });
    group.bench_function("scalar_txhash/four_column_row", |bencher| {
        bencher.iter(|| black_box(&row).txhash(super::INSTANT, DigestAlgorithm::Xxh3));
    });
    group.finish();
}

/// The value's projections: bytes out and back, the spelling, a restatement.
pub(crate) fn value_benchmarks(criterion: &mut Criterion) {
    let value = txhash::txh3(b"AAPL", super::INSTANT);
    let bytes = value.into_bytes();
    let spelled = value.to_string();
    let mut group = criterion.benchmark_group("txhash_value");
    group.bench_function("into_bytes", |bencher| {
        bencher.iter(|| black_box(value).into_bytes());
    });
    group.bench_function("from_bytes", |bencher| {
        bencher.iter(|| {
            TxHash::from_bytes(
                TimeUnit::Microsecond,
                DigestAlgorithm::Xxh3,
                black_box(&bytes),
            )
            .expect("the exact width")
        });
    });
    group.bench_function("to_string", |bencher| {
        bencher.iter(|| black_box(value).to_string());
    });
    group.bench_function("from_str", |bencher| {
        bencher.iter(|| TxHash::from_str(black_box(&spelled)).expect("the canonical spelling"));
    });
    group.bench_function("with_unit", |bencher| {
        bencher.iter(|| {
            black_box(value)
                .with_unit(TimeUnit::Second)
                .expect("a coarser unit")
        });
    });
    group.bench_function("into_datetime", |bencher| {
        bencher.iter(|| black_box(value).into_datetime());
    });
    group.bench_function("into_scalar", |bencher| {
        bencher.iter(|| black_box(value).into_scalar());
    });
    group.finish();
}

/// Reading an instant out of each spelling a value can hold.
pub(crate) fn instant_benchmarks(criterion: &mut Criterion) {
    let integer = Scalar::from(super::INSTANT);
    let zoned = Scalar::from_datetime(1_700_000_000, TimeUnit::Second, Timezone::UTC)
        .expect("a valid datetime");
    let text = Scalar::from("2023-11-14T22:13:20Z");
    let mut group = criterion.benchmark_group("txhash_instant");
    for (label, value) in [("integer", &integer), ("datetime", &zoned), ("text", &text)] {
        group.bench_function(label, |bencher| {
            bencher.iter(|| {
                txhash::unix_from_scalar(black_box(value), TimeUnit::Microsecond)
                    .expect("an instant")
            });
        });
    }
    group.bench_function("restate_unix", |bencher| {
        bencher.iter(|| {
            txhash::restate_unix(
                black_box(1_999),
                TimeUnit::Nanosecond,
                TimeUnit::Microsecond,
            )
            .expect("a coarser unit")
        });
    });
    group.bench_function("unix_now", |bencher| {
        bencher.iter(|| txhash::unix_now(TimeUnit::Microsecond).expect("the clock"));
    });
    group.finish();
}
