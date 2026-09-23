//! What a column costs across Arrow, shape by shape.
//!
//! Every group answers one question a caller actually pays for: landing an
//! array, a batch or a stream as the `Serie` its Field types, handing any
//! shape back as the reader every record surface speaks, collapsing a stream,
//! reshaping rows onto another Field, and crossing into structured text. The
//! last two carry a **baseline** - the bare Arrow door beside the column in
//! hand, and the native `Scalar` pair over the same bytes - so the column's
//! own overhead is a number rather than a claim.
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
use yggdryl::arrow::{BatchReader, batch_reader};
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{
    ArrowCastOptions, DataType, Field, IOBase, IOMedia, IOMode, MediaType, MimeType, Scalar, Serie,
    SerieReader, StructType, TimeUnit, Timezone, Url,
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
    StructType::from_fields([
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
    .map(DataType::from)
    .expect("the trade root is valid")
    .required_field("row")
}

/// The price column on its own, which is what a scalar and an array pair with.
fn price_field() -> Field {
    DataType::decimal128(12, 4)
        .expect("the price width is valid")
        .required_field("price")
}

/// `count` canonical trade rows, in the positional shape a record column's
/// rows read back as.
fn rows(count: usize) -> Vec<Scalar> {
    (0..count)
        .map(|row| {
            let index = i64::try_from(row).expect("the row index fits an i64");
            Scalar::from_sequence([
                Scalar::from(SYMBOLS[row % SYMBOLS.len()]),
                Scalar::d128(i128::from(index % 20_000) * 25, 4),
                Scalar::from(index % 500 + 1),
                Scalar::datetime64(EPOCH + index * 1_000, TimeUnit::Microsecond, Timezone::UTC)
                    .expect("microseconds under a named zone are an instant"),
            ])
        })
        .collect()
}

/// One batch of `count` trade rows, laid out once as the root's column.
fn batch(root: &Field, count: usize) -> RecordBatch {
    Serie::from_scalars(root.clone(), rows(count))
        .expect("the trade rows materialize")
        .into_arrow_batch()
        .expect("a record column with every row present is a batch")
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

/// One held batch landed as its root's record column, sharing its arrays.
fn held(root: &Field, batch: &RecordBatch) -> Serie {
    Serie::from_arrow_batch(Some(root), batch, ArrowCastOptions::new())
        .expect("the batch matches its root")
}

/// One stream over `parts`, which is one-shot and so is rebuilt per sample.
fn streamed(schema: &SchemaRef, parts: &[RecordBatch]) -> BatchReader {
    batch_reader(Arc::clone(schema), parts.to_vec())
}

/// A column in hand as the stream of the one batch it is.
fn held_reader(serie: Serie) -> SerieReader {
    SerieReader::from_serie(serie).expect("a held column is one batch")
}

/// Pull one batch: the latency half of an iteration surface.
fn first_batch(reader: SerieReader) -> usize {
    reader
        .into_arrow_reader()
        .next()
        .transpose()
        .expect("the first batch decodes")
        .map_or(0, |batch| batch.num_rows())
}

/// Pull every batch: the throughput half.
fn drain(reader: SerieReader) -> usize {
    drain_reader(reader.into_arrow_reader())
}

/// Count the rows a reader yields, holding no batch past the count.
fn drain_reader(reader: BatchReader) -> usize {
    reader
        .map(|batch| batch.expect("a batch decodes").num_rows())
        .sum()
}

/// One held batch as the stream a write takes: its record column, yielded as
/// it stands.
fn written(root: &Field, batch: &RecordBatch) -> SerieReader {
    SerieReader::from_serie(held(root, batch)).expect("a record column with every row present")
}

/// The one section a structured document reads off record options: the
/// declared field, carried by any record encoding's options.
fn declaring(field: &Field) -> RecordOptions {
    RecordOptions::for_media_type(&MediaType::new(MimeType::ARROW_STREAM))
        .expect("the IPC encoding is built in")
        .with_field(field.clone())
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
    let rows = Scalar::from(held(root, batch));
    Scalar::from_sequence(
        rows.sequence_rows()
            .expect("a record column reads back as a sequence")
            .iter()
            .map(|row| {
                root.into_natural_value(row.clone())
                    .expect("a canonical row names itself")
            })
            .collect::<Vec<_>>(),
    )
}

/// Construction per shape: what landing a payload under its Field costs.
///
/// An array and a batch land through one compiled plan whose exact layout is
/// the identity, so their buffers are shared and what a landing reads is its
/// proof: the layout, absence on the validity words, and each row of a leaf
/// narrower than its storage - the decimal price here - once. A stream only
/// plans; its batches land as they are pulled, so its cost is flat in the rows
/// it will carry. `from_scalars` is the door that lays columns out, typing
/// every row on the way.
fn construction_benchmarks(criterion: &mut Criterion) {
    let root = Arc::new(root());
    let price = Arc::new(price_field());
    let one = Scalar::d128(632_500, 4);
    let options = ArrowCastOptions::new();

    let mut group = criterion.benchmark_group("arrow_serie_construct");
    group.bench_function("from_scalars/value", |bencher| {
        bencher.iter(|| {
            Serie::from_scalars(Arc::clone(&price), [black_box(&one).clone()])
                .expect("one price materializes")
        });
    });
    // Planning a stream reads its schema and stops, so it costs the same at
    // either row count and reports no rate: elements per second would describe
    // elements it never touches. Criterion carries a group's throughput forward
    // once set, which is why the arms that do scale with the row count are
    // measured after it rather than beside it.
    for count in ROWS {
        let batch = batch(&root, count);
        let schema = batch.schema();
        let parts = parts(&batch);

        // The arm is handed its stream by a setup step outside the timer, so
        // it does not measure the clone that produced it.
        group.bench_function(format!("from_arrow_reader/{count}"), |bencher| {
            bencher.iter_batched(
                || batch_reader(Arc::clone(&schema), parts.clone()),
                |reader| {
                    SerieReader::from_arrow_reader(None, reader, options)
                        .expect("the stream names its root")
                },
                BatchSize::SmallInput,
            );
        });
    }

    // Landing a held payload proves its rows, and laying rows out types each
    // one, so these are the arms whose cost is the row count and that report
    // a rate.
    for count in ROWS {
        let column = prices(count);
        let batch = batch(&root, count);
        let native = rows(count);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("from_arrow_array/{count}"), |bencher| {
            bencher.iter_batched(
                || Arc::clone(&column),
                |column| {
                    Serie::from_arrow_array(Some(price.as_ref()), column, options)
                        .expect("the price column lands")
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_function(format!("from_arrow_batch/{count}"), |bencher| {
            bencher.iter(|| {
                Serie::from_arrow_batch(Some(root.as_ref()), black_box(&batch), options)
                    .expect("the batch lands under its root")
            });
        });
        group.bench_function(format!("from_scalars/{count}"), |bencher| {
            bencher.iter(|| {
                Serie::from_scalars(Arc::clone(&root), black_box(&native).iter().cloned())
                    .expect("the trade rows materialize")
            });
        });
    }
    group.finish();
}

/// The funnel: what handing each shape back as a reader costs, twice over.
///
/// An iteration surface is two numbers, not one - time to the first batch and
/// time to drain it - and they differ per shape. A held column is the one
/// batch it is - a record column's children are the columns, and any other
/// column is the one column of a record - so its first batch is the whole
/// cost. A stream over an identity plan is handed back as the reader it was,
/// so its first pull is one batch of eight and its drain the rest.
fn reader_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let price = price_field();
    let options = ArrowCastOptions::new();

    let mut group = criterion.benchmark_group("arrow_serie_into_reader");
    for count in ROWS {
        let column = prices(count);
        let batch = batch(&root, count);
        let schema = batch.schema();
        let parts = parts(&batch);

        let array_value = || {
            held_reader(
                Serie::from_arrow_array(Some(&price), Arc::clone(&column), options)
                    .expect("the price column lands"),
            )
        };
        let batch_value = || held_reader(held(&root, &batch));
        let stream_value = || {
            SerieReader::from_arrow_reader(None, streamed(&schema, &parts), options)
                .expect("the stream names its root")
        };
        let shapes: [(&str, &dyn Fn() -> SerieReader); 3] = [
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
/// `Serie::from_arrow_reader` lands every batch through the reader's one plan
/// and joins them into one column; `into_batch` hands that column out as one
/// batch, and `into_scalar` holds it as one value, zero copy. Both are
/// measured against the same doors on a held batch - the same rows with no
/// stream around them - because that gap is exactly what the concatenation
/// and the per-batch framing cost.
///
/// Every arm here returns the whole result, and Criterion keeps a batch of
/// results alive to drop them outside the timed loop, so these are the arms
/// that size their batches by the *output*: `LargeInput` keeps the retained
/// set small enough that the allocator is not what is being measured.
fn collect_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let options = ArrowCastOptions::new();

    let mut group = criterion.benchmark_group("arrow_serie_collect");
    for count in ROWS {
        let batch = batch(&root, count);
        let schema = batch.schema();
        let parts = parts(&batch);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("into_batch/stream/{count}"), |bencher| {
            bencher.iter_batched(
                || streamed(&schema, &parts),
                |reader| {
                    Serie::from_arrow_reader(Some(&root), reader, options)
                        .and_then(|serie| serie.into_arrow_batch())
                        .expect("the stream concatenates")
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("into_batch/batch/{count}"), |bencher| {
            bencher.iter_batched(
                || batch.clone(),
                |batch| {
                    Serie::from_arrow_batch(Some(&root), &batch, options)
                        .and_then(|serie| serie.into_arrow_batch())
                        .expect("a held batch is already a batch")
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("into_scalar/stream/{count}"), |bencher| {
            bencher.iter_batched(
                || streamed(&schema, &parts),
                |reader| {
                    Serie::from_arrow_reader(Some(&root), reader, options)
                        .map(Scalar::from)
                        .expect("the streamed rows land")
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("into_scalar/batch/{count}"), |bencher| {
            bencher.iter_batched(
                || batch.clone(),
                |batch| {
                    Serie::from_arrow_batch(Some(&root), &batch, options)
                        .map(Scalar::from)
                        .expect("the held rows land")
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

/// The root a cast reshapes trades onto: reordered, with the price restated at
/// a wider scale so the cast is a cast rather than an identity.
fn cast_target() -> Field {
    StructType::from_fields([
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
    .map(DataType::from)
    .expect("the cast target is valid")
    .required_field("row")
}

/// Casting a column, against the bare Arrow door over the same rows.
///
/// A held cast is `Serie::cast` on the column a batch landed as, which
/// compiles one plan from its field and applies it once; its bare call is
/// `Serie::from_arrow_batch` into the target and back out through
/// `into_arrow_batch`. A stream cast is one `SerieReader` planned for the
/// whole stream: drained as the record columns it lands, and as its bare
/// transport face through `into_arrow_reader`, which reconciles each batch and
/// lands none. Each column arm sits next to its bare call over the same rows,
/// so the column's own overhead is what separates them. The stream arms
/// drain, because a stream cast plans eagerly and converts lazily - timing the
/// call alone would measure the plan and nothing else.
///
/// Both members of a pair are batched the same way, so neither one is charged
/// for dropping the result the other one is not.
fn cast_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let target = cast_target();
    let options = ArrowCastOptions::new();

    let mut group = criterion.benchmark_group("arrow_serie_cast");
    for count in ROWS {
        let batch = batch(&root, count);
        let schema = batch.schema();
        let parts = parts(&batch);

        // Every path answers the same rows, which is what makes each pair a
        // comparison rather than two numbers.
        let streamed_rows = drain_reader(
            SerieReader::from_arrow_reader(Some(&target), streamed(&schema, &parts), options)
                .expect("the stream is plannable")
                .into_arrow_reader(),
        );
        let landed_rows =
            SerieReader::from_arrow_reader(Some(&target), streamed(&schema, &parts), options)
                .expect("the stream is plannable")
                .map(|column| column.expect("a batch casts").len())
                .sum::<usize>();
        assert_eq!(
            streamed_rows,
            Serie::from_arrow_batch(Some(&target), &batch, options)
                .and_then(|serie| serie.into_arrow_batch())
                .expect("the batch is castable")
                .num_rows(),
            "the two cast paths must answer the same rows"
        );
        assert_eq!(
            landed_rows,
            held(&root, &batch)
                .cast(&target, options)
                .expect("the column casts")
                .len(),
            "the two column paths must answer the same rows"
        );
        assert_eq!(streamed_rows, landed_rows);

        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("batch/serie/{count}"), |bencher| {
            bencher.iter_batched(
                || held(&root, &batch),
                |value| value.cast(&target, options).expect("the column casts"),
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("batch/kernel/{count}"), |bencher| {
            bencher.iter_batched(
                || batch.clone(),
                |batch| {
                    Serie::from_arrow_batch(Some(&target), &batch, options)
                        .and_then(|serie| serie.into_arrow_batch())
                        .expect("the batch casts")
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("stream/serie/{count}"), |bencher| {
            bencher.iter_batched(
                || streamed(&schema, &parts),
                |reader| {
                    SerieReader::from_arrow_reader(Some(&target), reader, options)
                        .expect("the stream is plannable")
                        .map(|column| column.expect("a batch casts").len())
                        .sum::<usize>()
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_function(format!("stream/kernel/{count}"), |bencher| {
            bencher.iter_batched(
                || batch_reader(Arc::clone(&schema), parts.clone()),
                |reader| {
                    drain_reader(
                        SerieReader::from_arrow_reader(Some(&target), reader, options)
                            .expect("the stream is plannable")
                            .into_arrow_reader(),
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

    let mut group = criterion.benchmark_group("arrow_serie_structured");
    group.sample_size(10);
    for count in DOCUMENT_ROWS {
        let batch = batch(&root, count);
        let natural = natural_rows(&root, &batch);
        for (format, name) in [("json", "trades.json"), ("jsonl", "trades.jsonl")] {
            let mut source = handle(name);
            source
                .write_arrow(written(&root, &batch), IOMode::Overwrite, None)
                .expect("the Arrow rows write");
            let options = declaring(&root);
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
            group.bench_function(format!("write_arrow/{format}/{count}"), |bencher| {
                bencher.iter_batched(
                    || (handle(name), written(&root, &batch)),
                    |(mut target, value)| {
                        target
                            .write_arrow(value, IOMode::Overwrite, None)
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
            group.bench_function(format!("read_arrow/{format}/{count}"), |bencher| {
                bencher.iter(|| {
                    black_box(&source)
                        .read_arrow(Some(black_box(&options)))
                        .expect("the document reads as rows")
                        .map(|column| column.expect("the document is a record column").len())
                        .sum::<usize>()
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
