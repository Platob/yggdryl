//! The Rust half of the expression exchange with an outside implementation.
//!
//! Self-consistency proves nothing about a semantics: the row evaluator, the
//! vectorized evaluator, and the statistics evaluator agreeing with *each
//! other* is exactly what this crate's own tests already assert, and all three
//! could still be wrong together about what `venue <> 'XNAS'` does to a null.
//!
//! So this half writes a corpus - one Parquet file of deliberately awkward
//! rows, plus a list of predicates in this grammar's spelling - and reports,
//! for each predicate, which row indexes it selects.
//! [`scripts/check_expression_interop.py`](../../scripts/check_expression_interop.py)
//! then asks PyArrow the same questions over the same file and compares, and
//! asks PyIceberg's inclusive metrics evaluator which files this engine's
//! statistics evaluator would skip.
//!
//! When the corpus cannot be written this prints `SKIPPED`, which the driver
//! fails on - so a skipped half can never read as a pass.

#![cfg(all(feature = "parquet", feature = "arrow"))]

use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int64Array, RecordBatch,
    StringArray,
};
use yggdryl::io::IOBase;
use yggdryl::local::File;
use yggdryl::{DataType, Expr, Field, TimeUnit};

/// Where the corpus is written, beside every other interop target.
fn interop() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("expression-interop")
}

/// The corpus schema: one column per family a predicate meets.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("id"),
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
    .expect("a seven-column struct")
    .required_field("row")
}

/// Rows chosen so a null appears in every column and every comparison has a
/// boundary case: equal, just below, just above.
fn corpus() -> RecordBatch {
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![
            Some("XNAS"),
            Some("XNYS"),
            None,
            Some("XLON"),
            Some("XNAS"),
            Some(""),
        ])),
        Arc::new(Int64Array::from(vec![
            Some(1_i64),
            Some(2),
            None,
            Some(3),
            Some(10),
            Some(-1),
        ])),
        Arc::new(
            Decimal128Array::from(vec![
                Some(1_000_i128),
                Some(1_050),
                Some(2_000),
                None,
                Some(0),
                Some(-500),
            ])
            .with_precision_and_scale(12, 2)
            .expect("a decimal column"),
        ),
        Arc::new(Float64Array::from(vec![
            Some(0.5),
            Some(1.5),
            None,
            Some(-1.0),
            Some(0.0),
            Some(2.5),
        ])),
        Arc::new(Date32Array::from(vec![
            Some(19_723),
            Some(19_724),
            None,
            Some(19_000),
            Some(20_000),
            Some(0),
        ])),
        Arc::new(arrow_array::TimestampMillisecondArray::from(vec![
            Some(1_700_000_000_000_i64),
            None,
            Some(1_700_000_100_000),
            Some(1_600_000_000_000),
            Some(1_800_000_000_000),
            Some(0),
        ])),
        Arc::new(BooleanArray::from(vec![
            Some(true),
            Some(false),
            None,
            Some(true),
            Some(false),
            Some(true),
        ])),
    ];
    RecordBatch::try_new(
        yggdryl::arrow::schema_from_field(&schema()).expect("an Arrow schema"),
        arrays,
    )
    .expect("a six-row corpus")
}

