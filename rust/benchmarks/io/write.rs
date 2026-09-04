//! Explicit write intent, adapters, and the stateful media redirections.
//!
//! The generic reader and record-batch triplets are timed in `record`; this
//! page covers the parts a straight round trip cannot distinguish: required-
//! mode dispatch, no-op streams, stopping at a declared row bound, field
//! casting plus selection, wide nested values, and the shared stateful media
//! surface. Stateful setup is outside the timer, so append and merge
//! report the operation rather than the cost of preparing their stored side.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::builder::{Int32Builder, ListBuilder};
use arrow_array::{
    ArrayRef, BinaryArray, Int32Array, Int64Array, ListArray, RecordBatch, StringArray, StructArray,
};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use criterion::measurement::WallTime;
use criterion::{BatchSize, BenchmarkGroup, Criterion, Throughput};
use yggdryl::IOMedia;
use yggdryl::arrow::BatchReader;
use yggdryl::avro::Avro;
use yggdryl::generic::{IORecordOptions, Media};
use yggdryl::holder::Holder;
use yggdryl::ipc::Ipc;
use yggdryl::parquet::Parquet;
use yggdryl::{DataType, Field, IOMode, Scalar};

use super::{batch, handle, reader, stored_with, wide};

/// The smaller stateful fixture keeps twelve format/method measurements
/// practical while remaining large enough that encoding dominates dispatch.
const STATEFUL_ROWS: usize = crate::bench_profile::corpus(4_096, 512);

/// Rows in the shaping fixtures.
const SHAPE_ROWS: usize = crate::bench_profile::corpus(8_192, 1_024);

/// Scalar columns before the nested columns in the wide fixture.
const WIDE_COLUMNS: usize = 40;

/// Invoke the three canonical write intents of one stateful media type.
fn stateful_triplet<T, Empty, Seeded>(
    group: &mut BenchmarkGroup<'_, WallTime>,
    label: &str,
    source: &RecordBatch,
    empty: Empty,
    seeded: Seeded,
) where
    T: IOMedia,
    Empty: Fn() -> T,
    Seeded: Fn(bool) -> T,
{
    group.bench_function(format!("{label}/overwrite_arrow_reader"), |bencher| {
        bencher.iter_batched(
            &empty,
            |mut target| {
                let options = target.record_options().expect("an implemented encoding");
                target
                    .overwrite_arrow_reader(reader(black_box(source)), &options)
                    .expect("the stateful fixture must overwrite");
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function(format!("{label}/append_arrow_reader"), |bencher| {
        bencher.iter_batched(
            || seeded(false),
            |mut target| {
                let options = target.record_options().expect("an implemented encoding");
                target
                    .append_arrow_reader(reader(black_box(source)), &options)
                    .expect("the stateful fixture must append");
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function(format!("{label}/merge_arrow_reader"), |bencher| {
        bencher.iter_batched(
            || seeded(true),
            |mut target| {
                let options = target.record_options().expect("an implemented encoding");
                target
                    .merge_arrow_reader(reader(black_box(source)), &options)
                    .expect("the stateful fixture must merge");
            },
            BatchSize::LargeInput,
        );
    });
}

fn ipc_target(source: &RecordBatch, seeded: bool, merge: bool) -> Ipc<yggdryl::holder::Buffer> {
    let mut target = Ipc::new(handle("stateful.arrows")).with_field(wide());
    if seeded {
        let options = target.record_options().expect("an implemented encoding");
        target
            .overwrite_arrow_reader(reader(source), &options)
            .expect("the IPC fixture must seed");
    }
    if merge {
        target
            .options_mut()
            .set_merge_by_names(vec!["id".to_owned()]);
    }
    target
}

fn parquet_target(
    source: &RecordBatch,
    seeded: bool,
    merge: bool,
) -> Parquet<yggdryl::holder::Buffer> {
    let mut target = Parquet::new(handle("stateful.parquet")).with_field(wide());
    if seeded {
        let options = target.record_options().expect("an implemented encoding");
        target
            .overwrite_arrow_reader(reader(source), &options)
            .expect("the Parquet fixture must seed");
    }
    if merge {
        target
            .options_mut()
            .set_merge_by_names(vec!["id".to_owned()]);
    }
    target
}

fn avro_target(source: &RecordBatch, seeded: bool, merge: bool) -> Avro<yggdryl::holder::Buffer> {
    let mut target = Avro::new(handle("stateful.avro")).with_field(wide());
    if seeded {
        let options = target.record_options().expect("an implemented encoding");
        target
            .overwrite_arrow_reader(reader(source), &options)
            .expect("the Avro fixture must seed");
    }
    if merge {
        target
            .options_mut()
            .set_merge_by_names(vec!["id".to_owned()]);
    }
    target
}

/// Use IPC underneath the generic enum so this leg isolates the enum's
/// redirection from the format comparisons immediately above it.
fn media_target(source: &RecordBatch, seeded: bool, merge: bool) -> Media {
    let mut ipc = Ipc::new(Holder::buffer(handle("stateful-media.arrows"))).with_field(wide());
    if seeded {
        let options = ipc.record_options().expect("an implemented encoding");
        ipc.overwrite_arrow_reader(reader(source), &options)
            .expect("the generic media fixture must seed");
    }
    if merge {
        ipc.options_mut().set_merge_by_names(vec!["id".to_owned()]);
    }
    Media::from(ipc)
}

/// Text's writable record shape: the encoder consumes the binary body column.
fn text_source() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "body",
        ArrowDataType::Binary,
        false,
    )]));
    RecordBatch::try_new(
        schema,
        vec![Arc::new(BinaryArray::from_iter_values(
            (0..STATEFUL_ROWS).map(|row| format!("event-{row:08}").into_bytes()),
        ))],
    )
    .expect("a text-line fixture")
}

fn text_target(source: &RecordBatch, seeded: bool) -> yggdryl::holder::Buffer {
    let mut target = handle("stateful.log");
    if seeded {
        let options = target.record_options().expect("text record options");
        target
            .overwrite_arrow_reader(reader(source), &options)
            .expect("the text fixture must seed");
    }
    target
}

/// Incoming rows whose declared field both widens a value and is then
/// narrowed/reordered by `select_by_names`.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
fn cast_source() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ArrowField::new("price", ArrowDataType::Int32, false),
        ArrowField::new("venue", ArrowDataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from_iter_values(
                (0..SHAPE_ROWS).map(|row| format!("SYMBOL-{row:08}")),
            )),
            Arc::new(Int32Array::from_iter_values(
                (0..SHAPE_ROWS).map(|row| row as i32),
            )),
            Arc::new(StringArray::from_iter_values(
                (0..SHAPE_ROWS).map(|row| format!("VENUE-{row:08}")),
            )),
        ],
    )
    .expect("a castable fixture")
}

