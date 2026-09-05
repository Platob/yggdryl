//! Per-row digest columns and holder filling.

#[cfg(feature = "arrow")]
use std::hint::black_box;
#[cfg(feature = "arrow")]
use std::sync::Arc;

use criterion::Criterion;

#[cfg(feature = "arrow")]
use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray, UInt64Array};
#[cfg(feature = "arrow")]
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
#[cfg(feature = "arrow")]
use yggdryl::{DataType, DigestAlgorithm, Field};

/// Rows per fixture, enough that the per-row cost dominates the setup.
#[cfg(feature = "arrow")]
const ROWS: usize = crate::bench_profile::corpus(65_536, 4_096);

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

/// The same four columns, with the two text ones dictionary-encoded.
///
/// A dictionary composes values rather than holding one buffer, so it has no
/// buffer arm and reads through the shared scalar boundary. The shape is
/// otherwise identical to [`buffered_batch`], which is what makes the two rows
/// a like-for-like comparison of the two paths rather than of two schemas.
#[cfg(feature = "arrow")]
fn fallback_batch() -> RecordBatch {
    use yggdryl::Scalar;

    let rows = |values: Vec<Scalar>| Scalar::from_sequence(values);
    let dictionary = DataType::from_str("dictionary<int32, utf8>").expect("a valid dictionary");
    let columns: Vec<(Field, Scalar)> = vec![
        (
            Field::new("id", DataType::Int64, false),
            rows((0..ROWS as i64).map(Scalar::from).collect()),
        ),
        (
            Field::new("symbol", dictionary.clone(), false),
            rows(
                (0..ROWS)
                    .map(|index| Scalar::from(if index % 2 == 0 { "AAPL" } else { "MSFT" }))
                    .collect(),
            ),
        ),
        (
            Field::new("price", DataType::Float64, false),
            rows(
                (0..ROWS)
                    .map(|index| Scalar::from(187.23 + index as f64 / 100.0))
                    .collect(),
            ),
        ),
        (
            Field::new("venue", dictionary, false),
            rows((0..ROWS).map(|_| Scalar::from("XNAS")).collect()),
        ),
    ];

    let mut fields = Vec::with_capacity(columns.len());
    let mut arrays: Vec<arrow_array::ArrayRef> = Vec::with_capacity(columns.len());
    for (field, values) in columns {
        arrays.push(
            yggdryl::arrow::array_from_value(&field, &values).expect("a valid column fixture"),
        );
        fields.push(field.into_arrow().expect("the field projects to Arrow"));
    }
    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).expect("a valid batch fixture")
}

/// One target root and the three source shapes holder filling distinguishes.
#[cfg(feature = "arrow")]
fn holder_fixtures() -> (Field, RecordBatch, RecordBatch, RecordBatch) {
    let symbol = Field::new("symbol", DataType::Utf8, false);
    let mut digest = Field::new("row_digest", DataType::UInt64, false);
    digest
        .as_digest_mut()
        .set_holder()
        .expect("a valid holder role");
    digest
        .as_digest_mut()
        .set_paths(["symbol"])
        .expect("a valid holder path");
    let root = DataType::from_fields([symbol.clone(), digest.clone()])
        .expect("a valid Struct")
        .required_field("row");

    let symbols = Arc::new(
        (0..ROWS)
            .map(|index| Some(if index % 2 == 0 { "AAPL" } else { "MSFT" }))
            .collect::<StringArray>(),
    ) as ArrayRef;
    let missing = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            symbol.clone().into_arrow().expect("Arrow field"),
        ])),
        vec![Arc::clone(&symbols)],
    )
    .expect("a batch without its holder");
    let defaults = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            symbol.clone().into_arrow().expect("Arrow field"),
            digest.clone().into_arrow().expect("Arrow field"),
        ])),
        vec![
            Arc::clone(&symbols),
            Arc::new(UInt64Array::from(vec![0_u64; ROWS])),
        ],
    )
    .expect("a batch of default holders");
    let populated = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            symbol.into_arrow().expect("Arrow field"),
            digest.into_arrow().expect("Arrow field"),
        ])),
        vec![
            symbols,
            Arc::new(UInt64Array::from_iter_values(1..=ROWS as u64)),
        ],
    )
    .expect("a batch of populated holders");
    (root, missing, defaults, populated)
}

