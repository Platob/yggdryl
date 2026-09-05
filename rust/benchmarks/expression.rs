//! What a filter costs, split into the two halves a caller pays separately.
//!
//! Parsing and binding happen once per stream; applying happens once per
//! batch. Measuring them together would hide the only number that scales, so
//! every group here measures exactly one of the two.
//!
//! The apply groups carry a **kernel baseline**: the same predicate written
//! directly against `arrow-ord` and `arrow-select`, with no expression
//! involved. The expression path can never be faster than that, and the gap is
//! the price of the grammar. A regression that widens the gap is visible here
//! before it is visible anywhere else.

#[path = "bench_profile.rs"]
mod bench_profile;

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, Decimal128Array, Int64Array, RecordBatch, Scalar as ArrowScalar,
    StringArray,
};
use arrow_buffer::BooleanBuffer;
use arrow_ord::cmp;
use criterion::{Criterion, criterion_group, criterion_main};
use yggdryl::expression::Statement;
use yggdryl::expression::{Bound, Bounds};
use yggdryl::{DataType, Expression, Field, Scalar};

/// Rows enough to make a per-batch cost visible and small enough to stay warm.
const ROWS: usize = bench_profile::corpus(65_536, 16_384);

/// The currencies the fixture cycles through, so a filter keeps a third.
const CURRENCIES: [&str; 3] = ["EUR", "USD", "GBP"];

/// One predicate, in the two spellings this benchmark compares.
struct Case {
    /// The group id, shared by the expression and the kernel measurement.
    name: &'static str,
    /// The predicate as a caller writes it.
    text: &'static str,
}

const CASES: [Case; 6] = [
    Case {
        name: "utf8_equality",
        text: "ccy = 'EUR'",
    },
    Case {
        name: "int64_range",
        text: "size > 500",
    },
    Case {
        name: "decimal_range",
        text: "price > decimal128(9,2) '100.00'",
    },
    Case {
        name: "null_test",
        text: "size is null",
    },
    Case {
        name: "set_membership",
        text: "ccy in ('EUR', 'GBP')",
    },
    Case {
        name: "conjunction",
        text: "ccy = 'EUR' and size > 500 and price > decimal128(9,2) '100.00'",
    },
];

/// The schema the fixture batch and every predicate is bound against.
fn schema() -> Field {
    Field::new(
        "trades",
        DataType::from_fields([
            Field::new("ccy", DataType::Utf8, true),
            Field::new("price", DataType::decimal128(9, 2).unwrap(), true),
            Field::new("size", DataType::Int64, true),
            Field::new("venue", DataType::Utf8, true),
        ])
        .unwrap(),
        false,
    )
}

/// One batch of trades: a third of one currency, one row in sixteen null.
fn batch() -> RecordBatch {
    let currencies: StringArray = (0..ROWS)
        .map(|row| Some(CURRENCIES[row % CURRENCIES.len()]))
        .collect();
    let prices: Decimal128Array = (0..ROWS)
        .map(|row| Some(i128::try_from(row % 20_000).unwrap_or_default()))
        .collect::<Decimal128Array>()
        .with_precision_and_scale(9, 2)
        .unwrap();
    let sizes: Int64Array = (0..ROWS)
        .map(|row| {
            if row % 16 == 0 {
                None
            } else {
                Some(i64::try_from(row % 1_000).unwrap_or_default())
            }
        })
        .collect();
    let venues: StringArray = (0..ROWS)
        .map(|row| Some(if row % 2 == 0 { "XNAS" } else { "XLON" }))
        .collect();
    let arrow_schema = schema().into_arrow_schema().unwrap();
    RecordBatch::try_new(
        arrow_schema,
        vec![
            Arc::new(currencies) as ArrayRef,
            Arc::new(prices) as ArrayRef,
            Arc::new(sizes) as ArrayRef,
            Arc::new(venues) as ArrayRef,
        ],
    )
    .unwrap()
}