/// The predicates both sides answer, in this grammar's spelling.
///
/// Each is paired with the PyArrow expression that asks the same question, so
/// the driver never has to translate one grammar into another - a translation
/// is exactly the thing that could hide a disagreement.
const PREDICATES: &[(&str, &str)] = &[
    ("venue = 'XNAS'", "pc.field('venue') == 'XNAS'"),
    ("venue <> 'XNAS'", "pc.field('venue') != 'XNAS'"),
    ("venue IS NULL", "pc.field('venue').is_null()"),
    ("venue IS NOT NULL", "pc.field('venue').is_valid()"),
    (
        "venue IN ('XNAS', 'XLON')",
        "pc.field('venue').isin(['XNAS', 'XLON'])",
    ),
    ("id > 2", "pc.field('id') > 2"),
    ("id >= 2", "pc.field('id') >= 2"),
    ("id < 2", "pc.field('id') < 2"),
    ("id <= 2", "pc.field('id') <= 2"),
    ("id = 2", "pc.field('id') == 2"),
    (
        "id BETWEEN 1 AND 3",
        "(pc.field('id') >= 1) & (pc.field('id') <= 3)",
    ),
    (
        "price > 10.00",
        "pc.field('price') > decimal.Decimal('10.00')",
    ),
    (
        "price <= 10.50",
        "pc.field('price') <= decimal.Decimal('10.50')",
    ),
    ("ratio > 0", "pc.field('ratio') > 0.0"),
    ("ratio < 0", "pc.field('ratio') < 0.0"),
    (
        "day > DATE '2024-01-01'",
        "pc.field('day') > datetime.date(2024, 1, 1)",
    ),
    (
        "ts >= TIMESTAMP '2023-11-14T22:13:20'",
        "pc.field('ts') >= datetime.datetime(2023, 11, 14, 22, 13, 20)",
    ),
    ("flag", "pc.field('flag')"),
    ("NOT flag", "~pc.field('flag')"),
    (
        "venue = 'XNAS' AND id > 1",
        "(pc.field('venue') == 'XNAS') & (pc.field('id') > 1)",
    ),
    (
        "venue = 'XNAS' OR id = 3",
        "(pc.field('venue') == 'XNAS') | (pc.field('id') == 3)",
    ),
    ("NOT (venue = 'XNAS')", "~(pc.field('venue') == 'XNAS')"),
    (
        "venue IS NULL OR id > 2",
        "pc.field('venue').is_null() | (pc.field('id') > 2)",
    ),
];

/// The pruning questions both engines answer, in each one's own grammar.
///
/// The statistics are fixed here and repeated in the driver, because they are
/// what a *data file* would carry - and the point is that two engines given the
/// same numbers reach the same decision.
const PRUNING: &[(&str, &str)] = &[
    ("id > 20", "id > 20"),
    ("id > 5", "id > 5"),
    ("id < 1", "id < 1"),
    ("id = 3", "id == 3"),
    ("id = 99", "id == 99"),
    ("venue = 'XNAS'", "venue == 'XNAS'"),
    ("venue = 'AAAA'", "venue == 'AAAA'"),
    ("venue IS NULL", "venue is null"),
    ("venue IS NOT NULL", "venue is not null"),
    ("id IS NULL", "id is null"),
    ("id >= 1 AND id <= 10", "id >= 1 and id <= 10"),
    ("id > 10", "id > 10"),
    ("id IN (3, 99)", "id in (3, 99)"),
    ("id IN (98, 99)", "id in (98, 99)"),
    // PyIceberg's *visitor* answers a prefix test, but its text parser has no
    // spelling for one, so these two name the constructor instead - which is
    // still unmodified PyIceberg deciding, just reached a different way.
    (
        "venue LIKE 'XN%'",
        "py:StartsWith(Reference('venue'), literal('XN'))",
    ),
    (
        "venue LIKE 'ZZ%'",
        "py:StartsWith(Reference('venue'), literal('ZZ'))",
    ),
];

/// The statistics of the one data file the pruning half asks about.
struct DataFileStats;

impl yggdryl::expressions::StatsSource for DataFileStats {
    fn stats(
        &self,
        column: &yggdryl::expressions::BoundColumn,
    ) -> Option<yggdryl::expressions::ColumnStats> {
        use yggdryl::Value;
        use yggdryl::expressions::ColumnStats;
        match column.name() {
            "id" => Some(ColumnStats {
                lower: Some(Value::I64(1)),
                upper: Some(Value::I64(10)),
                null_count: Some(1),
                value_count: Some(6),
            }),
            "venue" => Some(ColumnStats {
                lower: Some(Value::from("XLON")),
                upper: Some(Value::from("XNYS")),
                null_count: Some(1),
                value_count: Some(6),
            }),
            _ => None,
        }
    }
}

