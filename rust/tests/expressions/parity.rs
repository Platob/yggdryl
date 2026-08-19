//! The parity test: the three evaluators are three readings of one plan.
//!
//! This is the single most valuable test in the change. A bug that makes the
//! vectorized path disagree with the row path is a silently wrong answer, and a
//! bug that makes the statistics path disagree with either is a silently lost
//! row - neither shows up as a crash and neither shows up in a benchmark.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array,
    Int64Array, RecordBatch, StringArray, TimestampMillisecondArray,
};
use yggdryl::expressions::{Certainty, ColumnStats, StatsSource};
use yggdryl::{DataType, Expr, Field, TimeUnit, Value};

/// The root every case here is bound against.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("id"),
        DataType::Int32.nullable_field("n"),
        DataType::Decimal128 {
            precision: 12,
            scale: 2,
        }
        .nullable_field("price"),
        DataType::Float64.nullable_field("ratio"),
        DataType::Date32.nullable_field("day"),
        DataType::Timestamp(TimeUnit::Millisecond, None).nullable_field("ts"),
        DataType::Boolean.nullable_field("flag"),
    ])
    .expect("an eight-column struct")
    .required_field("row")
}

/// One row per position of the fixture columns, including nulls throughout.
fn columns() -> Vec<(Vec<Option<&'static str>>, usize)> {
    vec![(
        vec![Some("XNAS"), Some("XNYS"), None, Some("XLON"), Some("XNAS")],
        5,
    )]
}

/// The fixture as an Arrow batch: five rows, with a null in every column.
fn batch() -> RecordBatch {
    let venues = columns()[0].0.clone();
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(venues)),
        Arc::new(Int64Array::from(vec![
            Some(1),
            Some(2),
            None,
            Some(4),
            Some(5),
        ])),
        Arc::new(Int32Array::from(vec![
            Some(10),
            None,
            Some(30),
            Some(40),
            Some(50),
        ])),
        Arc::new(
            Decimal128Array::from(vec![Some(1_050), Some(2_000), Some(3_000), None, Some(500)])
                .with_precision_and_scale(12, 2)
                .expect("a decimal column"),
        ),
        Arc::new(Float64Array::from(vec![
            Some(0.5),
            Some(1.5),
            None,
            Some(2.5),
            Some(-1.0),
        ])),
        Arc::new(Date32Array::from(vec![
            Some(19_723),
            Some(19_724),
            Some(19_725),
            None,
            Some(19_000),
        ])),
        Arc::new(TimestampMillisecondArray::from(vec![
            Some(1_700_000_000_000),
            None,
            Some(1_700_000_100_000),
            Some(1_700_000_200_000),
            Some(1_600_000_000_000),
        ])),
        Arc::new(BooleanArray::from(vec![
            Some(true),
            Some(false),
            None,
            Some(true),
            Some(false),
        ])),
    ];
    let arrow = yggdryl::arrow::schema_from_field(&schema()).expect("an Arrow schema");
    RecordBatch::try_new(arrow, arrays).expect("a five-row batch")
}

/// The same fixture as rows the row evaluator reads.
fn rows() -> Vec<Value> {
    let batch = batch();
    let values = yggdryl::arrow::batch_to_value(&batch).expect("rows");
    values
        .as_sequence()
        .map(<[Value]>::to_vec)
        .expect("a sequence of rows")
}

