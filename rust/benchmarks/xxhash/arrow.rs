//! Per-row digest columns: the buffer path, the fallback, and the naive way.

#[cfg(feature = "arrow")]
use std::hint::black_box;
#[cfg(feature = "arrow")]
use std::sync::Arc;

use criterion::Criterion;

#[cfg(feature = "arrow")]
use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
#[cfg(feature = "arrow")]
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
#[cfg(feature = "arrow")]
use yggdryl::DigestAlgorithm;

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
    use yggdryl::{DataType, Field, Scalar};

    let rows = |values: Vec<Scalar>| Scalar::from_sequence(values);
    let dictionary = DataType::from_str("dictionary<int32, utf8>").expect("a valid dictionary");
    let columns: Vec<(Field, Scalar)> = vec![
        (
            Field::new("id", DataType::Int64, false),
            rows((0..ROWS as i64).map(Scalar::I64).collect()),
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
            yggdryl::xxhash::arrow::row_digests(black_box(&buffered), DigestAlgorithm::Xxh3_64)
                .expect("the batch digests")
        });
    });
    group.bench_function("scalar_fallback", |bencher| {
        bencher.iter(|| {
            yggdryl::xxhash::arrow::row_digests(black_box(&fallback), DigestAlgorithm::Xxh3_64)
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
                .map(|row| row.digest(DigestAlgorithm::Xxh3_64))
                .collect::<Vec<_>>()
        });
    });
    group.bench_function("buffer_path_128", |bencher| {
        bencher.iter(|| {
            yggdryl::xxhash::arrow::row_digests(black_box(&buffered), DigestAlgorithm::Xxh3_128)
                .expect("the batch digests")
        });
    });
    group.finish();
}

/// Row digests need the Arrow runtime; a schema-only build has no batch.
#[cfg(not(feature = "arrow"))]
pub(crate) fn row_digest_benchmarks(_criterion: &mut Criterion) {}
