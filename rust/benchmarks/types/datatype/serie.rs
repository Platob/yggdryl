//! The column side: what building one costs, what cloning one does not cost,
//! and the two Arrow directions the family claims.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::{DataType, Field, Scalar, Serie, StructType};

/// Rows per measured column. The smoke corpus keeps `cargo test
/// --all-targets` under a second in a debug build.
const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

/// One non-null 64-bit column field.
fn price_field() -> Field {
    Field::new("price", DataType::Int64, false)
}

/// `ROWS` already-canonical 64-bit rows.
fn price_rows() -> Vec<Scalar> {
    (0..ROWS)
        .map(|index| Scalar::from(i64::try_from(index).expect("a row count fits i64")))
        .collect::<Vec<_>>()
}

/// One non-null record root of two columns.
fn quotes_root() -> Field {
    let fields = StructType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::utf8(), false),
    ])
    .expect("two named children");
    Field::new("row", DataType::Struct(fields), false)
}

/// `ROWS` named records under [`quotes_root`].
fn quotes_rows() -> Vec<Scalar> {
    (0..ROWS)
        .map(|index| {
            Scalar::from_struct([
                (
                    "id",
                    Scalar::from(i64::try_from(index).expect("a row count fits i64")),
                ),
                ("symbol", Scalar::from("AAPL")),
            ])
            .expect("one record")
        })
        .collect::<Vec<_>>()
}

pub(crate) fn serie_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("serie");

    // Building a column is the field's value contract, once per row: this is
    // the cost the rest of the family is allowed to stop paying.
    let rows = price_rows();
    group.bench_function("from_rows", |bencher| {
        bencher.iter(|| {
            Serie::from_rows(price_field(), black_box(&rows).iter().cloned())
                .expect("canonical int64 rows")
        });
    });

    // A clone is two pointer bumps whatever the row count, which is what
    // lets a column cross a boundary as a value.
    let serie = Serie::from_rows(price_field(), rows.clone()).expect("canonical int64 rows");
    group.bench_function("clone", |bencher| {
        bencher.iter(|| black_box(&serie).clone());
    });
    group.bench_function("as_slice", |bencher| {
        bencher.iter(|| black_box(&serie).as_slice().len());
    });

    // Both Arrow directions for one column, through the single scalar-array
    // boundary the family routes everything through.
    group.bench_function("into_arrow_array", |bencher| {
        bencher.iter(|| {
            black_box(&serie)
                .into_arrow_array()
                .expect("an int64 column materializes")
        });
    });
    let array = serie
        .into_arrow_array()
        .expect("an int64 column materializes");
    group.bench_function("from_arrow_array", |bencher| {
        bencher.iter(|| {
            Serie::from_arrow_array(price_field(), black_box(&array).as_ref())
                .expect("an int64 array reads back")
        });
    });

    // And the record doors: one table, and the stream a record write speaks.
    let table = Serie::from_rows(quotes_root(), quotes_rows()).expect("canonical records");
    group.bench_function("into_arrow_batch", |bencher| {
        bencher.iter(|| {
            black_box(&table)
                .into_arrow_batch()
                .expect("a record root materializes")
        });
    });
    let batch = table
        .into_arrow_batch()
        .expect("a record root materializes");
    group.bench_function("from_arrow_batch", |bencher| {
        bencher.iter(|| Serie::from_arrow_batch(black_box(&batch)).expect("a batch reads back"));
    });
    group.bench_function("into_arrow_reader_first_batch", |bencher| {
        bencher.iter(|| {
            let mut reader = black_box(&table)
                .into_arrow_reader()
                .expect("a record root streams");
            reader.next().expect("one batch").expect("a readable batch")
        });
    });

    group.finish();
}