/// Row digests: the buffer path, the scalar fallback, and materializing rows.
///
/// The naive row is what a caller writes without this module - build every
/// row as a `Scalar`, then digest each - so the gap is what reading the
/// buffers directly is worth. The fallback row is the same work through the
/// shared scalar boundary, which every layout without a buffer arm takes.
#[cfg(feature = "arrow")]
pub(crate) fn row_digest_benchmarks(criterion: &mut Criterion) {
    use criterion::Throughput;

    let buffered = buffered_batch();
    let fallback = fallback_batch();

    let mut group = criterion.benchmark_group("xxhash_row_digests");
    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("buffer_path", |bencher| {
        bencher.iter(|| {
            yggdryl::xxhash::arrow::row_digests(black_box(&buffered), DigestAlgorithm::Xxh3)
                .expect("the batch digests")
        });
    });
    group.bench_function("scalar_fallback", |bencher| {
        bencher.iter(|| {
            yggdryl::xxhash::arrow::row_digests(black_box(&fallback), DigestAlgorithm::Xxh3)
                .expect("the batch digests")
        });
    });
    group.bench_function("materialized_rows", |bencher| {
        bencher.iter(|| {
            let rows = yggdryl::arrow::batch_to_value(black_box(&buffered))
                .expect("the batch reads as rows");
            rows.as_sequence()
                .expect("a sequence of rows")
                .iter()
                .map(|row| row.digest(DigestAlgorithm::Xxh3))
                .collect::<Vec<_>>()
        });
    });
    group.bench_function("buffer_path_128", |bencher| {
        bencher.iter(|| {
            yggdryl::xxhash::arrow::row_digests(black_box(&buffered), DigestAlgorithm::Xxh128)
                .expect("the batch digests")
        });
    });
    group.finish();
}

/// Fill insertion, conditional recomputation, preservation, and force paths.
#[cfg(feature = "arrow")]
pub(crate) fn holder_fill_benchmarks(criterion: &mut Criterion) {
    use criterion::Throughput;
    use yggdryl::xxhash::Xxh3;

    let (root, missing, defaults, populated) = holder_fixtures();
    let state = Xxh3::with_seed(7);
    let mut group = criterion.benchmark_group("xxhash_fill_arrow_batch");
    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("missing_holder", |bencher| {
        bencher.iter(|| {
            state
                .fill_arrow_batch(black_box(&root), black_box(missing.clone()), false)
                .expect("the holder fills")
        });
    });
    group.bench_function("default_holders", |bencher| {
        bencher.iter(|| {
            state
                .fill_arrow_batch(black_box(&root), black_box(defaults.clone()), false)
                .expect("the default holders fill")
        });
    });
    group.bench_function("populated_holders", |bencher| {
        bencher.iter(|| {
            state
                .fill_arrow_batch(black_box(&root), black_box(populated.clone()), false)
                .expect("the populated holders are preserved")
        });
    });
    group.bench_function("forced_holders", |bencher| {
        bencher.iter(|| {
            state
                .fill_arrow_batch(black_box(&root), black_box(populated.clone()), true)
                .expect("the populated holders are recomputed")
        });
    });
    group.finish();
}

/// Row digests need the Arrow runtime; a schema-only build has no batch.
#[cfg(not(feature = "arrow"))]
pub(crate) fn row_digest_benchmarks(_criterion: &mut Criterion) {}

/// Holder filling needs the Arrow runtime too.
#[cfg(not(feature = "arrow"))]
pub(crate) fn holder_fill_benchmarks(_criterion: &mut Criterion) {}
