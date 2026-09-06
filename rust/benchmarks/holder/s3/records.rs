//! Records over S3, which is what the byte handle exists to carry.
//!
//! There is no `object_store` leg here: it is a byte store, and reading rows
//! through it means bolting a Parquet or IPC reader on top. What this group
//! measures is the thing that matters for a caller - the round trip from
//! Arrow batches to an object and back - and the difference between doing it
//! on an opened handle and a closed one, which is the difference between one
//! metadata request and one per question.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use criterion::{Criterion, Throughput};
use yggdryl::{DataType, Field, IOBase, IOMedia};

use super::{ROWS, location, options, store};

/// The four-column root the round trips carry.
fn wide() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Utf8.required_field("venue"),
    ])
    .expect("a valid struct root")
    .required_field("row")
}

/// One batch holding every row of the fixture.
fn batch() -> RecordBatch {
    let ids: Vec<i64> = (0..ROWS).collect();
    #[allow(clippy::cast_precision_loss)]
    let prices: Vec<f64> = ids.iter().map(|id| *id as f64).collect();
    RecordBatch::try_new(
        wide().into_arrow_schema().expect("a projectable root"),
        vec![
            Arc::new(Int64Array::from(ids.clone())),
            Arc::new(StringArray::from(
                ids.iter()
                    .map(|id| format!("SYMBOL-{id:08}"))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(prices)),
            Arc::new(StringArray::from(
                ids.iter()
                    .map(|id| format!("VENUE-{id:08}"))
                    .collect::<Vec<_>>(),
            )),
        ],
    )
    .expect("a batch matching the root")
}

pub(crate) fn record_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("s3_records");
    group.throughput(Throughput::Elements(ROWS as u64));

    let store = store();
    let source = batch();

    for name in ["bench/rows.arrows", "bench/rows.parquet"] {
        let encoding = name.rsplit('.').next().expect("an extension");
        // Parquet is a non-default feature; skip its leg when it is not in.
        if encoding == "parquet" && !cfg!(feature = "parquet") {
            continue;
        }
        let mut handle = yggdryl::holder::s3::file_with(&location(name), options(&store))
            .expect("an object handle");
        let record_options = handle.record_options().expect("an implemented encoding");
        handle
            .overwrite_arrow_batch(source.clone(), &record_options)
            .expect("the fixture writes");

        group.bench_function(format!("read/{encoding}"), |bencher| {
            bencher.iter(|| {
                let reader = black_box(&handle)
                    .read_arrow_reader(&record_options)
                    .expect("a reader");
                black_box(
                    reader
                        .map(|batch| batch.expect("a batch").num_rows())
                        .sum::<usize>(),
                )
            });
        });

        // The same read on an opened handle, where the metadata questions the
        // encoding asks are answered from the scope rather than the store.
        let mut opened = yggdryl::holder::s3::file_with(&location(name), options(&store))
            .expect("an object handle");
        opened.open().expect("an open");
        group.bench_function(format!("read_opened/{encoding}"), |bencher| {
            bencher.iter(|| {
                let reader = black_box(&opened)
                    .read_arrow_reader(&record_options)
                    .expect("a reader");
                black_box(
                    reader
                        .map(|batch| batch.expect("a batch").num_rows())
                        .sum::<usize>(),
                )
            });
        });

        group.bench_function(format!("overwrite/{encoding}"), |bencher| {
            let mut target = yggdryl::holder::s3::file_with(
                &location(&format!("bench/write.{encoding}")),
                options(&store),
            )
            .expect("an object handle");
            let write_options = target.record_options().expect("an implemented encoding");
            bencher.iter(|| {
                target
                    .overwrite_arrow_batch(source.clone(), &write_options)
                    .expect("a written batch");
            });
        });
    }

    group.finish();
}