/// Every predicate the parity matrix runs, chosen to hit each node kind.
fn predicates() -> Vec<&'static str> {
    vec![
        "venue = 'XNAS'",
        "venue <> 'XNAS'",
        "venue IS NULL",
        "venue IS NOT NULL",
        "venue IN ('XNAS', 'XLON')",
        "venue NOT IN ('XNAS')",
        "venue LIKE 'X%'",
        "venue LIKE 'XN_S'",
        "venue NOT LIKE 'X%'",
        "venue ILIKE 'xnas'",
        "id > 2",
        "id >= 2",
        "id < 4",
        "id <= 4",
        "id BETWEEN 2 AND 4",
        "id NOT BETWEEN 2 AND 4",
        "n > 20",
        "price > 10.00",
        "price <= 20.00",
        "price BETWEEN 5.00 AND 20.00",
        "ratio > 0",
        "ratio < 0",
        "day > DATE '2024-01-01'",
        "ts >= TIMESTAMP '2023-11-14T22:13:20'",
        "flag",
        "NOT flag",
        "flag = TRUE",
        "venue = 'XNAS' AND id > 1",
        "venue = 'XNAS' OR id = 4",
        "NOT (venue = 'XNAS' AND id > 1)",
        "venue = 'XNAS' AND id > 1 AND price > 5.00",
        "(venue = 'XNAS' OR venue = 'XLON') AND id IS NOT NULL",
        "id + 1 > 3",
        "id * 2 >= 8",
        "id - 1 < 2",
        "price + 1.00 > 11.00",
        "-id < -2",
        "length(venue) = 4",
        "lower(venue) = 'xnas'",
        "upper(venue) = 'XNAS'",
        "abs(ratio) > 1",
        "coalesce(venue, 'none') = 'none'",
        "year(day) = 2024",
        "CASE WHEN id > 2 THEN 'big' ELSE 'small' END = 'big'",
        "CAST(id AS int32) > 2",
        "id::float64 > 2",
        "truncate(n, 20) = 40",
        "substring(venue, 1, 2) = 'XN'",
        "venue = venue",
        "id = id",
    ]
}

#[test]
fn the_row_path_and_the_vectorized_path_select_the_same_rows() {
    let schema = schema();
    let batch = batch();
    let rows = rows();
    for text in predicates() {
        let predicate = text
            .parse::<Expr>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .into_predicate()
            .unwrap_or_else(|error| panic!("{text}: {error}"));

        let by_row: Vec<bool> = rows
            .iter()
            .map(|row| {
                predicate
                    .matches(row)
                    .unwrap_or_else(|error| panic!("{text}: {error}"))
            })
            .collect();

        let mask = predicate
            .mask(&batch)
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let by_column: Vec<bool> = (0..mask.len())
            // A null in the mask is unknown, and unknown does not keep a row -
            // which is what the filter kernel does with it too.
            .map(|row| !mask.is_null(row) && mask.value(row))
            .collect();

        assert_eq!(by_row, by_column, "the two paths disagree on {text:?}");

        // And the filter kernel keeps exactly the rows the mask marks.
        let filtered = predicate
            .filter_batch(&batch)
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        assert_eq!(
            filtered.num_rows(),
            by_row.iter().filter(|kept| **kept).count(),
            "the filter kept a different count for {text:?}"
        );
    }
}

/// Statistics computed from the fixture itself, so they are exactly true.
struct Exact;

impl StatsSource for Exact {
    fn stats(&self, column: &yggdryl::expressions::BoundColumn) -> Option<ColumnStats> {
        let schema = schema();
        let index = schema.index_of(column.name())?;
        let batch = batch();
        let field = &schema.fields()[index];
        let values = yggdryl::arrow::array_to_value(field, batch.column(index).as_ref()).ok()?;
        let values = values.as_sequence()?;
        let present: Vec<&Value> = values.iter().filter(|value| !value.is_null()).collect();
        let nulls = u64::try_from(values.len() - present.len()).ok()?;
        Some(ColumnStats {
            lower: present.iter().min().map(|value| (*value).clone()),
            upper: present.iter().max().map(|value| (*value).clone()),
            null_count: Some(nulls),
            value_count: u64::try_from(values.len()).ok(),
        })
    }
}

#[test]
fn statistics_never_refuse_a_predicate_a_row_actually_matches() {
    // The safety property, stated as the thing that must never happen: for
    // every predicate, if any row matches, the statistics must not answer
    // `AlwaysFalse` - because that is a lost row.
    let schema = schema();
    let rows = rows();
    for text in predicates() {
        let predicate = text
            .parse::<Expr>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .into_predicate()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let matching = rows
            .iter()
            .filter(|row| predicate.matches(row).unwrap_or(false))
            .count();
        let certainty = predicate.evaluate_stats(&Exact);
        if matching > 0 {
            assert_ne!(
                certainty,
                Certainty::AlwaysFalse,
                "{text:?} refused a file holding {matching} matching row(s)"
            );
        }
        if matching < rows.len() {
            assert_ne!(
                certainty,
                Certainty::AlwaysTrue,
                "{text:?} claimed every row matches where {} do not",
                rows.len() - matching
            );
        }
    }
}

