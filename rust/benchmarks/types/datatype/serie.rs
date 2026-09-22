//! The column side: what holding the buffers buys over boxing the rows.
//!
//! Every case here is paired against the thing it is meant to beat - taking
//! buffers against proving rows, a buffer read against building the value
//! that row would cost, a native write against the field's own contract on
//! the way to the same write - so a number that moves says which side moved.
//! The counts behind the pairs are pinned in `rust/tests/allocations.rs`;
//! these cases are the time those counts buy.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, StringArray, StructArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};
use criterion::{BatchSize, Criterion};
use yggdryl::{DataType, Field, Scalar, Serie, SerieValue, StructType};

/// Rows per measured column. The smoke corpus keeps `cargo test
/// --all-targets` under a second in a debug build.
const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

/// Legs per order row in the struct-of-list case.
const LEGS: usize = 4;

/// One non-null 64-bit column field.
fn price_field() -> Field {
    Field::new("price", DataType::Int64, false)
}

/// `ROWS` 64-bit values as Arrow buffers, freshly allocated so the column
/// taken over them holds them alone and its first edit lands in place.
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

/// The column over a fresh [`price_array`], holding its buffers alone.
fn price_column() -> Serie {
    Serie::from_arrow_array(price_field(), price_array()).expect("an int64 column")
}

/// One non-null record root of two leaf columns.
fn quotes_root() -> Field {
    let fields = StructType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::utf8(), false),
    ])
    .expect("two named children");
    Field::new("row", DataType::from(fields), false)
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

/// One non-null record root of an identifier and a list of int64 legs.
fn orders_root() -> Field {
    let fields = StructType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new(
            "legs",
            DataType::list(Field::new("item", DataType::Int64, false)),
            false,
        ),
    ])
    .expect("two named children");
    Field::new("order", DataType::from(fields), false)
}

/// One order row: an identifier and [`LEGS`] legs.
fn order_row(id: i64) -> Scalar {
    Scalar::from_sequence([
        Scalar::from(id),
        Scalar::from_sequence((0..LEGS).map(|leg| Scalar::from(id + leg as i64))),
    ])
}

/// `ROWS` orders under [`orders_root`], laid out from proven rows.
fn orders_column() -> Serie {
    Serie::from_scalars(
        orders_root(),
        (0..ROWS).map(|index| order_row(i64::try_from(index).expect("a row count fits i64"))),
    )
    .expect("a record column of lists")
}

pub(crate) fn serie_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("serie");

    // In. Taking buffers is a layout check and a null count on the validity
    // words; taking values is the field's contract once per row and one
    // layout. The gap is what a column buys a reader that already has Arrow.
    let array = price_array();
    group.bench_function("from_arrow_array", |bencher| {
        bencher.iter(|| {
            Serie::from_arrow_array(price_field(), ArrayRef::clone(black_box(&array)))
                .expect("an int64 column")
        });
    });
    let rows = price_rows();
    group.bench_function("from_scalars", |bencher| {
        bencher.iter(|| {
            Serie::from_scalars(price_field(), black_box(&rows).iter().cloned())
                .expect("proven int64 rows")
        });
    });

    // Random access: one read off the values buffer, against building the
    // value that row would otherwise cost.
    let serie = price_column();
    let leaf = serie.as_int64().expect("an int64 column");
    group.bench_function("value", |bencher| {
        bencher.iter(|| black_box(leaf).value(black_box(ROWS / 2)));
    });
    group.bench_function("scalar", |bencher| {
        bencher.iter(|| {
            black_box(&serie)
                .scalar(black_box(ROWS / 2))
                .expect("a stored row")
        });
    });

    // Growing a column that holds its buffers alone: one native value into
    // the values buffer, and one value through the field's contract on the
    // way to the same buffer. The column is rebuilt outside the timing so
    // every push is the amortized one and never the first edit's copy.
    group.bench_function("push_value", |bencher| {
        bencher.iter_batched(
            price_column,
            |mut growing| {
                growing
                    .get_int64_mut()
                    .expect("an int64 column")
                    .push_value(Some(1))
                    .expect("a present row");
                growing.len()
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("push", |bencher| {
        bencher.iter_batched(
            price_column,
            |mut growing| {
                growing.push(Scalar::from(1_i64)).expect("one int64 row");
                growing.len()
            },
            BatchSize::LargeInput,
        );
    });

    // Rewriting the middle slot: one present value over one present value
    // is one buffer write, against the amortized append `push` measured.
    group.bench_function("set_middle", |bencher| {
        bencher.iter_batched(
            price_column,
            |mut writing| {
                writing
                    .set(ROWS / 2, Scalar::from(1_i64))
                    .expect("one slot");
                writing.len()
            },
            BatchSize::LargeInput,
        );
    });

    // Out. A record root lends the buffers it holds as one table rather
    // than gathering rows.
    let records = Serie::from_arrow_array(quotes_root(), quotes_array()).expect("a record column");
    group.bench_function("into_arrow_batch", |bencher| {
        bencher.iter(|| {
            black_box(&records)
                .into_arrow_batch()
                .expect("a record root is a table")
        });
    });

    // A nested write: one record whose child is a list, pushed down through
    // the record's write into the list's cut and the items under it.
    let next = i64::try_from(ROWS).expect("a row count fits i64");
    group.bench_function("struct_of_list_push", |bencher| {
        bencher.iter_batched(
            orders_column,
            |mut orders| {
                orders.push(order_row(next)).expect("one order row");
                SerieValue::len(orders.as_struct().expect("a record column"))
            },
            BatchSize::LargeInput,
        );
    });

    group.finish();
}