#[test]
fn the_pruning_decisions_are_left_for_an_external_reader() {
    let target = interop();
    if std::fs::create_dir_all(&target).is_err() {
        println!("SKIPPED: the interop directory cannot be created");
        return;
    }
    // The two columns a pruning engine is asked about, with the field ids a
    // manifest keys its statistics by.
    let schema = DataType::from_fields([
        DataType::Int64
            .nullable_field("id")
            .with_parquet_field_id(1),
        DataType::Utf8
            .nullable_field("venue")
            .with_parquet_field_id(2),
    ])
    .expect("a two-column struct")
    .required_field("row");

    let mut lines = Vec::with_capacity(PRUNING.len());
    for (text, pyiceberg) in PRUNING {
        let predicate = text
            .parse::<Expr>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .into_predicate()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let possible = predicate.evaluate_stats(&DataFileStats).is_possible();
        lines.push(format!(
            "  {{\"expression\": {}, \"pyiceberg\": {}, \"possible\": {possible}}}",
            quote(text),
            quote(pyiceberg)
        ));
    }
    let json = format!("[\n{}\n]\n", lines.join(",\n"));
    std::fs::write(target.join("pruning.json"), json).expect("the decisions are written");
    println!(
        "expression-interop: wrote {} pruning decisions",
        PRUNING.len()
    );
}

#[test]
fn the_corpus_and_this_engine_s_answers_are_left_for_an_external_reader() {
    let target = interop();
    if std::fs::create_dir_all(&target).is_err() {
        println!("SKIPPED: the interop directory cannot be created");
        return;
    }
    let schema = schema();
    let batch = corpus();

    // The corpus as one Parquet file, with a small row-group size so the
    // external side sees the same multi-group shape the pruning half uses.
    let path = target.join("corpus.parquet");
    let _ = std::fs::remove_file(&path);
    let mut handle = File::new(&path).expect("a local file");
    let options = yggdryl::generic::RecordOptions::Parquet(
        yggdryl::parquet::ParquetOptions::new().with_max_row_group_size(2),
    );
    handle
        .write_arrow_batch_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
            &options,
        )
        .expect("the corpus is written");
    handle.close().expect("the file is published");

    // What this engine answers, per predicate, as the row indexes it selects.
    let mut answers = Vec::with_capacity(PREDICATES.len());
    for (text, pyarrow) in PREDICATES {
        let predicate = text
            .parse::<Expr>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .into_predicate()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let mask = predicate
            .mask(&batch)
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let selected: Vec<usize> = (0..mask.len())
            .filter(|row| !arrow_array::Array::is_null(&mask, *row) && mask.value(*row))
            .collect();
        answers.push((*text, *pyarrow, selected));
    }

    let report = target.join("answers.json");
    let json = render(&answers);
    std::fs::write(&report, json).expect("the answers are written");

    println!(
        "expression-interop: wrote {} predicates over {} rows",
        answers.len(),
        batch.num_rows()
    );
}

/// Render the answers as the JSON the driver reads.
///
/// Hand-rolled rather than through a serializer, because the shape is three
/// fields and pulling a dependency into a test to write them would be worse.
fn render(answers: &[(&str, &str, Vec<usize>)]) -> String {
    let mut json = String::from("[\n");
    for (position, (text, pyarrow, selected)) in answers.iter().enumerate() {
        if position > 0 {
            json.push_str(",\n");
        }
        let rows = selected
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        json.push_str(&format!(
            "  {{\"expression\": {}, \"pyarrow\": {}, \"rows\": [{rows}]}}",
            quote(text),
            quote(pyarrow)
        ));
    }
    json.push_str("\n]\n");
    json
}

/// Quote one string as JSON.
fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}
