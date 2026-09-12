//! What the generic Arrow scalar family costs, shape by shape.
//!
//! Every group answers one question a caller actually pays for: pairing a
//! payload with the Field that types it, widening any shape to the reader
//! every record surface speaks, collapsing a stream, reshaping rows onto
//! another Field, and crossing into structured text. The last two carry a
//! **baseline** - the bare call the family wraps, and the native `Scalar` pair
//! over the same bytes - so the wrapper's own overhead is a number rather than
//! a claim.
//!
//! The corpus is a commodity tape: a symbol, a decimal price, an integer size,
//! and a microsecond instant, at two row counts.

#[path = "bench_profile.rs"]
mod bench_profile;

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, Decimal128Array, RecordBatch};
use arrow_schema::SchemaRef;
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use yggdryl::arrow::{BatchReader, batch_reader, cast_reader};
use yggdryl::holder::Buffer;
use yggdryl::{
    ArrowCast, ArrowCastOptions, ArrowValue, DataType, Field, IOBase, IOMedia, IOMode, Scalar,
    TimeUnit, Timezone, Url,
};

/// Rows per fixture: one small enough to stay warm, one at the size a
/// streamed read hands over in a single pull.
const ROWS: [usize; 2] = [
    bench_profile::corpus(1_024, 64),
    bench_profile::corpus(16_384, 256),
];

/// Rows per structured-text fixture.
///
/// JSON Lines frames one document per row and `Limits::default()` admits 1,024
/// documents, so this is the largest corpus a text round trip may carry; the
/// Arrow groups measure the wider one.
const DOCUMENT_ROWS: [usize; 2] = [
    bench_profile::corpus(128, 32),
    bench_profile::corpus(1_024, 128),
];

/// Batches one stream fixture is split into, so a stream is really a stream.
const BATCHES: usize = 8;

/// The symbols the tape cycles through.
const SYMBOLS: [&str; 4] = ["BRENT", "WTI", "TTF", "XAU"];

/// The first row's instant, microseconds since the Unix epoch.
const EPOCH: i64 = 1_767_225_600_000_000;

/// The trade root: what one commodity tick carries.
fn root() -> Field {
    DataType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::decimal128(12, 4)
            .expect("the price width is valid")
            .required_field("price"),
        DataType::Int64.required_field("size"),
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::UTC,
        }
        .required_field("timestamp"),
    ])
    .expect("the trade root is valid")
    .required_field("row")
}

/// The price column on its own, which is what a scalar and an array pair with.
fn price_field() -> Field {
    DataType::decimal128(12, 4)
        .expect("the price width is valid")
        .required_field("price")
}

/// `count` canonical trade rows, in the positional shape a batch reads back as.
fn rows(count: usize) -> Scalar {
    Scalar::from_sequence((0..count).map(|row| {
        let index = i64::try_from(row).expect("the row index fits an i64");
        Scalar::from_sequence([
            Scalar::from(SYMBOLS[row % SYMBOLS.len()]),
            Scalar::d128(i128::from(index % 20_000) * 25, 4),
            Scalar::from(index % 500 + 1),
            Scalar::datetime64(EPOCH + index * 1_000, TimeUnit::Microsecond, Timezone::UTC)
                .expect("microseconds under a named zone are an instant"),
        ])
    }))
}

/// One batch of `count` trade rows, built through the family's own boundary.
fn batch(root: &Field, count: usize) -> RecordBatch {
    ArrowValue::from_rows(root, &rows(count))
        .expect("the trade rows materialize")
        .into_batch()
        .expect("a held batch is already a batch")
}

/// The same rows as [`BATCHES`] batches, so a stream is more than one pull.
///
/// The parts are slices of the one fixture, so building a stream costs a
/// pointer per batch rather than a rebuild of the arrays.
fn parts(batch: &RecordBatch) -> Vec<RecordBatch> {
    let size = batch.num_rows().div_ceil(BATCHES);
    (0..batch.num_rows())
        .step_by(size)
        .map(|offset| batch.slice(offset, size.min(batch.num_rows() - offset)))
        .collect()
}

/// One price column of `count` values.
fn prices(count: usize) -> ArrayRef {
    Arc::new(
        (0..count)
            .map(|row| Some(i128::try_from(row % 20_000).unwrap_or_default() * 25))
            .collect::<Decimal128Array>()
            .with_precision_and_scale(12, 4)
            .expect("the price column matches its declared width"),
    )
}

