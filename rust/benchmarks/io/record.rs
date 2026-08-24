//! The three record methods, each over the one encoding underneath them.
//!
//! Measuring them together shows what each adds over a bare replacement: an
//! overwrite reads the stored schema and encodes, an append pays for one extra
//! full read of what is already there, and a merge pays for that read plus the
//! row-key encoding and the interleave that applies the matches.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput};
use yggdryl::generic::IORecordOptions;
use yggdryl::io::IOMedia;

use super::{ROWS, batch, handle, reader, stored, stored_with, wide};

pub(crate) fn round_trip_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("io_record");
    group.throughput(Throughput::Elements(ROWS as u64));

    let field = wide();
    let source = batch();

    group.bench_function("overwrite/ipc", |bencher| {
        bencher.iter_batched(
            || {
                let target = handle("bench-overwrite.arrows");
                let options = target
                    .record_options()
                    .expect("an implemented encoding")
                    .with_field(field.clone());
                (target, options)
            },
            |(mut target, options)| {
                target
                    .overwrite_arrow_reader(reader(black_box(&source)), &options)
                    .expect("the fixture must write");
            },
            BatchSize::LargeInput,
        );
    });

    group.bench_function("append/ipc", |bencher| {
        bencher.iter_batched(
            || {
                let target = stored_with("bench-append.arrows", &source);
                let options = target
                    .record_options()
                    .expect("an implemented encoding")
                    .with_field(field.clone());
                (target, options)
            },
            |(mut target, options)| {
                target
                    .append_arrow_reader(reader(black_box(&source)), &options)
                    .expect("the fixture must append");
            },
            BatchSize::LargeInput,
        );
    });

    group.bench_function("merge/ipc", |bencher| {
        bencher.iter_batched(
            || {
                let target = stored_with("bench-merge.arrows", &source);
                let options = target
                    .record_options()
                    .expect("an implemented encoding")
                    .with_field(field.clone())
                    .with_merge_by_names(["id"]);
                (target, options)
            },
            |(mut target, options)| {
                target
                    .merge_arrow_reader(reader(black_box(&source)), &options)
                    .expect("the fixture must merge");
            },
            BatchSize::LargeInput,
        );
    });

    group.bench_function("overwrite_record_batch/ipc", |bencher| {
        bencher.iter_batched(
            || {
                let target = handle("bench-overwrite-batch.arrows");
                let options = target
                    .record_options()
                    .expect("an implemented encoding")
                    .with_field(field.clone());
                (target, options)
            },
            |(mut target, options)| {
                target
                    .overwrite_arrow_batch(black_box(source.clone()), &options)
                    .expect("the fixture batch must overwrite");
            },
            BatchSize::LargeInput,
        );
    });

    group.bench_function("append_record_batch/ipc", |bencher| {
        bencher.iter_batched(
            || {
                let target = stored_with("bench-append-batch.arrows", &source);
                let options = target
                    .record_options()
                    .expect("an implemented encoding")
                    .with_field(field.clone());
                (target, options)
            },
            |(mut target, options)| {
                target
                    .append_arrow_batch(black_box(source.clone()), &options)
                    .expect("the fixture batch must append");
            },
            BatchSize::LargeInput,
        );
    });

    group.bench_function("merge_record_batch/ipc", |bencher| {
        bencher.iter_batched(
            || {
                let target = stored_with("bench-merge-batch.arrows", &source);
                let options = target
                    .record_options()
                    .expect("an implemented encoding")
                    .with_field(field.clone())
                    .with_merge_by_names(["id"]);
                (target, options)
            },
            |(mut target, options)| {
                target
                    .merge_arrow_batch(black_box(source.clone()), &options)
                    .expect("the fixture batch must merge");
            },
            BatchSize::LargeInput,
        );
    });

    group.bench_function("read/ipc", |bencher| {
        let handle = stored("bench-read.arrows");
        let options = handle.record_options().expect("an implemented encoding");
        bencher.iter(|| {
            black_box(&handle)
                .read_arrow_reader(&options)
                .expect("the fixture must read")
                .map(|batch| batch.expect("a decodable batch").num_rows())
                .sum::<usize>()
        });
    });

    group.finish();
}