fn cast_field() -> Field {
    DataType::from_fields([
        DataType::Utf8.required_field("symbol"),
        DataType::Int64.required_field("price"),
        DataType::Utf8.required_field("venue"),
    ])
    .expect("a struct root")
    .required_field("row")
}

/// Forty scalar columns plus a struct and a list: one fixture exercises both
/// wide schema traversal and recursive encoding rather than conflating either
/// with the four-column round-trip fixture.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
fn nested_wide() -> (Field, RecordBatch) {
    let mut fields = (0..WIDE_COLUMNS)
        .map(|column| DataType::Int64.required_field(format!("c{column:02}")))
        .collect::<Vec<_>>();
    fields.push(
        DataType::from_fields([
            DataType::Int64.required_field("sequence"),
            DataType::Utf8.required_field("label"),
        ])
        .expect("a nested struct")
        .required_field("details"),
    );
    fields.push(DataType::list(DataType::Int32.required_field("item")).required_field("tags"));
    let field = DataType::from_fields(fields)
        .expect("a wide nested root")
        .required_field("row");
    let schema = field.clone().into_arrow_schema().expect("an Arrow schema");

    let mut columns = (0..WIDE_COLUMNS)
        .map(|column| {
            Arc::new(Int64Array::from_iter_values(
                (0..SHAPE_ROWS).map(move |row| (row * (column + 1)) as i64),
            )) as ArrayRef
        })
        .collect::<Vec<_>>();

    let detail_fields = match schema.field(WIDE_COLUMNS).data_type() {
        ArrowDataType::Struct(fields) => fields.clone(),
        other => panic!("expected struct details, got {other}"),
    };
    columns.push(Arc::new(StructArray::new(
        detail_fields,
        vec![
            Arc::new(Int64Array::from_iter_values(
                (0..SHAPE_ROWS).map(|row| row as i64),
            )),
            Arc::new(StringArray::from_iter_values(
                (0..SHAPE_ROWS).map(|row| format!("detail-{row:08}")),
            )),
        ],
        None,
    )));

    let mut tags = ListBuilder::new(Int32Builder::new());
    for row in 0..SHAPE_ROWS {
        tags.append_value([Some(row as i32), Some((row + 1) as i32)]);
    }
    let tags = {
        let built = tags.finish();
        let (_, offsets, values, nulls) = built.into_parts();
        let child = match schema.field(WIDE_COLUMNS + 1).data_type() {
            ArrowDataType::List(child) => Arc::clone(child),
            other => panic!("expected list tags, got {other}"),
        };
        ListArray::new(child, offsets, values, nulls)
    };
    columns.push(Arc::new(tags));

    let batch = RecordBatch::try_new(schema, columns).expect("a wide nested batch");
    (field, batch)
}

