//! The two evaluation paths, each against the baseline it has to justify.
//!
//! Every leg here answers "compared to what": the row path against the
//! hand-written Rust closure that is its honest upper bound, and the vectorized
//! path against both the `arrow_ord` comparison it compiles to and the
//! per-row-per-column `ArrayFormatter` loop it replaced.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, Int64Array, RecordBatch, Scalar, StringArray};
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use criterion::Criterion;
use yggdryl::expressions::ArrowApply;
use yggdryl::{Expr, Value};

use super::{fixture_batch, fixture_rows, schema};

/// The string comparison this change replaced, kept here as its baseline.
///
/// This is what `io::partition::filter_rows` did: for every filtered column,
/// build a formatter and render *every row* to a `String`, then compare. It is
/// deleted from the crate and lives on only as the number to beat.
fn formatter_filter(batch: &RecordBatch, column: &str, wanted: &str) -> RecordBatch {
    let options = FormatOptions::new().with_null("null");
    let index = batch
        .schema()
        .fields()
        .iter()
        .position(|field| field.name() == column)
        .expect("the column");
    let array = batch.column(index);
    let formatter = ArrayFormatter::try_new(array.as_ref(), &options).expect("a formatter");
    let mask: Vec<bool> = (0..batch.num_rows())
        .map(|row| {
            let spelled = if array.is_null(row) {
                "null".to_owned()
            } else {
                formatter.value(row).to_string()
            };
            spelled == wanted
        })
        .collect();
    arrow_select::filter::filter_record_batch(batch, &BooleanArray::from(mask))
        .expect("a filtered batch")
}

pub fn benchmarks(criterion: &mut Criterion) {
    let schema = schema();
    let batch = fixture_batch();
    let rows = fixture_rows();

    let mut group = criterion.benchmark_group("expression_row_eval");
    group.throughput(criterion::Throughput::Elements(rows.len() as u64));

    let predicate = "venue = 'XNAS' AND id > 100"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds")
        .into_predicate()
        .expect("a predicate");
    group.bench_function("bound_plan", |bencher| {
        bencher.iter(|| {
            black_box(&rows)
                .iter()
                .filter(|row| predicate.matches(row).expect("evaluates"))
                .count()
        });
    });
    // The honest upper bound: the same question written by hand over the same
    // values, with no plan, no dispatch, and no three-valued bookkeeping.
    group.bench_function("hand_written_closure", |bencher| {
        bencher.iter(|| {
            black_box(&rows)
                .iter()
                .filter(|row| {
                    let Value::Record(_, values) = row else {
                        return false;
                    };
                    let venue = values[0].as_str() == Some("XNAS");
                    let id = values[1].as_i64().is_some_and(|id| id > 100);
                    venue && id
                })
                .count()
        });
    });
    group.finish();

    let mut group = criterion.benchmark_group("expression_vectorized_eval");
    group.throughput(criterion::Throughput::Elements(batch.num_rows() as u64));

    let equality = "venue = 'XNAS'"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds")
        .into_predicate()
        .expect("a predicate");
    group.bench_function("bound_plan_equality", |bencher| {
        bencher.iter(|| {
            equality
                .filter_batch(black_box(&batch))
                .expect("a filtered batch")
        });
    });
    // What that compiles to: one `arrow_ord` comparison against a scalar built
    // once, plus one filter. Anything more is the plan's overhead.
    group.bench_function("hand_written_kernel", |bencher| {
        let scalar: ArrayRef = Arc::new(StringArray::from(vec!["XNAS"]));
        let scalar = Scalar::new(scalar);
        bencher.iter(|| {
            let mask = arrow_ord::cmp::eq(black_box(batch.column(0)), &scalar).expect("a mask");
            arrow_select::filter::filter_record_batch(black_box(&batch), &mask)
                .expect("a filtered batch")
        });
    });
    // And what it replaced: one rendered string per row per filtered column.
    group.bench_function("replaced_formatter_filter", |bencher| {
        bencher.iter(|| formatter_filter(black_box(&batch), "venue", "XNAS"));
    });
    group.finish();

    // Per family, so a type whose path quietly fell back to the row evaluator
    // shows up as an outlier rather than hiding inside an average.
    let mut group = criterion.benchmark_group("expression_by_family");
    group.throughput(criterion::Throughput::Elements(batch.num_rows() as u64));
    for (name, text) in [
        ("utf8_equality", "venue = 'XNAS'"),
        ("int64_range", "id > 100"),
        ("decimal_range", "price > 100.00"),
        ("null_test", "venue IS NULL"),
        ("set_membership", "id IN (1, 2, 3, 4, 5, 6, 7, 8)"),
        ("prefix", "venue LIKE 'XN%'"),
        ("wildcard_like", "venue LIKE 'X%S'"),
        // The fallback legs: arithmetic and functions have no kernel in the
        // crates the `arrow` feature links, so this is what that costs.
        ("arithmetic_fallback", "id + 1 > 100"),
        ("function_fallback", "lower(venue) = 'xnas'"),
    ] {
        let predicate = text
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema)
            .expect("binds")
            .into_predicate()
            .expect("a predicate");
        group.bench_function(name, |bencher| {
            bencher.iter(|| predicate.mask(black_box(&batch)).expect("a mask"));
        });
    }
    group.finish();

    // The subject legs: what the documentation tells a reader to hoist, with
    // the number that makes it advice worth taking.
    let mut group = criterion.benchmark_group("expression_apply_subject");
    group.throughput(criterion::Throughput::Elements(batch.num_rows() as u64));
    group.bench_function("text_subject_parsed_per_call", |bencher| {
        bencher.iter(|| {
            "venue = 'XNAS'"
                .apply_arrow_batch(black_box(batch.clone()))
                .expect("filters")
        });
    });
    group.bench_function("hoisted_bound_subject", |bencher| {
        let bound = "venue = 'XNAS'"
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema)
            .expect("binds");
        bencher.iter(|| {
            bound
                .apply_arrow_batch(black_box(batch.clone()))
                .expect("filters")
        });
    });
    group.finish();

    // Common-subexpression elimination, stated as a number: the same product
    // written three times is one node, so it is computed once per batch.
    let mut group = criterion.benchmark_group("expression_shared_subtree");
    group.throughput(criterion::Throughput::Elements(batch.num_rows() as u64));
    for (name, text) in [
        ("one_occurrence", "id * 2 > 100"),
        (
            "three_occurrences",
            "id * 2 > 100 AND id * 2 < 9000 AND id * 2 <> 400",
        ),
    ] {
        let predicate = text
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema)
            .expect("binds")
            .into_predicate()
            .expect("a predicate");
        group.bench_function(name, |bencher| {
            bencher.iter(|| predicate.mask(black_box(&batch)).expect("a mask"));
        });
    }
    group.finish();

    let _ = Int64Array::from(vec![0_i64; 0]);
}