/// One held batch as a value, without rebuilding its arrays.
fn held(root: &Field, batch: &RecordBatch) -> ArrowValue {
    ArrowValue::from_batch_as(root.clone(), batch.clone()).expect("the batch matches its root")
}

/// One stream over `parts`, which is one-shot and so is rebuilt per sample.
fn streamed(schema: &SchemaRef, parts: &[RecordBatch]) -> ArrowValue {
    ArrowValue::from_reader(batch_reader(Arc::clone(schema), parts.to_vec()))
        .expect("the stream names its root")
}

/// Pull one batch: the latency half of an iteration surface.
fn first_batch(value: ArrowValue) -> usize {
    value
        .into_reader()
        .expect("every shape widens to a reader")
        .next()
        .transpose()
        .expect("the first batch decodes")
        .map_or(0, |batch| batch.num_rows())
}

/// Pull every batch: the throughput half.
fn drain(value: ArrowValue) -> usize {
    drain_reader(value.into_reader().expect("every shape widens to a reader"))
}

/// Count the rows a reader yields, holding no batch past the count.
fn drain_reader(reader: BatchReader) -> usize {
    reader
        .map(|batch| batch.expect("a batch decodes").num_rows())
        .sum()
}

/// A handle whose media type comes from a name, so the format is declared.
fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("the benchmark name is a valid location")
            .media_type(),
    )
}

/// The rows a text document carries: canonical rows with their names put back.
///
/// This is exactly what a structured write puts on the wire, so the native
/// arm is measured over the same document rather than a similar one.
fn natural_rows(root: &Field, batch: &RecordBatch) -> Scalar {
    let rows = held(root, batch)
        .into_scalar()
        .expect("a batch reads back as rows");
    Scalar::from_sequence(
        rows.as_sequence()
            .expect("a batch reads back as a sequence")
            .iter()
            .map(|row| {
                root.into_natural_value(row.clone())
                    .expect("a canonical row names itself")
            })
            .collect::<Vec<_>>(),
    )
}

