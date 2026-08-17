//! The three record methods, each over the one encoding underneath them.
//!
//! Measuring them together shows what each adds over a bare replacement: an
//! overwrite reads the stored schema and encodes, an append pays for one extra
//! full read of what is already there, and a merge pays for that read plus the
//! row-key encoding and the interleave that applies the matches.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::generic::IORecordOptions;
use yggdryl::io::IOBase;

use super::{ROWS, batch, handle, stored, wide};

pub(crate) fn round_trip_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("io_record");
    group.throughput(Throughput::Elements(ROWS as u64));

    let field = wide();
    let source = batch();

    group.bench_function("overwrite/ipc", |bencher| {
        bencher.iter(|| {
            let mut target = handle("bench-overwrite.arrows");
            let options = target
                .record_options()
                .expect("an implemented encoding")
                .with_schema(field.clone());
            target
                .write_arrow_batch_reader(
                    yggdryl::arrow::batch_reader(source.schema(), [black_box(source.clone())]),
                    &options,
                )
                .expect("the fixture must write");
        });
    });

    group.bench_function("append/ipc", |bencher| {
        bencher.iter(|| {
            let mut target = stored("bench-append.arrows");
            let options = target
                .record_options()
                .expect("an implemented encoding")
                .with_schema(field.clone());
            target
                .append_arrow_batch_reader(
                    yggdryl::arrow::batch_reader(source.schema(), [black_box(source.clone())]),
                    &options,
                )
                .expect("the fixture must append");
        });
    });

    group.bench_function("merge/ipc", |bencher| {
        bencher.iter(|| {
            let mut target = stored("bench-merge.arrows");
            let options = target
                .record_options()
                .expect("an implemented encoding")
                .with_schema(field.clone())
                .with_merge_by_names(["id"]);
            target
                .write_arrow_batch_reader(
                    yggdryl::arrow::batch_reader(source.schema(), [black_box(source.clone())]),
                    &options,
                )
                .expect("the fixture must merge");
        });
    });

    group.bench_function("read/ipc", |bencher| {
        let handle = stored("bench-read.arrows");
        let options = handle.record_options().expect("an implemented encoding");
        bencher.iter(|| {
            black_box(&handle)
                .read_arrow_batch_reader(&options)
                .expect("the fixture must read")
                .map(|batch| batch.expect("a decodable batch").num_rows())
                .sum::<usize>()
        });
    });

    group.finish();
}
