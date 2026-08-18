//! Projection over a wide schema: 3 of 40 columns against all 40.
//!
//! Avro interleaves columns per record, so a projection cannot skip reading
//! the row - what it skips is decoding and allocating the 37 unselected
//! columns, whose bytes are jumped by their length prefixes. The skip cost
//! should stay near-linear in the skipped bytes, which is what the ratio
//! between these two cases watches.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::avro::AvroOptions;
use yggdryl::io::Buffer;
use yggdryl::{DataType, Field, Url, avro};

/// Rows in the wide fixture.
const ROWS: usize = 8_192;

/// Columns in the wide fixture.
const COLUMNS: usize = 40;

/// The wide root: forty columns alternating long and string.
fn wide() -> Field {
    let fields = (0..COLUMNS).map(|index| {
        if index % 2 == 0 {
            DataType::Int64.required_field(format!("c{index:02}"))
        } else {
            DataType::Utf8.required_field(format!("c{index:02}"))
        }
    });
    DataType::from_fields(fields)
        .expect("a struct")
        .required_field("row")
}

/// Three columns spread across the row, so skips happen before and between.
fn narrow() -> Field {
    let root = wide();
    let fields = root.fields();
    DataType::from_fields([fields[0].clone(), fields[19].clone(), fields[39].clone()])
        .expect("a struct")
        .required_field("row")
}

pub(crate) fn projection_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("codec/avro_projection");
    group.sample_size(30);

    let root = wide();
    let arrow_schema = root.to_arrow_schema().expect("an arrow schema");
    let columns: Vec<ArrayRef> = (0..COLUMNS)
        .map(|index| -> ArrayRef {
            if index % 2 == 0 {
                Arc::new(Int64Array::from_iter_values(
                    (0..ROWS).map(|row| (row * index) as i64),
                ))
            } else {
                Arc::new(StringArray::from_iter_values(
                    (0..ROWS).map(|row| format!("value-{index:02}-{row:05}")),
                ))
            }
        })
        .collect();
    let batch = RecordBatch::try_new(arrow_schema.clone(), columns).expect("a batch");

    let mut stored = Buffer::new().with_media_type(
        Url::from_str("file:///wide.avro")
            .expect("a url")
            .media_type(),
    );
    // The null codec isolates the skip itself: with compression on, the
    // whole block is decompressed either way and the decompression dominates.
    let options = AvroOptions::new().with_codec("null");
    avro::write_batch_reader(
        &mut stored,
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )
    .expect("the wide fixture encodes");

    let narrow = narrow();
    // Proven once outside the timers: the projection keeps exactly 3 columns.
    let projected: Vec<RecordBatch> = avro::read_batch_reader(&stored, Some(&narrow), &options)
        .expect("the projected fixture reads")
        .collect::<Result<_, _>>()
        .expect("the projected fixture decodes");
    assert_eq!(projected[0].num_columns(), 3);
    assert_eq!(
        projected.iter().map(RecordBatch::num_rows).sum::<usize>(),
        ROWS
    );

    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("all_40_of_40", |bencher| {
        bencher.iter(|| {
            avro::read_batch_reader(black_box(&stored), None, black_box(&options))
                .expect("reads")
                .collect::<Result<Vec<_>, _>>()
                .expect("decodes")
        });
    });
    group.bench_function("skip_37_of_40", |bencher| {
        bencher.iter(|| {
            avro::read_batch_reader(black_box(&stored), Some(black_box(&narrow)), &options)
                .expect("reads")
                .collect::<Result<Vec<_>, _>>()
                .expect("decodes")
        });
    });

    group.finish();
}