/// A lazy sequence of one-row batches that records exactly how far a write
/// pulls. This is a benchmark instrument, not a second record implementation.
struct CountedBatches {
    batch: RecordBatch,
    remaining: usize,
    pulled: Arc<AtomicUsize>,
}

/// One ordinary Rust row. The record benchmarks pass these values to the
/// `TryInto<Scalar>` adapters; they do not pre-build binding-side records or a
/// parallel schema object.
#[derive(Clone, Copy)]
struct NativeRow {
    id: i64,
    symbol: &'static str,
    price: f64,
    venue: &'static str,
}

impl From<NativeRow> for Scalar {
    fn from(row: NativeRow) -> Self {
        Scalar::from_sequence([
            Scalar::from(row.id),
            Scalar::from(row.symbol),
            Scalar::from(row.price),
            Scalar::from(row.venue),
        ])
    }
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
fn native_rows() -> Vec<NativeRow> {
    const SYMBOLS: [&str; 4] = ["AAPL", "MSFT", "AMD", "NVDA"];
    const VENUES: [&str; 3] = ["XNAS", "XNYS", "XPAR"];

    (0..STATEFUL_ROWS)
        .map(|row| NativeRow {
            id: row as i64,
            symbol: SYMBOLS[row % SYMBOLS.len()],
            price: row as f64 / 100.0,
            venue: VENUES[row % VENUES.len()],
        })
        .collect()
}

impl Iterator for CountedBatches {
    type Item = RecordBatch;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.pulled.fetch_add(1, Ordering::Relaxed);
        Some(self.batch.clone())
    }
}

fn counted_reader(batch: &RecordBatch, batches: usize, pulled: Arc<AtomicUsize>) -> BatchReader {
    yggdryl::arrow::batch_reader(
        batch.schema(),
        CountedBatches {
            batch: batch.clone(),
            remaining: batches,
            pulled,
        },
    )
}

fn stateful_benchmarks(criterion: &mut Criterion) {
    let source = batch().slice(0, STATEFUL_ROWS);
    let mut stateful = criterion.benchmark_group("io_write_stateful");
    stateful.sample_size(10);
    stateful.throughput(Throughput::Elements(STATEFUL_ROWS as u64));

    stateful_triplet(
        &mut stateful,
        "ipc",
        &source,
        || ipc_target(&source, false, false),
        |merge| ipc_target(&source, true, merge),
    );
    stateful_triplet(
        &mut stateful,
        "parquet",
        &source,
        || parquet_target(&source, false, false),
        |merge| parquet_target(&source, true, merge),
    );
    stateful_triplet(
        &mut stateful,
        "avro",
        &source,
        || avro_target(&source, false, false),
        |merge| avro_target(&source, true, merge),
    );
    stateful_triplet(
        &mut stateful,
        "media_ipc",
        &source,
        || media_target(&source, false, false),
        |merge| media_target(&source, true, merge),
    );

    // Text has no stable row identity, so it deliberately exposes read,
    // overwrite, and append but refuses merge before consuming its reader.
    let text = text_source();
    stateful.bench_function("text/read_arrow_reader", |bencher| {
        bencher.iter_batched(
            || text_target(&text, true),
            |target| {
                let options = target.record_options().expect("text record options");
                let rows = target
                    .read_arrow_reader(&options)
                    .expect("the text fixture must read")
                    .map(|batch| batch.expect("a text batch").num_rows())
                    .sum::<usize>();
                black_box(rows);
            },
            BatchSize::LargeInput,
        );
    });
    stateful.bench_function("text/overwrite_arrow_reader", |bencher| {
        bencher.iter_batched(
            || text_target(&text, false),
            |mut target| {
                let options = target.record_options().expect("text record options");
                target
                    .overwrite_arrow_reader(reader(black_box(&text)), &options)
                    .expect("the text fixture must overwrite");
            },
            BatchSize::LargeInput,
        );
    });
    stateful.bench_function("text/append_arrow_reader", |bencher| {
        bencher.iter_batched(
            || text_target(&text, true),
            |mut target| {
                let options = target.record_options().expect("text record options");
                target
                    .append_arrow_reader(reader(black_box(&text)), &options)
                    .expect("the text fixture must append");
            },
            BatchSize::LargeInput,
        );
    });
    stateful.finish();
}

