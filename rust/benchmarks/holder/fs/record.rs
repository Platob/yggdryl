//! Record round trips over a foreign filesystem, against the same encoding
//! over the native in-memory handle.
//!
//! The three record methods are inherited, not reimplemented, so these
//! measure exactly what the wrapper adds to an IPC or Parquet round trip:
//! one staged materialization on the way in and one whole-value publish on
//! the way out.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::holder::fs::File as FsFile;
use yggdryl::media::IORecordOptions;
use yggdryl::{IOBase, IOMedia};

use super::{ROWS, batch, buffer, memory, store, wide};

pub(crate) fn record_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("fs_record");
    group.throughput(Throughput::Elements(ROWS as u64));

    let source = batch();
    let field = wide();

    let mut names = vec!["bucket/bench.arrows"];
    if cfg!(feature = "parquet") {
        names.push("bucket/bench.parquet");
    }

    for name in names {
        let encoding = name.rsplit('.').next().unwrap_or("arrows");

        group.bench_function(format!("write/fs_memory/{encoding}"), |bencher| {
            let filesystem = memory();
            bencher.iter(|| {
                let mut handle =
                    FsFile::from_location(filesystem.clone(), name).expect("a valid location");
                let options = handle
                    .record_options()
                    .expect("an implemented encoding")
                    .with_field(field.clone());
                handle
                    .overwrite_arrow_reader(
                        yggdryl::arrow::batch_reader(source.schema(), [black_box(source.clone())]),
                        &options,
                    )
                    .expect("the fixture must write");
                handle.close().expect("the write publishes");
            });
        });

        group.bench_function(format!("write/buffer/{encoding}"), |bencher| {
            bencher.iter(|| {
                let mut handle = buffer(name);
                let options = handle
                    .record_options()
                    .expect("an implemented encoding")
                    .with_field(field.clone());
                handle
                    .overwrite_arrow_reader(
                        yggdryl::arrow::batch_reader(source.schema(), [black_box(source.clone())]),
                        &options,
                    )
                    .expect("the fixture must write");
            });
        });

        group.bench_function(format!("read/fs_memory/{encoding}"), |bencher| {
            let filesystem = memory();
            let mut handle = FsFile::from_location(filesystem, name).expect("a valid location");
            store(&mut handle, &source);
            let options = handle.record_options().expect("an implemented encoding");
            bencher.iter(|| {
                black_box(&handle)
                    .read_arrow_reader(&options)
                    .expect("the fixture must read")
                    .map(|batch| batch.expect("a decodable batch").num_rows())
                    .sum::<usize>()
            });
        });

        group.bench_function(format!("read/buffer/{encoding}"), |bencher| {
            let mut handle = buffer(name);
            let options = handle
                .record_options()
                .expect("an implemented encoding")
                .with_field(field.clone());
            handle
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(source.schema(), [source.clone()]),
                    &options,
                )
                .expect("the fixture must write");
            let options = handle.record_options().expect("an implemented encoding");
            bencher.iter(|| {
                black_box(&handle)
                    .read_arrow_reader(&options)
                    .expect("the fixture must read")
                    .map(|batch| batch.expect("a decodable batch").num_rows())
                    .sum::<usize>()
            });
        });
    }

    group.finish();
}
