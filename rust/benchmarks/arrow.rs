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

use arrow_array::Array as _;
use std::hint::black_box;
use std::sync::Arc;
use yggdryl::SerieValue as _;

use arrow_array::{ArrayRef, Decimal128Array, RecordBatch};
use arrow_schema::SchemaRef;
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use yggdryl::SerieSource;
use yggdryl::arrow::{BatchReader, batch_reader};
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{
    ArrowCastOptions, ChunkedSerie, DataType, Field, IOBase, IOMedia, IOMode, MediaType, MimeType,
    Scalar, Serie, SerieReader, StructType, TimeUnit, Timezone, Url,
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
                Scalar::decimal128(i128::from(index % 20_000) * 25, 4),
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
    let one = Scalar::decimal128(632_500, 4);
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

/// Holding a stream as its chunks: what keeping the batches apart costs
/// against joining them.
///
/// `ChunkedSerie::from_arrow_reader` lands every batch through the reader's
/// one plan and keeps each as a chunk, where `Serie::from_arrow_reader` -
/// the `joined` arm - also concatenates them, so the gap between the two is
/// what the concatenation costs; `into_serie` pays it later, once, over
/// chunks already landed. A row read through the chunk ends is a binary
/// search and one row of one chunk, and a table's column is the child of
/// every batch, a pointer bump per chunk: those two are flat in the row
/// count, so they are measured before the group carries a rate.
fn chunked_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let options = ArrowCastOptions::new();
    let tables: Vec<(usize, SchemaRef, Vec<RecordBatch>, ChunkedSerie)> = ROWS
        .into_iter()
        .map(|count| {
            let batch = batch(&root, count);
            let schema = batch.schema();
            let parts = parts(&batch);
            let chunked =
                ChunkedSerie::from_arrow_reader(Some(&root), streamed(&schema, &parts), options)
                    .expect("the stream lands as its batches");
            assert_eq!(chunked.num_chunks(), BATCHES);
            (count, schema, parts, chunked)
        })
        .collect();

    let mut group = criterion.benchmark_group("arrow_chunked_serie");
    for (count, _, _, chunked) in &tables {
        let middle = count / 2;
        group.bench_function(format!("scalar/{count}"), |bencher| {
            bencher.iter(|| {
                black_box(chunked)
                    .scalar(black_box(middle))
                    .expect("the middle row")
            });
        });
        group.bench_function(format!("child/{count}"), |bencher| {
            bencher.iter(|| black_box(chunked).child("price").expect("a table column"));
        });
    }

    // Every arm below returns the whole result, so, as in the collect group,
    // batches are sized by the output.
    for (count, schema, parts, chunked) in &tables {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_function(format!("from_arrow_reader/{count}"), |bencher| {
            bencher.iter_batched(
                || streamed(schema, parts),
                |reader| {
                    ChunkedSerie::from_arrow_reader(Some(&root), reader, options)
                        .expect("the stream lands as its batches")
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("joined/{count}"), |bencher| {
            bencher.iter_batched(
                || streamed(schema, parts),
                |reader| {
                    Serie::from_arrow_reader(Some(&root), reader, options)
                        .expect("the stream concatenates")
                },
                BatchSize::LargeInput,
            );
        });
        group.bench_function(format!("into_serie/{count}"), |bencher| {
            bencher.iter(|| {
                black_box(chunked)
                    .into_serie()
                    .expect("the chunks concatenate")
            });
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

/// The SerieReader boundary with a wrapper-owned projection, against the same
/// options passed explicitly. Both arms read or publish the same selected rows.
fn record_options_benchmarks(criterion: &mut Criterion) {
    use yggdryl::ipc::{Ipc, IpcOptions};

    let root = root();
    let selected = IpcOptions::new().with_select("symbol, size").unwrap();
    let plain = RecordOptions::Ipc(IpcOptions::new());
    let mut group = criterion.benchmark_group("arrow_serie_record_options");
    for count in ROWS {
        let batch = batch(&root, count);
        let mut raw = handle("trades.arrows");
        raw.write_serie(written(&root, &batch).into(), IOMode::Overwrite, None)
            .unwrap();
        let source = Ipc::new(raw).with_options(selected.clone());
        let options = source.record_options().unwrap();
        let mut documents = Vec::new();
        group.throughput(Throughput::Elements(count as u64));
        for (name, options) in [("owned", None), ("explicit", Some(&options))] {
            let read = source.read_serie(options).unwrap();
            assert_eq!(read.field().field_len(), 2);
            assert_eq!(read.field().fields()[0].name(), "symbol");
            assert_eq!(read.field().fields()[1].name(), "size");
            assert_eq!(drain(read), count);
            let mut target = Ipc::new(handle("trades.arrows")).with_options(selected.clone());
            target
                .write_serie(written(&root, &batch).into(), IOMode::Overwrite, options)
                .unwrap();
            documents.push(target.read_all_bytes().unwrap());
            let stored = target.read_serie(Some(&plain)).unwrap();
            assert_eq!(stored.field().field_len(), 2);
            assert_eq!(drain(stored), count);

            group.bench_function(format!("read_serie/{name}/{count}"), |bencher| {
                bencher.iter(|| drain(black_box(&source).read_serie(black_box(options)).unwrap()));
            });
            group.bench_function(format!("write_serie/{name}/{count}"), |bencher| {
                bencher.iter_batched(
                    || {
                        (
                            Ipc::new(handle("trades.arrows")).with_options(selected.clone()),
                            written(&root, &batch),
                        )
                    },
                    |(mut target, rows)| {
                        target
                            .write_serie(rows.into(), IOMode::Overwrite, black_box(options))
                            .unwrap();
                        target
                    },
                    BatchSize::SmallInput,
                );
            });
        }
        assert_eq!(
            documents[0], documents[1],
            "the selected streams are identical"
        );
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
                .write_serie(
                    SerieSource::from(written(&root, &batch)),
                    IOMode::Overwrite,
                    None,
                )
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
            group.bench_function(format!("write_serie/{format}/{count}"), |bencher| {
                bencher.iter_batched(
                    || (handle(name), written(&root, &batch)),
                    |(mut target, value)| {
                        target
                            .write_serie(SerieSource::from(value), IOMode::Overwrite, None)
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
            group.bench_function(format!("read_serie/{format}/{count}"), |bencher| {
                bencher.iter(|| {
                    black_box(&source)
                        .read_serie(Some(black_box(&options)))
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

/// One physical item per offset-list row, with every other row absent.
fn alternating_list(item: Field, values: ArrayRef) -> (Field, ArrayRef) {
    let count = values.len();
    let offsets = (0..=count)
        .map(|offset| i32::try_from(offset).expect("the benchmark fits i32 offsets"))
        .collect::<Vec<_>>();
    let present = (0..count).map(|row| row % 2 == 0).collect::<Vec<_>>();
    let arrow_item = item
        .clone()
        .into_arrow_field_ref()
        .expect("the item projects");
    let array: ArrayRef = Arc::new(arrow_array::ListArray::new(
        arrow_item,
        arrow_buffer::OffsetBuffer::new(offsets.into()),
        values,
        Some(arrow_buffer::NullBuffer::from(present)),
    ));
    (Field::new("spans", DataType::serie(item), true), array)
}

/// One run per logical row, alternating valid and deliberately invalid ISIN
/// values, with the projected extension metadata Arrow's array needs.
fn isin_runs(count: usize) -> (Field, ArrayRef) {
    let run_field = Field::new(
        "encoded",
        DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Isin.required_field("values"),
        )
        .expect("int32 is a run-end type"),
        false,
    );
    let run_ends = arrow_array::Int32Array::from(
        (1..=count)
            .map(|end| i32::try_from(end).expect("the benchmark fits i32 run ends"))
            .collect::<Vec<_>>(),
    );
    let values = arrow_array::StringArray::from(
        (0..count)
            .map(|row| if row % 2 == 0 { "US0378331005" } else { "BAD" })
            .collect::<Vec<_>>(),
    );
    let plain = arrow_array::RunArray::<arrow_array::types::Int32Type>::try_new(&run_ends, &values)
        .expect("one run per row");
    let projected = run_field
        .clone()
        .into_arrow_field_ref()
        .expect("the run-end field projects");
    let encoded = arrow_array::make_array(
        plain
            .to_data()
            .into_builder()
            .data_type(projected.data_type().clone())
            .build()
            .expect("the ISIN extension is legal Arrow metadata"),
    );
    (run_field, encoded)
}

/// A nullable record over the alternating ISIN runs. This keeps the run array
/// in place and masks its narrow physical values.
fn alternating_masked_isin_runs(count: usize) -> (Field, ArrayRef) {
    let (run_field, encoded) = isin_runs(count);
    let projected = run_field
        .clone()
        .into_arrow_field_ref()
        .expect("the run-end field projects");
    let records: ArrayRef = Arc::new(arrow_array::StructArray::new(
        vec![projected].into(),
        vec![encoded],
        Some(arrow_buffer::NullBuffer::from(
            (0..count).map(|row| row % 2 == 0).collect::<Vec<_>>(),
        )),
    ));
    let root = StructType::from_fields([run_field])
        .map(DataType::from)
        .expect("one run-end child")
        .nullable_field("wrapped");
    (root, records)
}

/// Alternating null list rows over one physical run per item. Compacting the
/// required items gathers many disjoint ranges from a run-end array.
fn alternating_isin_run_list(count: usize) -> (Field, ArrayRef) {
    let (run_field, runs) = isin_runs(count);
    alternating_list(run_field.with_name("item"), runs)
}

/// Alternating null list rows over records containing run-end ISIN values.
/// This exercises the nested-run gather route without a zero-width fixed list.
fn alternating_struct_isin_run_list(count: usize) -> (Field, ArrayRef) {
    let (run_field, runs) = isin_runs(count);
    let item = Field::new(
        "item",
        DataType::from(
            StructType::from_fields([run_field, DataType::Int64.required_field("id")])
                .expect("two record children"),
        ),
        false,
    );
    let projected = item
        .clone()
        .into_arrow_field_ref()
        .expect("the record item projects");
    let arrow_schema::DataType::Struct(fields) = projected.data_type() else {
        panic!("the benchmark item is no longer a record");
    };
    let ids = arrow_array::Int64Array::from(
        (0..count)
            .map(|row| i64::try_from(row).expect("the benchmark fits i64"))
            .collect::<Vec<_>>(),
    );
    let records: ArrayRef = Arc::new(arrow_array::StructArray::new(
        fields.clone(),
        vec![runs, Arc::new(ids)],
        None,
    ));
    alternating_list(item, records)
}

/// Visibility repair has five distinct costs: a layout-contract child stays
/// borrowed, a required narrow list item compacts away hidden spans, root and
/// nested run-end arrays gather many disjoint ranges, and a narrow run value
/// is rebuilt with physical null placeholders under a struct mask.
fn null_visibility_benchmarks(criterion: &mut Criterion) {
    let options = ArrowCastOptions::new();
    let mut group = criterion.benchmark_group("arrow_serie_null_visibility");
    group.sample_size(10);

    for count in ROWS {
        let borrowed = alternating_list(
            DataType::Int64.required_field("item"),
            Arc::new(arrow_array::Int64Array::from(
                (0..count)
                    .map(|value| i64::try_from(value).expect("the benchmark fits i64"))
                    .collect::<Vec<_>>(),
            )),
        );
        let compacted = alternating_list(
            DataType::Isin.required_field("item"),
            Arc::new(arrow_array::StringArray::from(
                (0..count)
                    .map(|row| if row % 2 == 0 { "US0378331005" } else { "BAD" })
                    .collect::<Vec<_>>(),
            )),
        );
        let list_runs = alternating_isin_run_list(count);
        let nested_runs = alternating_struct_isin_run_list(count);
        let masked_runs = alternating_masked_isin_runs(count);

        let borrowed_landed =
            Serie::from_arrow_array(Some(&borrowed.0), Arc::clone(&borrowed.1), options)
                .expect("layout-contract items stay borrowed");
        assert_eq!(borrowed_landed.items().map(Serie::len), Some(count));
        assert!(
            borrowed_landed
                .into_arrow_array()
                .unwrap()
                .to_data()
                .ptr_eq(&borrowed.1.to_data()),
            "the layout-only control must retain every input buffer"
        );

        let compacted_landed =
            Serie::from_arrow_array(Some(&compacted.0), Arc::clone(&compacted.1), options)
                .expect("hidden required ISIN items compact away");
        assert_eq!(
            compacted_landed.items().map(Serie::len),
            Some(count.div_ceil(2))
        );
        assert_eq!(compacted_landed.scalar(1).unwrap(), Scalar::Null);

        let list_runs_landed =
            Serie::from_arrow_array(Some(&list_runs.0), Arc::clone(&list_runs.1), options)
                .expect("hidden required run-end items compact away");
        let gathered = list_runs_landed
            .items()
            .and_then(Serie::as_run_end_encoded)
            .expect("the gathered run-end items");
        assert_eq!(gathered.logical_len(), count.div_ceil(2));
        assert_eq!(gathered.values().len(), count.div_ceil(2));
        assert_eq!(list_runs_landed.scalar(1).unwrap(), Scalar::Null);

        let nested_runs_landed =
            Serie::from_arrow_array(Some(&nested_runs.0), Arc::clone(&nested_runs.1), options)
                .expect("records with nested ISIN runs compact away");
        let nested_records = nested_runs_landed.items().expect("the gathered records");
        let nested_encoded = nested_records
            .child("encoded")
            .and_then(Serie::as_run_end_encoded)
            .expect("the nested run-end child");
        assert_eq!(nested_encoded.logical_len(), count.div_ceil(2));
        assert_eq!(nested_encoded.values().len(), count.div_ceil(2));
        assert_eq!(nested_runs_landed.scalar(1).unwrap(), Scalar::Null);

        let run_landed =
            Serie::from_arrow_array(Some(&masked_runs.0), Arc::clone(&masked_runs.1), options)
                .expect("hidden ISIN run values become null placeholders");
        let encoded = run_landed
            .child("encoded")
            .and_then(Serie::as_run_end_encoded)
            .expect("the run-end child");
        assert_eq!(encoded.values().len(), count);
        assert_eq!(encoded.values().null_count(), count / 2);
        assert_eq!(encoded.scalar(1).unwrap(), Scalar::Null);

        group.throughput(Throughput::Elements(count as u64));
        for (name, (field, array)) in [
            ("borrowed_list_i64", borrowed),
            ("compact_list_isin", compacted),
            ("compact_list_run_end_isin", list_runs),
            ("compact_list_struct_run_end_isin", nested_runs),
            ("masked_run_end_isin", masked_runs),
        ] {
            group.bench_function(format!("{name}/{count}"), |bencher| {
                bencher.iter_batched(
                    || Arc::clone(&array),
                    |array| {
                        Serie::from_arrow_array(Some(&field), array, options)
                            .expect("the visibility fixture lands")
                    },
                    BatchSize::SmallInput,
                );
            });
        }
    }
    group.finish();
}

/// What the one memory estimate every byte bound reads costs to take.
///
/// A commit cadence, a write limit and the Iceberg file rolling measure each
/// batch they hold, so the estimate sits on the write path once per batch:
/// over the whole batch, and over the [`BATCHES`] zero-copy slices a stream
/// of it is cut into, which count the batch once between them. The estimate
/// reads offsets and lengths, never a value, so its cost follows the
/// columns rather than the rows.
fn memory_size_benchmarks(criterion: &mut Criterion) {
    let root = root();
    let mut group = criterion.benchmark_group("arrow_memory_size");
    for count in ROWS {
        let batch = batch(&root, count);
        let parts = parts(&batch);
        // Every slice but the first adds only the one offset that opens its
        // symbol column; no slice is charged its parent's buffers.
        assert_eq!(
            parts.iter().map(yggdryl::arrow::memory_size).sum::<usize>(),
            yggdryl::arrow::memory_size(&batch) + (parts.len() - 1) * 4,
            "the slices count the batch once between them"
        );
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("whole/{count}"), |bencher| {
            bencher.iter(|| yggdryl::arrow::memory_size(black_box(&batch)));
        });
        group.bench_function(format!("sliced/{count}"), |bencher| {
            bencher.iter(|| {
                black_box(&parts)
                    .iter()
                    .map(yggdryl::arrow::memory_size)
                    .sum::<usize>()
            });
        });
    }
    group.finish();
}

/// The record a windowed stream carries: a symbol in key order, the venue
/// it trades on as a registered code, the order it fills - its symbol the
/// tick's - a size and a second-spaced instant.
fn window_root() -> Field {
    StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Mic.required_field("venue"),
        DataType::from(
            StructType::from_fields([
                DataType::utf8().required_field("symbol"),
                DataType::Int64.required_field("id"),
            ])
            .expect("the order record is valid"),
        )
        .nullable_field("order"),
        DataType::Int64.required_field("size"),
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::UTC,
        }
        .required_field("timestamp"),
    ])
    .map(DataType::from)
    .expect("the window root is valid")
    .required_field("row")
}

/// `count` rows under [`window_root`] as one batch: the symbol over a quarter
/// of the rows each, in key order, the venue over an eighth each, one
/// second between instants, so a fifteen-minute bucket holds 900 rows.
fn window_batch(count: usize) -> RecordBatch {
    use arrow_array::{Int64Array, StringArray, StructArray, TimestampMicrosecondArray};
    use arrow_schema::DataType as ArrowDataType;

    const SORTED: [&str; 4] = ["BRENT", "TTF", "WTI", "XAU"];
    const VENUES: [&str; 2] = ["XLON", "XNYS"];
    let root = window_root();
    let schema = root.into_arrow_schema().expect("the window root projects");
    let symbols: ArrayRef = Arc::new(StringArray::from(
        (0..count)
            .map(|row| SORTED[row * SORTED.len() / count])
            .collect::<Vec<_>>(),
    ));
    let ids: ArrayRef = Arc::new(Int64Array::from(
        (0..count)
            .map(|row| i64::try_from(row).expect("the row index fits an i64"))
            .collect::<Vec<_>>(),
    ));
    let ArrowDataType::Struct(order) = schema.field(2).data_type().clone() else {
        panic!("an order record projects to a struct")
    };
    let orders: ArrayRef = Arc::new(
        StructArray::try_new(order, vec![Arc::clone(&symbols), Arc::clone(&ids)], None)
            .expect("the order record matches its fields"),
    );
    let venues: ArrayRef = Arc::new(StringArray::from(
        (0..count)
            .map(|row| VENUES[row * 8 / count % VENUES.len()])
            .collect::<Vec<_>>(),
    ));
    let instants: ArrayRef = Arc::new(
        TimestampMicrosecondArray::from(
            (0..count)
                .map(|row| {
                    EPOCH + i64::try_from(row).expect("the row index fits an i64") * 1_000_000
                })
                .collect::<Vec<_>>(),
        )
        .with_data_type(schema.field(4).data_type().clone()),
    );
    RecordBatch::try_new(schema, vec![symbols, venues, orders, ids, instants])
        .expect("the window batch matches its root")
}

/// Pull every window and every piece of it, each window read before the
/// next is taken: the rows served.
fn drain_windows(windows: yggdryl::SerieReaderWindows) -> usize {
    windows
        .map(|window| {
            window
                .expect("a window opens")
                .map(|piece| piece.expect("a piece decodes").len())
                .sum::<usize>()
        })
        .sum()
}

/// Windows of a stream: the walk over [`BATCHES`] batches, each cut where its
/// key changes, against the stream drained alone as the baseline.
///
/// Each window is a lazy reader over the stream, holding one batch; the time
/// to the first window is the bind, the first batch's landing and its cut,
/// and the drain adds a key and a cut per batch and a slice per piece a
/// window opens or closes inside a batch. A column, a record path and a code
/// key cut the landed cells where they lie; a period term evaluates over a
/// batch of the one column it reads, per row through the row tier; `sorted`
/// reads the order verdict the cut already holds.
fn window_benchmarks(criterion: &mut Criterion) {
    let root = window_root();
    let options = ArrowCastOptions::new();
    let keys: [(&str, yggdryl::Selector, bool); 5] = [
        ("column_key", "symbol".parse().expect("a column key"), false),
        (
            "path_key",
            "order.symbol".parse().expect("a path key"),
            false,
        ),
        ("code_key", "venue".parse().expect("a code key"), false),
        (
            "period_key",
            "minutes(timestamp, 15)".parse().expect("a period key"),
            false,
        ),
        ("sorted", "symbol".parse().expect("a column key"), true),
    ];

    let mut group = criterion.benchmark_group("serie_reader");
    for count in ROWS {
        let batch = window_batch(count);
        let schema = batch.schema();
        let parts = parts(&batch);
        let stream = || {
            SerieReader::from_arrow_reader(Some(&root), streamed(&schema, &parts), options)
                .expect("the stream lands under its root")
        };
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("drain/{count}"), |bencher| {
            bencher.iter_batched(
                stream,
                |reader| {
                    reader
                        .map(|batch| batch.expect("a batch lands").len())
                        .sum::<usize>()
                },
                BatchSize::SmallInput,
            );
        });
        for (name, key, sorted) in &keys {
            group.bench_function(format!("window_by/{name}/first/{count}"), |bencher| {
                bencher.iter_batched(
                    stream,
                    |reader| {
                        reader
                            .window_by(key, *sorted)
                            .expect("the key binds")
                            .next()
                            .expect("a window")
                            .expect("a window opens")
                            .next()
                            .map_or(0, |piece| piece.expect("a piece decodes").len())
                    },
                    BatchSize::SmallInput,
                );
            });
            group.bench_function(format!("window_by/{name}/drain/{count}"), |bencher| {
                bencher.iter_batched(
                    stream,
                    |reader| drain_windows(reader.window_by(key, *sorted).expect("the key binds")),
                    BatchSize::SmallInput,
                );
            });
        }
    }
    group.finish();
}

/// The record either side of a join carries: an instrument id, its code as
/// windows-1252 text, its symbol, and a count - a trade's size, an
/// instrument's lot.
fn join_root(name: &str, count: &str) -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::cp1252().required_field("code"),
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field(count),
    ])
    .map(DataType::from)
    .expect("the join root is valid")
    .required_field(name)
}

/// One row per id under `root`: the code and symbol the id names, the row
/// index as the count.
///
/// The code opens with one of four windows-1252 characters whose bytes
/// order otherwise than the characters they decode to - `€` is byte `0x80`
/// and `U+20AC`, `ÿ` byte `0xFF` and `U+00FF` - so the stored order of a
/// code column is not its value order, and a join on it hashes the values.
fn join_side(root: &Field, ids: impl IntoIterator<Item = i64>) -> Serie {
    const LEADS: [&str; 4] = ["\u{20ac}", "\u{ff}", "\u{160}", "A"];
    let rows = ids.into_iter().enumerate().map(|(index, id)| {
        let at = usize::try_from(id).expect("an id is not negative");
        Scalar::from_sequence([
            Scalar::from(id),
            Scalar::from(format!("{}{id:06}", LEADS[at % LEADS.len()])),
            Scalar::from(SYMBOLS[at % SYMBOLS.len()]),
            Scalar::from(i64::try_from(index).expect("the row index fits an i64")),
        ])
    });
    Serie::from_scalars(root.clone(), rows).expect("the join rows lay out")
}

/// `serie` as [`BATCHES`] chunks of equal rows, sharing its buffers.
fn join_chunks(serie: &Serie) -> ChunkedSerie {
    let size = serie.len().div_ceil(BATCHES);
    ChunkedSerie::from_series(
        None,
        (0..serie.len()).step_by(size).map(|offset| {
            serie
                .slice(offset, size.min(serie.len() - offset))
                .expect("a chunk")
        }),
        ArrowCastOptions::new(),
    )
    .expect("the chunks share one field")
}

/// Hash joins: `count` trades against the instruments they name, the
/// instruments - a quarter as many, every eighth id missing - pinned as the
/// build side.
///
/// `inner` and `left` key one int64 column on Arrow's row format; `two_keys`
/// adds the symbol to the row; `values_rung` keys the windows-1252 code,
/// whose stored order is not its value order, so every key row is built as
/// a value and hashed through its own equality. `streamed` probes with the
/// trades as a stream of [`BATCHES`] batches, one output batch per probe
/// batch, nothing of the probe collected. `prune_on` and `prune_off` join a
/// probe whose ids rise row by row against instruments covering the first
/// batch's alone, so seven batches of eight fall outside the build keys'
/// range: pruned, each stands alone as a slice of its own rows; hashed,
/// each row is probed, misses and is gathered. `grace` is `inner` under a
/// one-byte spill bound: the build and the probe scattered over partitions
/// written to disk, joined partition by partition, every output batch
/// spilled too.
fn join_benchmarks(criterion: &mut Criterion) {
    use yggdryl::expression::{IntoJoinKeys as _, JoinKeys};
    use yggdryl::{JoinKind, JoinOptions, JoinSide, SpillOptions};

    let trade_root = join_root("trade", "size");
    let instrument_root = join_root("instrument", "lot");
    let built = JoinOptions::new().with_build(Some(JoinSide::Right));
    let grace = built
        .clone()
        .with_spill(SpillOptions::new().with_byte_size(1));
    let by_id: JoinKeys = "id".parse().expect("one key");
    let by_id_and_symbol: JoinKeys = ["id", "symbol"].into_join_keys().expect("two keys");
    let by_code: JoinKeys = "code".parse().expect("one key");

    let mut group = criterion.benchmark_group("join");
    for count in ROWS {
        let rows = i64::try_from(count).expect("the row count fits an i64");
        let keys = (rows / 4).max(16);
        let trades = join_side(&trade_root, (0..rows).map(|row| row % keys));
        let instruments = join_side(&instrument_root, (0..keys).filter(|id| id % 8 != 7));
        let trade_chunks = join_chunks(&trades);
        let rising = join_chunks(&join_side(&trade_root, 0..rows));
        let first_batch = join_side(
            &instrument_root,
            0..i64::try_from(rising.chunks()[0].len()).expect("a batch's rows fit an i64"),
        );
        let first_batch = ChunkedSerie::from_serie(first_batch).expect("one chunk");
        // Every trade whose instrument is listed matches once; the keyed
        // shapes and the partitioned join answer those rows alike, the last
        // from disk.
        let matched = (0..rows).filter(|row| row % keys % 8 != 7).count();
        for (by, options) in [
            (&by_id, &built),
            (&by_id_and_symbol, &built),
            (&by_code, &built),
            (&by_id, &grace),
        ] {
            let joined = trades
                .join_with(&instruments, by, JoinKind::Inner, options)
                .expect("the join answers");
            assert_eq!(joined.len(), matched, "{by} under {options:?}");
            assert_eq!(joined.is_spilled(), options.spill().is_some(), "{by}");
        }
        let pruned = rising
            .join_with(&first_batch, &by_id, JoinKind::Left, &built)
            .expect("the join answers");
        assert_eq!(pruned.len(), count, "a left join keeps every trade");
        assert_eq!(
            pruned.num_chunks(),
            BATCHES,
            "one output batch per probe batch"
        );

        group.throughput(Throughput::Elements(count as u64));
        for (name, by, how) in [
            ("inner", &by_id, JoinKind::Inner),
            ("left", &by_id, JoinKind::Left),
            ("two_keys", &by_id_and_symbol, JoinKind::Inner),
            ("values_rung", &by_code, JoinKind::Inner),
        ] {
            group.bench_function(format!("{name}/{count}"), |bencher| {
                bencher.iter(|| {
                    black_box(&trades)
                        .join_with(black_box(&instruments), by, how, &built)
                        .expect("the join answers")
                });
            });
        }
        group.bench_function(format!("streamed/{count}"), |bencher| {
            bencher.iter_batched(
                || {
                    (
                        SerieReader::from_chunked(trade_chunks.clone()).expect("the chunks stream"),
                        instruments.clone(),
                    )
                },
                |(probe, build)| {
                    let joined = probe
                        .join_with(build, &by_id, JoinKind::Inner, &built)
                        .expect("the join resolves");
                    let mut rows = 0;
                    for batch in joined {
                        rows += batch.expect("an output batch").len();
                    }
                    rows
                },
                BatchSize::SmallInput,
            );
        });
        for (name, prune) in [("prune_on", true), ("prune_off", false)] {
            let options = built.clone().with_prune(prune);
            group.bench_function(format!("{name}/{count}"), |bencher| {
                bencher.iter(|| {
                    black_box(&rising)
                        .join_with(black_box(&first_batch), &by_id, JoinKind::Left, &options)
                        .expect("the join answers")
                });
            });
        }
        group.bench_function(format!("grace/{count}"), |bencher| {
            bencher.iter(|| {
                black_box(&trades)
                    .join_with(black_box(&instruments), &by_id, JoinKind::Inner, &grace)
                    .expect("the partitioned join answers")
            });
        });
    }
    group.finish();
}

/// A nested column and its JSON text, both ways: the tape as one struct
/// column and a basket of four sizes per row, written as text and read back.
///
/// The **baseline** beside the write is Arrow's own list-to-text kernel over
/// the same basket, which renders a display form rather than JSON; beside
/// each read it is serde_json parsing the same cells into its own value
/// tree, with no column built.
fn json_cast_benchmarks(criterion: &mut Criterion) {
    let strict = ArrowCastOptions::new().with_safe(false);
    let tape: DataType = "struct<symbol: utf8, price: decimal128(12, 4), size: int64>"
        .parse()
        .expect("the tape struct is valid");
    let tape = Field::new("tick", tape, false);
    let basket = Field::new(
        "basket",
        DataType::serie(DataType::Int64.nullable_field("item")),
        false,
    );
    let marks = Field::new(
        "marks",
        DataType::map_of(DataType::utf8(), DataType::Int64, false).expect("a map of marks"),
        false,
    );
    let text = Field::new("json", DataType::utf8(), false);
    let mut group = criterion.benchmark_group("arrow_serie_json");
    for count in ROWS {
        let ticks = Serie::from_scalars(
            tape.clone(),
            (0..count).map(|row| {
                Scalar::from_sequence([
                    Scalar::from(SYMBOLS[row % SYMBOLS.len()]),
                    Scalar::decimal128(i128::try_from(row).expect("a small row") * 125, 4),
                    Scalar::from(i64::try_from(row).expect("a small row")),
                ])
            }),
        )
        .expect("the tape column");
        let baskets = Serie::from_scalars(
            basket.clone(),
            (0..count).map(|row| {
                let size = i64::try_from(row).expect("a small row");
                Scalar::from_sequence((0..4).map(|lot| Scalar::from(size + lot)))
            }),
        )
        .expect("the basket column");
        // A plain map reads JSON back in its keys' text order, so the marks
        // are held in that order and read back as written.
        let mut symbols = SYMBOLS;
        symbols.sort_unstable();
        let book =
            Serie::from_scalars(
                marks.clone(),
                (0..count).map(|row| {
                    let mark = i64::try_from(row).expect("a small row");
                    Scalar::from_mapping(symbols.iter().zip(0..).map(|(symbol, offset)| {
                        (Scalar::from(*symbol), Scalar::from(mark + offset))
                    }))
                    .expect("distinct symbols")
                }),
            )
            .expect("the marks column");
        let tick_text = ticks.cast(&text, strict).expect("the tape spells JSON");
        let basket_text = baskets.cast(&text, strict).expect("the baskets spell JSON");
        let book_text = book.cast(&text, strict).expect("the marks spell JSON");
        assert_eq!(
            tick_text.cast(&tape, strict).expect("the JSON reads back"),
            ticks,
            "the tape reads back as written"
        );
        assert_eq!(
            basket_text
                .cast(&basket, strict)
                .expect("the JSON reads back"),
            baskets,
            "the baskets read back as written"
        );
        assert_eq!(
            book_text.cast(&marks, strict).expect("the JSON reads back"),
            book,
            "the marks read back as written"
        );
        let basket_array = baskets.require_arrow_array().expect("an Arrow list");
        let documents = |text: &Serie| {
            text.require_arrow_array()
                .expect("an Arrow text column")
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .expect("utf8 cells")
                .clone()
        };
        let (tick_cells, basket_cells, book_cells) = (
            documents(&tick_text),
            documents(&basket_text),
            documents(&book_text),
        );
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(format!("write/struct/{count}"), |bencher| {
            bencher.iter(|| black_box(&ticks).cast(&text, strict).expect("JSON"));
        });
        group.bench_function(format!("write/serie/{count}"), |bencher| {
            bencher.iter(|| black_box(&baskets).cast(&text, strict).expect("JSON"));
        });
        group.bench_function(format!("write/map/{count}"), |bencher| {
            bencher.iter(|| black_box(&book).cast(&text, strict).expect("JSON"));
        });
        group.bench_function(format!("write/kernel/{count}"), |bencher| {
            bencher.iter(|| {
                arrow_cast::cast(black_box(&basket_array), &arrow_schema::DataType::Utf8)
                    .expect("Arrow's display text")
            });
        });
        group.bench_function(format!("read/struct/{count}"), |bencher| {
            bencher.iter(|| black_box(&tick_text).cast(&tape, strict).expect("a tape"));
        });
        group.bench_function(format!("read/serie/{count}"), |bencher| {
            bencher.iter(|| {
                black_box(&basket_text)
                    .cast(&basket, strict)
                    .expect("baskets")
            });
        });
        group.bench_function(format!("read/map/{count}"), |bencher| {
            bencher.iter(|| black_box(&book_text).cast(&marks, strict).expect("marks"));
        });
        for (shape, cells) in [
            ("struct", &tick_cells),
            ("serie", &basket_cells),
            ("map", &book_cells),
        ] {
            group.bench_function(format!("read/serde_json_{shape}/{count}"), |bencher| {
                bencher.iter(|| {
                    for cell in black_box(cells) {
                        black_box(
                            serde_json::from_str::<serde_json::Value>(cell.expect("a document"))
                                .expect("JSON"),
                        );
                    }
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
    chunked_benchmarks,
    cast_benchmarks,
    json_cast_benchmarks,
    structured_benchmarks,
    record_options_benchmarks,
    null_visibility_benchmarks,
    memory_size_benchmarks,
    window_benchmarks,
    join_benchmarks,
);
criterion_main!(arrow_values);
