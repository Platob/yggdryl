//! Coupled columns beside the digest columns they wrap, and coupled holders.

#[cfg(feature = "arrow")]
use std::hint::black_box;
#[cfg(feature = "arrow")]
use std::sync::Arc;

use criterion::Criterion;

#[cfg(feature = "arrow")]
use arrow_array::{
    ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
    TimestampNanosecondArray,
};
#[cfg(feature = "arrow")]
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
#[cfg(feature = "arrow")]
use yggdryl::{DataType, DigestAlgorithm, Field, TimeUnit, Timezone};

/// Rows per fixture, enough that the per-row cost dominates the setup.
#[cfg(feature = "arrow")]
const ROWS: usize = crate::bench_profile::corpus(65_536, 4_096);

/// The instant column every case couples, one microsecond apart.
#[cfg(feature = "arrow")]
fn instants() -> ArrayRef {
    Arc::new(
        TimestampMicrosecondArray::from_iter_values(
            (0..ROWS as i64).map(|index| super::INSTANT + index),
        )
        .with_timezone("UTC"),
    )
}

/// The same instants at nanoseconds, so the coupling has to floor each one.
#[cfg(feature = "arrow")]
fn nano_instants() -> ArrayRef {
    Arc::new(TimestampNanosecondArray::from_iter_values(
        (0..ROWS as i64).map(|index| (super::INSTANT + index) * 1_000 + 7),
    ))
}

/// A four-column batch whose columns all take the buffer path.
#[cfg(feature = "arrow")]
fn buffered_batch() -> RecordBatch {
    let ids: Int64Array = (0..ROWS as i64).collect::<Vec<_>>().into();
    let symbols: StringArray = (0..ROWS)
        .map(|index| Some(if index % 2 == 0 { "AAPL" } else { "MSFT" }))
        .collect();
    let prices: Float64Array = (0..ROWS)
        .map(|index| 187.23 + index as f64 / 100.0)
        .collect::<Vec<_>>()
        .into();
    let venues: StringArray = (0..ROWS).map(|_| Some("XNAS")).collect();
    RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
            ArrowField::new("price", ArrowDataType::Float64, false),
            ArrowField::new("venue", ArrowDataType::Utf8, false),
        ])),
        vec![
            Arc::new(ids),
            Arc::new(symbols),
            Arc::new(prices),
            Arc::new(venues),
        ],
    )
    .expect("a valid batch fixture")
}

/// Coupled columns against the digest column and instant column they join.
#[cfg(feature = "arrow")]
pub(crate) fn column_benchmarks(criterion: &mut Criterion) {
    use criterion::Throughput;

    let batch = buffered_batch();
    let instants = instants();
    let nanos = nano_instants();
    let digests = yggdryl::xxhash::arrow::row_digests(&batch, DigestAlgorithm::Xxh3)
        .expect("the batch digests");
    let coupled = yggdryl::txhash::arrow::row_txhashes(
        &batch,
        instants.as_ref(),
        TimeUnit::Microsecond,
        DigestAlgorithm::Xxh3,
    )
    .expect("the batch couples");

    let mut group = criterion.benchmark_group("txhash_columns");
    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("row_digests", |bencher| {
        bencher.iter(|| {
            yggdryl::xxhash::arrow::row_digests(black_box(&batch), DigestAlgorithm::Xxh3)
                .expect("the batch digests")
        });
    });
    group.bench_function("row_txhashes", |bencher| {
        bencher.iter(|| {
            yggdryl::txhash::arrow::row_txhashes(
                black_box(&batch),
                black_box(instants.as_ref()),
                TimeUnit::Microsecond,
                DigestAlgorithm::Xxh3,
            )
            .expect("the batch couples")
        });
    });
    group.bench_function("row_txhashes_128", |bencher| {
        bencher.iter(|| {
            yggdryl::txhash::arrow::row_txhashes(
                black_box(&batch),
                black_box(instants.as_ref()),
                TimeUnit::Microsecond,
                DigestAlgorithm::Xxh128,
            )
            .expect("the batch couples")
        });
    });
    let symbol = Field::new("symbol", DataType::utf8(), false);
    let symbols = Arc::clone(batch.column(1));
    group.bench_function("column_digests", |bencher| {
        bencher.iter(|| {
            yggdryl::xxhash::arrow::column_digests(
                black_box(Arc::clone(&symbols)),
                &symbol,
                DigestAlgorithm::Xxh3,
            )
            .expect("the column digests")
        });
    });
    group.bench_function("column_txhashes", |bencher| {
        bencher.iter(|| {
            yggdryl::txhash::arrow::column_txhashes(
                black_box(instants.as_ref()),
                black_box(Arc::clone(&symbols)),
                &symbol,
                TimeUnit::Microsecond,
                DigestAlgorithm::Xxh3,
            )
            .expect("the column couples")
        });
    });
    group.bench_function("unix_array/same_unit", |bencher| {
        bencher.iter(|| {
            yggdryl::txhash::arrow::unix_array(black_box(instants.as_ref()), TimeUnit::Microsecond)
                .expect("an instant column")
        });
    });
    group.bench_function("unix_array/floored", |bencher| {
        bencher.iter(|| {
            yggdryl::txhash::arrow::unix_array(black_box(nanos.as_ref()), TimeUnit::Microsecond)
                .expect("an instant column")
        });
    });
    group.bench_function("compose", |bencher| {
        bencher.iter(|| {
            yggdryl::txhash::arrow::compose(
                black_box(instants.as_ref()),
                black_box(digests.as_ref()),
                TimeUnit::Microsecond,
                DigestAlgorithm::Xxh3,
            )
            .expect("the halves compose")
        });
    });
    group.bench_function("decompose", |bencher| {
        bencher.iter(|| {
            yggdryl::txhash::arrow::decompose(
                black_box(coupled.as_ref()),
                TimeUnit::Microsecond,
                DigestAlgorithm::Xxh3,
            )
            .expect("the column splits")
        });
    });
    group.finish();
}

