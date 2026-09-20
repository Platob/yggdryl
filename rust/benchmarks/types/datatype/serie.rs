//! The column side: what holding the buffers buys over boxing the rows.
//!
//! Every group here is paired against the thing it is meant to beat - random
//! access against decoding the whole column, appending a batch against
//! rebuilding one, dropping a child against rewriting the rows - so a number
//! that moves says which side moved.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, StringArray, StructArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};
use criterion::Criterion;
use yggdryl::{DataType, Field, Scalar, Serie, StructType};

/// Rows per measured column. The smoke corpus keeps `cargo test
/// --all-targets` under a second in a debug build.
const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

/// One non-null 64-bit column field.
fn price_field() -> Field {
    Field::new("price", DataType::Int64, false)
}

/// `ROWS` 64-bit values as Arrow buffers.
fn price_array() -> ArrayRef {
    Arc::new(Int64Array::from(
        (0..ROWS)
            .map(|index| i64::try_from(index).expect("a row count fits i64"))
            .collect::<Vec<_>>(),
    ))
}

/// `ROWS` 64-bit values as native rows.
fn price_rows() -> Vec<Scalar> {
    (0..ROWS)
        .map(|index| Scalar::from(i64::try_from(index).expect("a row count fits i64")))
        .collect()
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

/// `ROWS` records under [`quotes_root`], as Arrow buffers.
fn quotes_array() -> ArrayRef {
    let fields: Fields = vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]
    .into();
    let ids: ArrayRef = price_array();
    let symbols: ArrayRef = Arc::new(StringArray::from(vec!["AAPL"; ROWS]));
    Arc::new(StructArray::try_new(fields, vec![ids, symbols], None).expect("two equal columns"))
}

pub(crate) fn serie_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("serie");

    // Taking buffers is a layout check and a null count; taking values is the
    // field's value contract once per row. The gap is what a column buys a
    // reader that already has Arrow.
    let array = price_array();
    group.bench_function("from_arrow_array", |bencher| {
        bencher.iter(|| {
            Serie::from_arrow_array(price_field(), ArrayRef::clone(black_box(&array)))
                .expect("an int64 column")
        });
    });
    let rows = price_rows();
    group.bench_function("from_rows", |bencher| {
        bencher.iter(|| {
            Serie::from_rows(price_field(), black_box(&rows).iter().cloned())
                .expect("canonical int64 rows")
        });
    });

    let serie =
        Serie::from_arrow_array(price_field(), ArrayRef::clone(&array)).expect("an int64 column");

    // A clone is one pointer bump whatever the row count.
    group.bench_function("clone", |bencher| {
        bencher.iter(|| black_box(&serie).clone());
    });

    // Random access straight off the buffer, against building the value it
    // would otherwise cost, against decoding the whole column once.
    group.bench_function("i64_at", |bencher| {
        bencher.iter(|| black_box(&serie).i64_at(ROWS / 2));
    });
    group.bench_function("get", |bencher| {
        bencher.iter(|| black_box(&serie).get(ROWS / 2).expect("a readable row"));
    });
    group.bench_function("rows_uncached", |bencher| {
        bencher.iter(|| {
            let fresh = Serie::from_arrow_array(price_field(), ArrayRef::clone(&array))
                .expect("an int64 column");
            fresh.rows().expect("readable rows").len()
        });
    });

    // Growing: a value into the tail, and a whole run as one chunk.
    group.bench_function("push", |bencher| {
        bencher.iter(|| {
            let mut growing = black_box(&serie).clone();
            growing.push(Scalar::from(1_i64)).expect("one int64 row");
            growing.len()
        });
    });
    group.bench_function("append_arrow_array", |bencher| {
        bencher.iter(|| {
            let mut growing = black_box(&serie).clone();
            growing
                .append_arrow_array(ArrayRef::clone(&array))
                .expect("one more run");
            growing.len()
        });
    });

    // A window shares the buffers it spans.
    group.bench_function("slice", |bencher| {
        bencher.iter(|| black_box(&serie).slice(ROWS / 4, ROWS / 2).len());
    });

    // Out, in both shapes. One run lends its buffers rather than gathering.
    group.bench_function("into_arrow_array", |bencher| {
        bencher.iter(|| {
            black_box(&serie)
                .into_arrow_array()
                .expect("the column lends its run")
        });
    });

    // The record doors, and the child mutators that never read a row.
    let records = Serie::from_arrow_array(quotes_root(), quotes_array()).expect("a record column");
    group.bench_function("into_arrow_batch", |bencher| {
        bencher.iter(|| {
            black_box(&records)
                .into_arrow_batch()
                .expect("a record root materializes")
        });
    });
    group.bench_function("into_arrow_reader_first_batch", |bencher| {
        bencher.iter(|| {
            let mut reader = black_box(&records)
                .into_arrow_reader()
                .expect("a record root streams");
            reader.next().expect("one batch").expect("a readable batch")
        });
    });
    group.bench_function("child", |bencher| {
        bencher.iter(|| {
            black_box(&records)
                .child("symbol")
                .expect("a named child")
                .len()
        });
    });
    group.bench_function("without_child", |bencher| {
        bencher.iter(|| {
            black_box(&records)
                .without_child("symbol")
                .expect("a named child")
                .len()
        });
    });

    group.finish();
}