/// Construction per shape: what pairing a payload with its Field costs.
///
/// Four of the five only prove a pairing - a layout check, or a root read back
/// out of a schema a batch or a reader already reports - so their cost is flat
/// in the rows they accept. `from_rows` is the one that materializes columns,
/// and the only one that scales with them.
fn construction_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let price = price_field();
    let one = Scalar::d128(632_500, 4);

    let mut group = criterion.benchmark_group("arrow_value_construct");
    group.bench_function("from_value", |bencher| {
        bencher.iter(|| {
            ArrowValue::from_value(black_box(&price), black_box(&one))
                .expect("one price materializes")
        });
    });
    // Pairing an existing payload with its Field reads the layout and stops,
    // so it costs the same at either row count and reports no rate: elements
    // per second would describe elements it never touches. Criterion carries a
    // group's throughput forward once set, which is why the arm that does
    // scale with the row count is measured after these rather than among them.
    for count in ROWS {
        let column = prices(count);
        let batch = batch(&root, count);
        let schema = batch.schema();
        let parts = parts(&batch);

        // Each arm is handed its argument by a setup step outside the timer,
        // so none of them measures the clone that produced it.
        group.bench_function(format!("from_array/{count}"), |bencher| {
            bencher.iter_batched(
                || (price.clone(), Arc::clone(&column)),
                |(field, column)| {
                    ArrowValue::from_array(field, column).expect("the price column pairs")
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_function(format!("from_batch/{count}"), |bencher| {
            bencher.iter_batched(
                || batch.clone(),
                |batch| ArrowValue::from_batch(batch).expect("the batch names its root"),
                BatchSize::SmallInput,
            );
        });
        group.bench_function(format!("from_reader/{count}"), |bencher| {
            bencher.iter_batched(
                || batch_reader(Arc::clone(&schema), parts.clone()),
                |reader| ArrowValue::from_reader(reader).expect("the stream names its root"),
                BatchSize::SmallInput,
            );
        });
    }

    // Materializing every row is the one shape whose cost is the row count, so
    // it is the one that reports a rate.
    for count in ROWS {
        let native = rows(count);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("from_rows/{count}"), |bencher| {
            bencher.iter(|| {
                ArrowValue::from_rows(black_box(&root), black_box(&native))
                    .expect("the trade rows materialize")
            });
        });
    }
    group.finish();
}

/// The funnel: what widening each shape to a reader costs, twice over.
///
/// An iteration surface is two numbers, not one - time to the first batch and
/// time to drain it - and they differ per shape. A batch and a stream are
/// handed over as they stand, so their first batch is the whole cost; an array
/// has to be laid out as rows first, and that is paid before the first pull.
fn reader_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let price = price_field();

    let mut group = criterion.benchmark_group("arrow_value_into_reader");
    for count in ROWS {
        let column = prices(count);
        let batch = batch(&root, count);
        let schema = batch.schema();
        let parts = parts(&batch);

        let array_value = || {
            ArrowValue::from_array(price.clone(), Arc::clone(&column)).expect("the column pairs")
        };
        let batch_value = || held(&root, &batch);
        let stream_value = || streamed(&schema, &parts);
        let shapes: [(&str, &dyn Fn() -> ArrowValue); 3] = [
            ("array", &array_value),
            ("batch", &batch_value),
            ("stream", &stream_value),
        ];

        group.throughput(Throughput::Elements(count as u64));
        for (name, value) in shapes {
            group.bench_function(format!("first_batch/{name}/{count}"), |bencher| {
                bencher.iter_batched(value, first_batch, BatchSize::SmallInput);
            });
            group.bench_function(format!("drain/{name}/{count}"), |bencher| {
                bencher.iter_batched(value, drain, BatchSize::SmallInput);
            });
        }
    }
    group.finish();
}

/// Collapsing a stream: what it costs to stop streaming.
///
/// `into_batch` pulls every batch and concatenates them; `into_scalar` decodes
/// every row and joins the per-batch sequences. Both are measured against the
/// same call on a held batch - the same rows with no stream around them -
/// because that gap is exactly what the concatenation and the per-batch
/// framing cost.
///
/// Every arm here returns the whole result, and Criterion keeps a batch of
/// results alive to drop them outside the timed loop, so these are the arms
/// that size their batches by the *output*: `LargeInput` keeps the retained
/// set small enough that the allocator is not what is being measured.
fn collect_benchmarks(criterion: &mut Criterion) {
    let root = root();

    let mut group = criterion.benchmark_group("arrow_value_collect");
    for count in ROWS {
        let batch = batch(&root, count);
        let schema = batch.schema();
        let parts = parts(&batch);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("into_batch/stream/{count}"), |bencher| {
            bencher.iter_batched(
                || streamed(&schema, &parts),
                |value| value.into_batch().expect("the stream concatenates"),
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("into_batch/batch/{count}"), |bencher| {
            bencher.iter_batched(
                || held(&root, &batch),
                |value| value.into_batch().expect("a held batch is already a batch"),
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("into_scalar/stream/{count}"), |bencher| {
            bencher.iter_batched(
                || streamed(&schema, &parts),
                |value| value.into_scalar().expect("the streamed rows decode"),
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("into_scalar/batch/{count}"), |bencher| {
            bencher.iter_batched(
                || held(&root, &batch),
                |value| value.into_scalar().expect("the held rows decode"),
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

/// The root a cast reshapes trades onto: reordered, with the price restated at
/// a wider scale so the cast is a cast rather than an identity.
fn cast_target() -> Field {
    DataType::from_fields([
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::UTC,
        }
        .required_field("timestamp"),
        DataType::utf8().required_field("symbol"),
        DataType::decimal128(18, 6)
            .expect("the widened price width is valid")
            .required_field("price"),
        DataType::Int64.required_field("size"),
    ])
    .expect("the cast target is valid")
    .required_field("row")
}

/// Casting, against the bare call the family wraps.
///
/// A batch cast is `ArrowCast::cast_arrow_batch` plus one Field clone; a
/// stream cast is `arrow::cast_reader`, which compiles one plan for the whole
/// stream and applies it per batch. Each family arm sits next to the bare call
/// over the same rows, so the wrapper's own overhead is what separates them.
/// The stream arms drain, because a stream cast plans eagerly and converts
/// lazily - timing the call alone would measure the plan and nothing else.
///
/// Both members of a pair are batched the same way, so neither one is charged
/// for dropping the result the other one is not.
fn cast_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let target = cast_target();
    let options = ArrowCastOptions::new();

    let mut group = criterion.benchmark_group("arrow_value_cast");
    for count in ROWS {
        let batch = batch(&root, count);
        let schema = batch.schema();
        let parts = parts(&batch);

        // Both paths answer the same rows, which is what makes the pair a
        // comparison rather than two numbers.
        let streamed_rows = drain_reader(
            cast_reader(
                batch_reader(Arc::clone(&schema), parts.clone()),
                &target,
                options,
            )
            .expect("the stream is plannable"),
        );
        assert_eq!(
            streamed_rows,
            target
                .cast_arrow_batch(batch.clone(), options)
                .expect("the batch is castable")
                .num_rows(),
            "the two cast paths must answer the same rows"
        );

        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("batch/family/{count}"), |bencher| {
            bencher.iter_batched(
                || held(&root, &batch),
                |value| value.cast(&target, options).expect("the batch casts"),
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("batch/kernel/{count}"), |bencher| {
            bencher.iter_batched(
                || batch.clone(),
                |batch| {
                    target
                        .cast_arrow_batch(batch, options)
                        .expect("the batch casts")
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("stream/family/{count}"), |bencher| {
            bencher.iter_batched(
                || streamed(&schema, &parts),
                |value| {
                    drain_reader(
                        value
                            .cast(&target, options)
                            .expect("the stream is plannable")
                            .into_reader()
                            .expect("a cast stream is still a stream"),
                    )
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_function(format!("stream/kernel/{count}"), |bencher| {
            bencher.iter_batched(
                || batch_reader(Arc::clone(&schema), parts.clone()),
                |reader| {
                    drain_reader(
                        cast_reader(reader, &target, options).expect("the stream is plannable"),
                    )
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

/// Structured text both ways, against the native `Scalar` pair on the same rows.
///
/// The setup asserts that the two writes produce byte-identical documents, so
/// the gap is only what typing a document and materializing it as columns
/// costs - never a difference in what was encoded. JSON frames every row in
/// one document, so its write holds the rows it frames; JSON Lines frames one
/// row at a time, which is what lets its write stream.
fn structured_benchmarks(criterion: &mut Criterion) {
    let root = root();

    let mut group = criterion.benchmark_group("arrow_value_structured");
    group.sample_size(10);
    for count in DOCUMENT_ROWS {
        let batch = batch(&root, count);
        let natural = natural_rows(&root, &batch);
        for (format, name) in [("json", "trades.json"), ("jsonl", "trades.jsonl")] {
            let mut source = handle(name);
            source
                .write_arrow_value(held(&root, &batch), IOMode::Overwrite)
                .expect("the Arrow rows write");
            let bytes = source.read_all_bytes().expect("the document reads back");

            let mut native = handle(name);
            native
                .write_scalar(&natural)
                .expect("the native rows write");
            assert_eq!(
                bytes,
                native.read_all_bytes().expect("the document reads back"),
                "{format} must carry the same document either way"
            );

            group.throughput(Throughput::Bytes(bytes.len() as u64));
            group.bench_function(format!("write_arrow_value/{format}/{count}"), |bencher| {
                bencher.iter_batched(
                    || (handle(name), held(&root, &batch)),
                    |(mut target, value)| {
                        target
                            .write_arrow_value(value, IOMode::Overwrite)
                            .expect("the Arrow rows write");
                    },
                    BatchSize::SmallInput,
                );
            });
            group.bench_function(format!("write_scalar/{format}/{count}"), |bencher| {
                bencher.iter_batched(
                    || handle(name),
                    |mut target| {
                        target
                            .write_scalar(black_box(&natural))
                            .expect("the native rows write");
                    },
                    BatchSize::SmallInput,
                );
            });
            group.bench_function(format!("read_arrow_value/{format}/{count}"), |bencher| {
                bencher.iter(|| {
                    black_box(&source)
                        .read_arrow_value(Some(black_box(&root)))
                        .expect("the document reads as rows")
                });
            });
            group.bench_function(format!("read_scalar/{format}/{count}"), |bencher| {
                bencher.iter(|| {
                    black_box(&source)
                        .read_scalar(None)
                        .expect("the document reads as a value")
                });
            });
        }
    }
    group.finish();
}

criterion_group!(
    arrow_values,
    construction_benchmarks,
    reader_benchmarks,
    collect_benchmarks,
    cast_benchmarks,
    structured_benchmarks,
);
criterion_main!(arrow_values);