/// One root with a coupled holder beside one with a plain holder.
#[cfg(feature = "arrow")]
fn holder_fixtures() -> (Field, Field, RecordBatch) {
    let event = Field::new(
        "event",
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::UTC,
        },
        false,
    );
    let symbol = Field::new("symbol", DataType::utf8(), false);
    let mut plain = Field::new("row_digest", DataType::UInt64, false);
    plain
        .as_digest_mut()
        .set_holder()
        .expect("a valid holder role");
    let mut coupled = Field::new(
        "key",
        DataType::fixed_size_binary(16).expect("sixteen bytes is a width"),
        false,
    );
    coupled
        .as_digest_mut()
        .set_holder()
        .expect("a valid holder role");
    coupled
        .as_digest_mut()
        .set_time("event")
        .expect("a valid instant path");
    let plain_root = DataType::from_fields([event.clone(), symbol.clone(), plain])
        .expect("a valid Struct")
        .required_field("row");
    let coupled_root = DataType::from_fields([event.clone(), symbol.clone(), coupled])
        .expect("a valid Struct")
        .required_field("row");
    let symbols: ArrayRef = Arc::new(
        (0..ROWS)
            .map(|index| Some(if index % 2 == 0 { "AAPL" } else { "MSFT" }))
            .collect::<StringArray>(),
    );
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            event.into_arrow().expect("Arrow field"),
            symbol.into_arrow().expect("Arrow field"),
        ])),
        vec![instants(), symbols],
    )
    .expect("a batch without its holder");
    (plain_root, coupled_root, batch)
}

/// Filling a coupled holder beside filling a plain one over the same rows.
#[cfg(feature = "arrow")]
pub(crate) fn holder_fill_benchmarks(criterion: &mut Criterion) {
    use criterion::Throughput;

    let (plain, coupled, batch) = holder_fixtures();
    let mut group = criterion.benchmark_group("txhash_apply_arrow_batch");
    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("plain_holder", |bencher| {
        bencher.iter(|| {
            plain
                .as_digest()
                .apply_arrow_batch(black_box(&batch))
                .expect("the holder fills")
        });
    });
    group.bench_function("coupled_holder", |bencher| {
        bencher.iter(|| {
            coupled
                .as_digest()
                .apply_arrow_batch(black_box(&batch))
                .expect("the coupled holder fills")
        });
    });
    group.finish();
}

#[cfg(not(feature = "arrow"))]
pub(crate) fn column_benchmarks(_criterion: &mut Criterion) {}

#[cfg(not(feature = "arrow"))]
pub(crate) fn holder_fill_benchmarks(_criterion: &mut Criterion) {}