/// Parsing: the half a caller pays once, per stream.
fn parse_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("expression_parse");
    for case in &CASES {
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| {
                black_box(case.text)
                    .parse::<Expression>()
                    .expect("the static predicate must parse")
            });
        });
    }
    group.bench_function("nested_path", |bencher| {
        bencher.iter(|| {
            black_box("trade.legs[0]['ccy'] = 'EUR' and trade.legs[1].size between 1 and 10")
                .parse::<Expression>()
                .expect("the static predicate must parse")
        });
    });
    group.bench_function("statement", |bencher| {
        bencher.iter(|| {
            black_box("select ccy, price as amount where ccy = 'EUR' order by price desc limit 10")
                .parse::<yggdryl::expression::Statement>()
                .expect("the static statement must parse")
        });
    });
    group.finish();
}

/// Binding and printing: the other half paid once, per stream.
fn bind_benchmarks(criterion: &mut Criterion) {
    let schema = schema();
    let mut group = criterion.benchmark_group("expression_bind");
    for case in &CASES {
        let parsed: Expression = case.text.parse().unwrap();
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| {
                black_box(&parsed)
                    .bind(black_box(&schema))
                    .expect("the static predicate must bind")
            });
        });
    }
    group.finish();

    let mut group = criterion.benchmark_group("expression_display");
    for case in &CASES {
        let parsed: Expression = case.text.parse().unwrap();
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| black_box(&parsed).to_string());
        });
    }
    group.finish();

    let expression: Expression = "ccy = 'EUR' and size > 500".parse().unwrap();
    let statement: Statement = "select ccy, price where ccy = 'EUR' order by price desc limit 10"
        .parse()
        .unwrap();
    let mut group = criterion.benchmark_group("expression_identity");
    group.bench_function("stable_hash_expression", |bencher| {
        bencher.iter(|| black_box(&expression).stable_hash());
    });
    group.bench_function("stable_hash_statement", |bencher| {
        bencher.iter(|| black_box(&statement).stable_hash());
    });
    group.finish();
}

/// Applying: the half a caller pays per batch, against the kernel baseline.
fn apply_benchmarks(criterion: &mut Criterion) {
    let schema = schema();
    let batch = batch();
    let bound: Vec<Bound> = CASES
        .iter()
        .map(|case| {
            case.text
                .parse::<Expression>()
                .unwrap()
                .bind(&schema)
                .unwrap()
        })
        .collect();

    let mut group = criterion.benchmark_group("expression_mask");
    group.throughput(criterion::Throughput::Elements(ROWS as u64));
    for (case, bound) in CASES.iter().zip(&bound) {
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| {
                black_box(bound)
                    .filter_mask(black_box(&batch))
                    .expect("the bound predicate must answer")
            });
        });
    }
    group.finish();

    let mut group = criterion.benchmark_group("kernel_mask");
    group.throughput(criterion::Throughput::Elements(ROWS as u64));
    for case in &CASES {
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| kernel_mask(black_box(&batch), black_box(case.name)));
        });
    }
    group.finish();

    let mut group = criterion.benchmark_group("expression_filter");
    group.throughput(criterion::Throughput::Elements(ROWS as u64));
    for (case, bound) in CASES.iter().zip(&bound) {
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| {
                black_box(bound)
                    .filter(black_box(&batch))
                    .expect("the bound predicate must filter")
            });
        });
    }
    group.finish();

    let mut group = criterion.benchmark_group("kernel_filter");
    group.throughput(criterion::Throughput::Elements(ROWS as u64));
    for case in &CASES {
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| {
                let mask = kernel_mask(black_box(&batch), black_box(case.name));
                arrow_select::filter::filter_record_batch(black_box(&batch), &mask)
                    .expect("the raw kernel must filter")
            });
        });
    }
    group.finish();
}