fn empty_no_op_benchmarks(criterion: &mut Criterion) {
    let source = batch().slice(0, STATEFUL_ROWS);
    let empty_schema = source.schema();
    let empty_batch = RecordBatch::new_empty(Arc::clone(&empty_schema));
    let mut no_op = criterion.benchmark_group("io_write_empty_noop");

    let mut append_target = stored_with("empty-append.arrows", &source);
    let append_options = append_target
        .record_options()
        .expect("an implemented encoding");
    no_op.bench_function("append_arrow_reader", |bencher| {
        bencher.iter(|| {
            append_target
                .append_arrow_reader(
                    yggdryl::arrow::batch_reader(
                        Arc::clone(&empty_schema),
                        std::iter::empty::<RecordBatch>(),
                    ),
                    black_box(&append_options),
                )
                .expect("an empty append is a no-op");
        });
    });
    no_op.bench_function("append_arrow_batch", |bencher| {
        bencher.iter(|| {
            append_target
                .append_arrow_batch(empty_batch.clone(), black_box(&append_options))
                .expect("an empty batch append is a no-op");
        });
    });

    let mut merge_target = stored_with("empty-merge.arrows", &source);
    let merge_options = merge_target
        .record_options()
        .expect("an implemented encoding")
        .with_merge_by_names(["id"]);
    no_op.bench_function("merge_arrow_reader", |bencher| {
        bencher.iter(|| {
            merge_target
                .merge_arrow_reader(
                    yggdryl::arrow::batch_reader(
                        Arc::clone(&empty_schema),
                        std::iter::empty::<RecordBatch>(),
                    ),
                    black_box(&merge_options),
                )
                .expect("an empty merge is a no-op");
        });
    });
    no_op.bench_function("merge_arrow_batch", |bencher| {
        bencher.iter(|| {
            merge_target
                .merge_arrow_batch(empty_batch.clone(), black_box(&merge_options))
                .expect("an empty batch merge is a no-op");
        });
    });
    no_op.finish();
}