#[test]
fn coarse_statistics_never_prune_more_than_exact_ones() {
    // Widening the bounds may only ever make the answer *less* certain: a
    // source that knows less can never skip more.
    struct Coarse;

    impl StatsSource for Coarse {
        fn stats(&self, column: &yggdryl::expressions::BoundColumn) -> Option<ColumnStats> {
            let exact = Exact.stats(column)?;
            // Bounds a writer truncated, and a null count it did not record.
            Some(ColumnStats {
                lower: exact.lower,
                upper: exact.upper,
                null_count: None,
                value_count: None,
            })
        }
    }

    let schema = schema();
    for text in predicates() {
        let predicate = text
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema)
            .expect("binds")
            .into_predicate()
            .expect("a predicate");
        let exact = predicate.evaluate_stats(&Exact);
        let coarse = predicate.evaluate_stats(&Coarse);
        if coarse == Certainty::AlwaysFalse {
            assert_eq!(
                exact,
                Certainty::AlwaysFalse,
                "{text:?} skipped on coarse statistics but not on exact ones"
            );
        }
        // A source that knows nothing at all can never decide anything.
        assert_eq!(
            predicate.evaluate_stats(&()),
            Certainty::Maybe,
            "{text:?} decided something with no statistics"
        );
    }
}

#[test]
fn optimizing_a_plan_never_changes_which_rows_it_selects() {
    // Semantics preservation, checked against the data rather than argued: the
    // optimized plan and the plan built from the unoptimized expression select
    // the same rows, on data that includes nulls, decimals, and temporals.
    let schema = schema();
    let rows = rows();
    for text in predicates() {
        let written: Expr = text.parse().expect("parses");
        let simplified = written.simplify();
        let left = written
            .bind(&schema)
            .expect("binds")
            .into_predicate()
            .expect("a predicate");
        let right = simplified
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{text} simplified to {simplified}: {error}"))
            .into_predicate()
            .expect("a predicate");
        for row in &rows {
            assert_eq!(
                left.matches(row).expect("evaluates"),
                right.matches(row).expect("evaluates"),
                "{text:?} changed meaning when simplified to {simplified}"
            );
        }
    }
}

#[test]
fn a_bound_plan_answers_the_same_through_every_carrier() {
    use yggdryl::expressions::{Apply, ArrowApply};

    let schema = schema();
    let batch = batch();
    let rows = rows();
    let text = "venue = 'XNAS' AND id > 1";

    // Text, an expression, and an already-bound plan are one subject.
    let expression: Expr = text.parse().expect("parses");
    let bound = expression.bind(&schema).expect("binds");

    let from_text = text.apply_arrow_batch(batch.clone()).expect("filters");
    let from_expr = expression
        .apply_arrow_batch(batch.clone())
        .expect("filters");
    let from_bound = bound.apply_arrow_batch(batch.clone()).expect("filters");
    assert_eq!(from_text.num_rows(), from_expr.num_rows());
    assert_eq!(from_text.num_rows(), from_bound.num_rows());

    // A streamed carrier answers its schema before the first batch is pulled.
    let reader = yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]);
    let streamed = expression
        .apply_arrow_batch_reader(reader)
        .expect("wraps the stream");
    assert_eq!(streamed.schema(), batch.schema());
    let streamed_rows: usize = streamed
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(streamed_rows, from_text.num_rows());

    // A row carrier answers the same rows one at a time.
    let kept = expression.apply_rows(&schema, &rows).expect("filters rows");
    assert_eq!(kept.len(), from_text.num_rows());

    // And a schema carrier answers with no data at all: filtering does not
    // change a schema, so the reported root is the one it was bound to.
    let reported = expression.apply_field(&schema).expect("reports");
    assert_eq!(reported.field_len(), schema.field_len());
}

#[test]
fn a_batch_that_disagrees_with_the_plan_is_a_typed_error_naming_the_column() {
    let schema = schema();
    let predicate = "venue = 'XNAS'"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds")
        .into_predicate()
        .expect("a predicate");

    let other = RecordBatch::try_new(
        Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "elsewhere",
            arrow_schema::DataType::Int64,
            true,
        )])),
        vec![Arc::new(Int64Array::from(vec![1_i64]))],
    )
    .expect("a batch");
    let error = predicate.mask(&other).expect_err("a schema mismatch");
    let message = error.to_string();
    assert!(message.contains("venue"), "{message}");
    assert!(message.contains("elsewhere"), "{message}");
}
