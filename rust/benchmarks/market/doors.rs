//! What each product door costs over the bridge's own capture, against
//! the lifecycle every door composes.
//!
//! The reference is `lifecycle`: the one walk over the messages, which
//! every door runs first and then reads its products out of. A door's
//! time less the lifecycle's is what reading, folding and chaining its
//! product costs, and a door that came in under the lifecycle would be
//! skipping the walk. The book door carries a second walk over the makers
//! and a ladder read per step, so it is expected furthest above the line.
//! The Arrow twins are timed the same way against the lifecycle's own
//! twin, over the message rows the same messages land in, so the cost of
//! reading a row back as a message and writing a product as one is what
//! separates the pairs. Throughput is in messages of the capture.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput};
use yggdryl::arrow::BatchReader;
use yggdryl::{FixMsg, fix_schema};

use super::{MESSAGES, REPEATS, codec, messages};

/// The depth every book is read to, and the grid step: one second, which
/// is the whole capture, so every instrument reads one ladder per copy.
const DEPTH: u32 = 5;
const STEP: i64 = 1_000_000_000;

pub fn benchmarks(criterion: &mut Criterion) {
    let codec = codec();
    let messages = messages(&codec);
    let count = u64::try_from(messages.len()).expect("a count");
    let held: Vec<FixMsg> = messages;

    let mut group = criterion.benchmark_group("market/doors");
    group.throughput(Throughput::Elements(count));
    group.bench_function("lifecycle", |bencher| {
        bencher.iter_batched(
            || held.clone(),
            |messages| black_box(codec.lifecycle(black_box(messages)).count()),
            BatchSize::LargeInput,
        );
    });
    group.bench_function("orders", |bencher| {
        bencher.iter_batched(
            || held.clone(),
            |messages| black_box(codec.orders(black_box(messages)).count()),
            BatchSize::LargeInput,
        );
    });
    group.bench_function("executions", |bencher| {
        bencher.iter_batched(
            || held.clone(),
            |messages| black_box(codec.executions(black_box(messages)).count()),
            BatchSize::LargeInput,
        );
    });
    group.bench_function("trades", |bencher| {
        bencher.iter_batched(
            || held.clone(),
            |messages| black_box(codec.trades(black_box(messages)).count()),
            BatchSize::LargeInput,
        );
    });
    group.bench_function("quotes", |bencher| {
        bencher.iter_batched(
            || held.clone(),
            |messages| black_box(codec.quotes(black_box(messages)).count()),
            BatchSize::LargeInput,
        );
    });
    group.bench_function("books", |bencher| {
        bencher.iter_batched(
            || held.clone(),
            |messages| {
                black_box(
                    codec
                        .books(black_box(messages), DEPTH, STEP)
                        .expect("a grid and a depth")
                        .count(),
                )
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();

    // The Arrow twins, over the rows the same messages land in: one stream
    // of batches read once, its batches handed back as a fresh reader per
    // measured run.
    let schema = fix_schema(codec.registry(), "fix").expect("the fixed row");
    let batches: Vec<arrow_array::RecordBatch> = codec
        .arrow_reader(schema, held.clone())
        .expect("the message rows open")
        .map(|batch| batch.expect("a batch"))
        .collect();
    let arrow_schema = batches.first().expect("a batch").schema();
    let reader =
        || -> BatchReader { yggdryl::arrow::batch_reader(arrow_schema.clone(), batches.clone()) };
    let drain = |reader: BatchReader| -> usize {
        reader.map(|batch| batch.expect("a batch").num_rows()).sum()
    };
    assert_eq!(drain(reader()), MESSAGES * REPEATS, "the corpus as rows");

    let mut group = criterion.benchmark_group("market/arrow");
    group.throughput(Throughput::Elements(count));
    group.bench_function("lifecycle", |bencher| {
        bencher.iter_batched(
            reader,
            |source| {
                black_box(drain(
                    codec
                        .lifecycle_arrow_reader(black_box(source))
                        .expect("the walk opens"),
                ))
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("orders", |bencher| {
        bencher.iter_batched(
            reader,
            |source| {
                black_box(drain(
                    codec
                        .orders_arrow_reader(black_box(source))
                        .expect("the order rows open"),
                ))
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("executions", |bencher| {
        bencher.iter_batched(
            reader,
            |source| {
                black_box(drain(
                    codec
                        .executions_arrow_reader(black_box(source))
                        .expect("the execution rows open"),
                ))
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("trades", |bencher| {
        bencher.iter_batched(
            reader,
            |source| {
                black_box(drain(
                    codec
                        .trades_arrow_reader(black_box(source))
                        .expect("the trade rows open"),
                ))
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("quotes", |bencher| {
        bencher.iter_batched(
            reader,
            |source| {
                black_box(drain(
                    codec
                        .quotes_arrow_reader(black_box(source))
                        .expect("the quote rows open"),
                ))
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("books", |bencher| {
        bencher.iter_batched(
            reader,
            |source| {
                black_box(drain(
                    codec
                        .books_arrow_reader(black_box(source), DEPTH, STEP)
                        .expect("the book rows open"),
                ))
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}