fn native_record_benchmarks(criterion: &mut Criterion) {
    let rows = native_rows();
    let source = batch().slice(0, STATEFUL_ROWS);
    let options = handle("native-records.arrows")
        .record_options()
        .expect("an implemented encoding")
        .with_field(wide())
        .with_batch_row_size(512);
    let merging = options.clone().with_merge_by_names(["id"]);

    let mut group = criterion.benchmark_group("io_write_records");
    group.sample_size(10);
    group.throughput(Throughput::Elements(STATEFUL_ROWS as u64));

    group.bench_function("overwrite_records", |bencher| {
        bencher.iter_batched(
            || (handle("native-overwrite.arrows"), rows.clone()),
            |(mut target, rows)| {
                // Fixture cloning is setup; NativeRow -> Scalar::Sequence and
                // row validation stay inside the public operation being timed.
                target
                    .overwrite_records(black_box(rows), &options)
                    .expect("native rows must overwrite");
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("append_records", |bencher| {
        bencher.iter_batched(
            || (stored_with("native-append.arrows", &source), rows.clone()),
            |(mut target, rows)| {
                target
                    .append_records(black_box(rows), &options)
                    .expect("native rows must append");
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("merge_records", |bencher| {
        bencher.iter_batched(
            || (stored_with("native-merge.arrows", &source), rows.clone()),
            |(mut target, rows)| {
                target
                    .merge_records(black_box(rows), &merging)
                    .expect("native rows must merge");
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();

    // Empty overwrite has distinct behavior: it publishes the declared field
    // with zero rows. The adjacent append case is the native adapter's no-op
    // overhead; Arrow reader and batch no-ops live in the shared no-op group.
    let mut edge = criterion.benchmark_group("io_write_records_edge");
    edge.bench_function("overwrite_records/empty_field", |bencher| {
        bencher.iter_batched(
            || handle("native-empty-overwrite.arrows"),
            |mut target| {
                target
                    .overwrite_records(std::iter::empty::<NativeRow>(), &options)
                    .expect("an empty native overwrite must publish its field");
            },
            BatchSize::SmallInput,
        );
    });
    edge.bench_function("append_records/empty_noop", |bencher| {
        let mut target = stored_with("native-empty-append.arrows", &source);
        bencher.iter(|| {
            target
                .append_records(std::iter::empty::<NativeRow>(), black_box(&options))
                .expect("an empty native append must be a no-op");
        });
    });
    edge.finish();
}

/// Measure the three mode-selected public adapters independently from their
/// intent-specific counterparts. Setup supplies the stored side append and
/// merge require, so each sample includes only the selected operation.
fn mode_dispatch_benchmarks(criterion: &mut Criterion) {
    let source = batch().slice(0, STATEFUL_ROWS);
    let rows = native_rows();
    let plain = handle("mode-dispatch.arrows")
        .record_options()
        .expect("an implemented encoding")
        .with_field(wide())
        .with_batch_row_size(512);
    let merging = plain.clone().with_merge_by_names(["id"]);
    let mut group = criterion.benchmark_group("io_write_mode_dispatch");
    group.sample_size(10);
    group.throughput(Throughput::Elements(STATEFUL_ROWS as u64));

    for mode in IOMode::WRITE {
        let options = if mode == IOMode::Merge {
            &merging
        } else {
            &plain
        };
        let name = mode.as_str();

        group.bench_function(format!("write_arrow_reader/{name}"), |bencher| {
            bencher.iter_batched(
                || {
                    if mode == IOMode::Overwrite {
                        handle("mode-reader.arrows")
                    } else {
                        stored_with("mode-reader.arrows", &source)
                    }
                },
                |mut target| {
                    target
                        .write_arrow_reader(reader(black_box(&source)), mode, black_box(options))
                        .expect("the generic reader write must dispatch");
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("write_arrow_batch/{name}"), |bencher| {
            bencher.iter_batched(
                || {
                    if mode == IOMode::Overwrite {
                        handle("mode-batch.arrows")
                    } else {
                        stored_with("mode-batch.arrows", &source)
                    }
                },
                |mut target| {
                    target
                        .write_arrow_batch(black_box(source.clone()), mode, black_box(options))
                        .expect("the generic held-batch write must dispatch");
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("write_records/{name}"), |bencher| {
            bencher.iter_batched(
                || {
                    let target = if mode == IOMode::Overwrite {
                        handle("mode-records.arrows")
                    } else {
                        stored_with("mode-records.arrows", &source)
                    };
                    (target, rows.clone())
                },
                |(mut target, rows)| {
                    target
                        .write_records(black_box(rows), mode, black_box(options))
                        .expect("the generic native-row write must dispatch");
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn shape_benchmarks(criterion: &mut Criterion) {
    let cast_source = cast_source();
    let cast_field = cast_field();
    let (nested_field, nested_source) = nested_wide();
    let mut shape = criterion.benchmark_group("io_write_shape");
    shape.sample_size(10);

    for (label, name) in [
        ("ipc", "shape.arrows"),
        ("parquet", "shape.parquet"),
        ("avro", "shape.avro"),
    ] {
        shape.throughput(Throughput::Elements(SHAPE_ROWS as u64));
        shape.bench_function(format!("cast_select/{label}"), |bencher| {
            bencher.iter_batched(
                || {
                    let target = handle(name);
                    let options = target
                        .record_options()
                        .expect("an implemented encoding")
                        .with_field(cast_field.clone())
                        .with_select_by_names(["PRICE", "symbol"]);
                    (target, options)
                },
                |(mut target, options)| {
                    target
                        .overwrite_arrow_reader(reader(black_box(&cast_source)), &options)
                        .expect("the declared cast and selection must write");
                },
                BatchSize::LargeInput,
            );
        });
        shape.bench_function(format!("wide_nested/{label}"), |bencher| {
            bencher.iter_batched(
                || {
                    let target = handle(name);
                    let options = target
                        .record_options()
                        .expect("an implemented encoding")
                        .with_field(nested_field.clone());
                    (target, options)
                },
                |(mut target, options)| {
                    target
                        .overwrite_arrow_reader(reader(black_box(&nested_source)), &options)
                        .expect("the wide nested fixture must write");
                },
                BatchSize::LargeInput,
            );
        });
    }
    shape.finish();
}

fn streaming_benchmarks(criterion: &mut Criterion) {
    let source = batch().slice(0, STATEFUL_ROWS);
    let one_row = source.slice(0, 1);
    let bounded_options = handle("bounded.arrows")
        .record_options()
        .expect("an implemented encoding")
        .with_field(wide())
        .with_max_row_size(1);

    // Pin the benchmark claim before measuring it: the 1,024-batch tail must
    // not be pulled after the first row satisfies the bound.
    let proof = Arc::new(AtomicUsize::new(0));
    let mut proof_target = handle("bounded-proof.arrows");
    proof_target
        .overwrite_arrow_reader(
            counted_reader(&one_row, 1_024, Arc::clone(&proof)),
            &bounded_options,
        )
        .expect("the bounded fixture must write");
    assert_eq!(proof.load(Ordering::Relaxed), 1);

    let mut bounded = criterion.benchmark_group("io_write_streaming");
    bounded.throughput(Throughput::Elements(1));
    bounded.bench_function("overwrite/max_rows_1_of_1024_batches", |bencher| {
        bencher.iter_batched(
            || {
                let pulled = Arc::new(AtomicUsize::new(0));
                let batches = counted_reader(&one_row, 1_024, Arc::clone(&pulled));
                (handle("bounded.arrows"), batches, pulled)
            },
            |(mut target, batches, pulled)| {
                target
                    .overwrite_arrow_reader(batches, black_box(&bounded_options))
                    .expect("the bounded fixture must write");
                black_box(pulled.load(Ordering::Relaxed));
            },
            BatchSize::SmallInput,
        );
    });
    bounded.finish();
}

fn commit_benchmarks(criterion: &mut Criterion) {
    const COMMIT_ROWS: usize = 64;
    let source = batch().slice(0, COMMIT_ROWS);
    let mut commits = criterion.benchmark_group("io_write_commit_rows");
    commits.sample_size(10);
    commits.throughput(Throughput::Elements(COMMIT_ROWS as u64));

    // The edge cadences make the publication cost visible: unset and a bound
    // larger than the stream each publish once, while N=1 is the intentional
    // worst case against which a practical cadence can be chosen.
    for (label, cadence) in [
        ("unset", None),
        ("n_1", Some(1)),
        ("n_8", Some(8)),
        ("n_larger", Some(COMMIT_ROWS + 1)),
    ] {
        commits.bench_function(format!("overwrite/{label}"), |bencher| {
            bencher.iter_batched(
                || {
                    let target = handle("commit-overwrite.arrows");
                    let mut options = target
                        .record_options()
                        .expect("an implemented encoding")
                        .with_field(wide());
                    options.set_commit_row_size(cadence);
                    (target, options)
                },
                |(mut target, options)| {
                    target
                        .overwrite_arrow_reader(reader(black_box(&source)), &options)
                        .expect("the cadence fixture must overwrite");
                },
                BatchSize::LargeInput,
            );
        });
    }

    let committed = handle("commit-intents.arrows")
        .record_options()
        .expect("an implemented encoding")
        .with_field(wide())
        .with_commit_row_size(8);
    commits.bench_function("append/n_8", |bencher| {
        bencher.iter_batched(
            || stored_with("commit-append.arrows", &source),
            |mut target| {
                target
                    .append_arrow_reader(reader(black_box(&source)), &committed)
                    .expect("the cadence fixture must append");
            },
            BatchSize::LargeInput,
        );
    });
    let merging = committed.clone().with_merge_by_names(["id"]);
    commits.bench_function("merge/n_8", |bencher| {
        bencher.iter_batched(
            || stored_with("commit-merge.arrows", &source),
            |mut target| {
                target
                    .merge_arrow_reader(reader(black_box(&source)), &merging)
                    .expect("the cadence fixture must merge");
            },
            BatchSize::LargeInput,
        );
    });
    commits.finish();
}

pub(crate) fn write_surface_benchmarks(criterion: &mut Criterion) {
    stateful_benchmarks(criterion);
    empty_no_op_benchmarks(criterion);
    native_record_benchmarks(criterion);
    mode_dispatch_benchmarks(criterion);
    shape_benchmarks(criterion);
    streaming_benchmarks(criterion);
    commit_benchmarks(criterion);
}