/// The same predicates, written straight against Arrow with no expression.
fn kernel_mask(batch: &RecordBatch, name: &str) -> BooleanArray {
    let column = |index: usize| batch.column(index).clone();
    let text = |value: &str| ArrowScalar::new(Arc::new(StringArray::from(vec![value])) as ArrayRef);
    let number = |value: i64| ArrowScalar::new(Arc::new(Int64Array::from(vec![value])) as ArrayRef);
    let decimal = |value: i128| {
        ArrowScalar::new(Arc::new(
            Decimal128Array::from(vec![value])
                .with_precision_and_scale(9, 2)
                .unwrap(),
        ) as ArrayRef)
    };
    match name {
        "utf8_equality" => cmp::eq(&column(0), &text("EUR")).unwrap(),
        "int64_range" => cmp::gt(&column(2), &number(500)).unwrap(),
        "decimal_range" => cmp::gt(&column(1), &decimal(10_000)).unwrap(),
        "null_test" => {
            let sizes = column(2);
            let present = sizes.nulls().map_or_else(
                || BooleanBuffer::new_set(sizes.len()),
                |nulls| nulls.inner().clone(),
            );
            BooleanArray::new(!&present, None)
        }
        "set_membership" => {
            let ccy = column(0);
            let left = cmp::eq(&ccy, &text("EUR")).unwrap();
            let right = cmp::eq(&ccy, &text("GBP")).unwrap();
            or_masks(&left, &right)
        }
        _ => {
            let first = cmp::eq(&column(0), &text("EUR")).unwrap();
            let second = cmp::gt(&column(2), &number(500)).unwrap();
            let third = cmp::gt(&column(1), &decimal(10_000)).unwrap();
            and_masks(&and_masks(&first, &second), &third)
        }
    }
}

/// Two-valued `or` of two masks, which is what a hand-written filter writes.
fn or_masks(left: &BooleanArray, right: &BooleanArray) -> BooleanArray {
    BooleanArray::new(left.values() | right.values(), None)
}

/// Two-valued `and` of two masks.
fn and_masks(left: &BooleanArray, right: &BooleanArray) -> BooleanArray {
    BooleanArray::new(left.values() & right.values(), None)
}

/// The scalar tier, for the shape of read that has no batch.
fn scalar_benchmarks(criterion: &mut Criterion) {
    let schema = schema();
    let rows: Vec<Scalar> = (0..1_024)
        .map(|row| {
            Scalar::from_sequence([
                Scalar::from(CURRENCIES[row % CURRENCIES.len()]),
                Scalar::d128(i128::try_from(row % 20_000).unwrap_or_default(), 2),
                if row % 16 == 0 {
                    Scalar::Null
                } else {
                    Scalar::from(i64::try_from(row % 1_000).unwrap_or_default())
                },
                Scalar::from(if row % 2 == 0 { "XNAS" } else { "XLON" }),
            ])
        })
        .collect();
    let mut group = criterion.benchmark_group("expression_rows");
    group.throughput(criterion::Throughput::Elements(rows.len() as u64));
    for case in &CASES {
        let bound = case
            .text
            .parse::<Expression>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| {
                black_box(&rows)
                    .iter()
                    .filter(|row| bound.matches(row).unwrap_or(false))
                    .count()
            });
        });
    }
    group.finish();
}

/// Pruning: the work a predicate does instead of reading anything at all.
fn prune_benchmarks(criterion: &mut Criterion) {
    let schema = schema();
    let bounds = Bounds::new(Some(ROWS as u64))
        .with_column(
            "ccy",
            Some(Scalar::from("EUR")),
            Some(Scalar::from("USD")),
            Some(0),
        )
        .with_column(
            "price",
            Some(Scalar::d128(0, 2)),
            Some(Scalar::d128(1_999_900, 2)),
            Some(0),
        )
        .with_column(
            "size",
            Some(Scalar::from(0)),
            Some(Scalar::from(999)),
            Some(4_096),
        )
        .with_column(
            "venue",
            Some(Scalar::from("XLON")),
            Some(Scalar::from("XNAS")),
            Some(0),
        );
    let mut group = criterion.benchmark_group("expression_prune");
    for case in &CASES {
        let bound = case
            .text
            .parse::<Expression>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        group.bench_function(case.name, |bencher| {
            bencher.iter(|| black_box(&bound).statistics_prune(black_box(&bounds)));
        });
    }
    group.finish();
}

criterion_group!(
    expression,
    parse_benchmarks,
    bind_benchmarks,
    apply_benchmarks,
    scalar_benchmarks,
    prune_benchmarks
);
criterion_main!(expression);
