//! The edge cases this module is built to get right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a free attribute
//! never costs a backend call, and a pruning decision never loses a row.

use std::cell::Cell;
use std::hash::Hash;
use std::sync::Arc;

use super::{Bound, Bounds, ColumnBounds, Expression, Residual, Selector, Statement};
use crate::{DataType, Field, MediaType, Result, Scalar, TimeUnit, Timezone, Url};

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Every spelling the grammar accepts, one of each shape.
const CORPUS: [&str; 30] = [
    "ccy = 'EUR' and price > 100",
    "a or b and c",
    "(a or b) and c",
    "not a",
    "x is null",
    "x is not null",
    "x is distinct from y",
    "x is not distinct from y",
    "x in (1, 2, 3)",
    "x not in (1, 2)",
    "x between 1 and 10",
    "x not between 1 and 10",
    "name like 'a%' escape '\\'",
    "name ilike 'A%'",
    "name not like 'a%'",
    "path glob '**/*.parquet'",
    "&holder.size > 0",
    "&holder.partition['year'] = '2024'",
    "&holder.name like 'part-%'",
    "lower(name) = 'x'",
    "coalesce(a, b, 0) > 1",
    "cast(x as int32) = 1",
    "try_cast(x as decimal128(9,2)) > decimal128(9,2) '1.50'",
    "case when a then 1 else 2 end",
    "struct(1 as a, 'b' as b)",
    "[1, 2, 3]",
    "{'a': 1}",
    "trade.legs[0]['ccy'] = 'EUR'",
    "-x + 3 * 2 - 1",
    ":since <= ts",
];

#[test]
fn text_round_trips() {
    for text in CORPUS {
        let parsed: Expression = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let printed = parsed.to_string();
        let again: Expression = printed
            .parse()
            .unwrap_or_else(|error| panic!("{printed}: {error}"));
        assert_eq!(parsed, again, "{text} printed as {printed}");
    }
}

#[test]
fn expressions_and_statements_have_core_total_order_and_stable_hash() {
    fn assert_value_traits<T: Clone + Eq + Hash + Ord>() {}
    assert_value_traits::<ColumnBounds>();
    assert_value_traits::<Bounds>();
    assert_value_traits::<Residual>();

    let first: Expression = "a = 1".parse().unwrap();
    let equal: Expression = first.to_string().parse().unwrap();
    let later: Expression = "b = 1".parse().unwrap();
    assert_eq!(first.stable_hash(), equal.stable_hash());
    assert!(first < later);

    let first: Statement = "select a where a > 1".parse().unwrap();
    let equal: Statement = first.to_string().parse().unwrap();
    let later: Statement = "select b where b > 1".parse().unwrap();
    assert_eq!(first.stable_hash(), equal.stable_hash());
    assert!(first < later);
}

#[test]
fn quoted_names_survive_every_encapsulator() {
    for text in ["\"odd name\" = 1", "`odd name` = 1"] {
        let parsed: Expression = text.parse().unwrap();
        assert_eq!(parsed.columns(), vec!["odd name".to_owned()]);
        assert_eq!(parsed.to_string(), "\"odd name\" = 1");
    }
    // A doubled quote inside a quoted name is one quote, as SQL spells it.
    let parsed: Expression = "\"say \"\"hi\"\"\" = 1".parse().unwrap();
    assert_eq!(parsed.columns(), vec!["say \"hi\"".to_owned()]);
    assert_eq!(parsed.to_string().parse::<Expression>().unwrap(), parsed);
}

#[test]
fn statements_round_trip() {
    for text in [
        "select *",
        "select a, b as c where a > 1 order by b desc nulls first limit 10",
        "select lower(name) as name where name is not null",
        "select a order by a asc nulls last",
    ] {
        let parsed: Statement = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let printed = parsed.to_string();
        let again: Statement = printed
            .parse()
            .unwrap_or_else(|error| panic!("{printed}: {error}"));
        assert_eq!(parsed, again, "{text} printed as {printed}");
    }
}

#[test]
fn documents_round_trip() {
    for text in CORPUS {
        let parsed: Expression = text.parse().unwrap();
        let document = parsed.clone().into_json().unwrap();
        assert_eq!(Expression::from_json(&document).unwrap(), parsed, "{text}");
    }
    let statement: Statement = "select a as b where a > 1 limit 3".parse().unwrap();
    let document = statement.clone().into_json().unwrap();
    assert_eq!(Statement::from_json(&document).unwrap(), statement);
}

#[test]
fn a_parse_failure_names_where_it_stopped() {
    let error = "a = ".parse::<Expression>().unwrap_err();
    assert!(
        format!("{error}").contains("at byte 4"),
        "expected a byte position, got {error}"
    );
    let error = "a === 1".parse::<Expression>().unwrap_err();
    assert!(format!("{error}").contains("at byte "), "{error}");
    let error = "nosuchfn(a)".parse::<Expression>().unwrap_err();
    assert!(format!("{error}").contains("lower"), "{error}");
    let error = "&holder.nosuch".parse::<Expression>().unwrap_err();
    assert!(format!("{error}").contains("partition"), "{error}");
    let error = "a in ()".parse::<Expression>().unwrap_err();
    assert!(format!("{error}").contains("at least one"), "{error}");
}

#[test]
fn nesting_past_the_limit_is_refused_not_crashed() {
    let deep = format!(
        "{}a{}",
        "(".repeat(super::RECURSION_LIMIT + 8),
        ")".repeat(super::RECURSION_LIMIT + 8)
    );
    let error = deep.parse::<Expression>().unwrap_err();
    assert!(format!("{error}").contains("hard limit"), "{error}");
}

#[test]
fn a_pattern_that_changes_per_row_is_refused_at_bind() {
    let schema = rows_schema();
    let error = "s like s".parse::<Expression>().unwrap().bind(&schema);
    let message = format!("{}", error.unwrap_err());
    assert!(message.contains("constant"), "{message}");
}

// ---------------------------------------------------------------------------
// The shared fixture
// ---------------------------------------------------------------------------

/// A schema that covers one column of every family a comparison can meet.
fn rows_schema() -> Field {
    Field::new(
        "rows",
        DataType::from_fields([
            Field::new("i", DataType::Int64, true),
            Field::new("f", DataType::Float64, true),
            Field::new("d", DataType::decimal128(9, 2).unwrap(), true),
            Field::new("s", DataType::Utf8, true),
            Field::new("b", DataType::Boolean, true),
            Field::new(
                "t",
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: Timezone::UTC,
                },
                true,
            ),
            Field::new("n", DataType::Int32, true).with_partition(true),
            Field::new(
                "nested",
                DataType::from_fields([Field::new("leg", DataType::Utf8, true)]).unwrap(),
                true,
            ),
            // Temporal text, so a cast into and out of a temporal is one of
            // the pairs the two tiers are compared on.
            Field::new("clock", DataType::Utf8, true),
        ])
        .unwrap(),
        false,
    )
}

/// Rows chosen so every operator meets a null, a `nan`, and a boundary.
fn rows() -> Vec<Scalar> {
    let stamp =
        |micros: i64| Scalar::datetime64(micros, TimeUnit::Microsecond, Timezone::UTC).unwrap();
    let nested =
        |leg: Option<&str>| Scalar::from_sequence([leg.map_or(Scalar::Null, Scalar::from)]);
    vec![
        Scalar::from_sequence([
            Scalar::from(1),
            Scalar::from(1.5_f64),
            Scalar::d128(150, 2),
            Scalar::from("alpha"),
            Scalar::from(true),
            stamp(1_700_000_000_000_000),
            Scalar::from(2024),
            nested(Some("EUR")),
            Scalar::from("10:23:45"),
        ]),
        Scalar::from_sequence([
            Scalar::from(-3),
            Scalar::from(f64::NAN),
            Scalar::d128(-25, 2),
            Scalar::from("beta"),
            Scalar::from(false),
            stamp(0),
            Scalar::from(2024),
            nested(None),
            Scalar::from("25:30:00"),
        ]),
        Scalar::from_sequence([
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::from(2024),
            Scalar::Null,
            Scalar::Null,
        ]),
        Scalar::from_sequence([
            Scalar::from(100),
            Scalar::from(f64::INFINITY),
            Scalar::d128(10_000, 2),
            Scalar::from("Alpha"),
            Scalar::Null,
            stamp(-1_000_000),
            Scalar::from(2023),
            nested(Some("USD")),
            Scalar::from("99:59:59"),
        ]),
        Scalar::from_sequence([
            Scalar::from(0),
            Scalar::from(0.0_f64),
            Scalar::d128(0, 2),
            Scalar::from(""),
            Scalar::from(true),
            stamp(1_700_000_000_000_001),
            Scalar::from(2025),
            nested(Some("eur")),
            Scalar::from("00:00:00.500"),
        ]),
    ]
}

/// The expressions the two tiers are compared on, all evaluable per row.
const AGREEMENT: [&str; 26] = [
    "i = 1",
    "i <> 1",
    "i < 0",
    "i >= 100",
    "i is null",
    "i is not null",
    "i is distinct from 1",
    "i is not distinct from null",
    "f = f",
    "f > 1.0",
    "f is null",
    "d > decimal128(9,2) '1.00'",
    "d <= 0",
    "s = 'alpha'",
    "s like 'a%'",
    "s ilike 'A%'",
    "b",
    "t > timestamp(microsecond,'UTC') '2023-11-14T22:13:20Z'",
    "i in (1, 100)",
    "i between 0 and 100",
    "i = 1 or s = 'beta'",
    "not (i = 1) and s is not null",
    // Text entering a temporal and a temporal leaving as text: the vectorized
    // tier reads and spells with the code the row tier reads and spells with,
    // a zone name and an hour past the end of the day included.
    "cast(clock as time64(microsecond))",
    "cast(clock as duration64(millisecond))",
    "cast(t as string)",
    "try_cast(clock as time32(second))",
];

#[test]
fn scalar_and_vectorized_agree() {
    let schema = rows_schema();
    let rows = rows();
    let batch = batch_of(&schema, &rows);
    for text in AGREEMENT {
        let bound = text
            .parse::<Expression>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let vectorized = bound.evaluate(&batch).unwrap();
        for (position, row) in rows.iter().enumerate() {
            let scalar = bound.eval(row).unwrap();
            let held = crate::arrow::value::value_from_array(
                bound.field().dtype(),
                vectorized.as_ref(),
                position,
            )
            .unwrap();
            assert_eq!(
                scalar, held,
                "{text} disagreed on row {position}: scalar {scalar:?}, vectorized {held:?}"
            );
        }
    }
}

#[test]
fn scalar_arithmetic_propagates_checked_failures() {
    let bound = "n / i"
        .parse::<Expression>()
        .unwrap()
        .bind(&rows_schema())
        .unwrap();
    assert!(matches!(
        bound.eval(&rows()[4]),
        Err(crate::Error::DivisionByZero { .. })
    ));
    assert_eq!(bound.eval(&rows()[2]).unwrap(), Scalar::Null);

    let schema = Field::new(
        "rows",
        DataType::from_fields([Field::new("small", DataType::Int8, false)]).unwrap(),
        false,
    );
    let negated = "-small"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert!(matches!(
        negated.eval(&Scalar::from_sequence([Scalar::from(i8::MIN)])),
        Err(crate::Error::ArithmeticOverflow {
            operation: "negation",
            ..
        })
    ));
}

#[test]
fn projections_agree_between_the_tiers() {
    let schema = rows_schema();
    let rows = rows();
    let batch = batch_of(&schema, &rows);
    for text in [
        "lower(s)",
        "length(s)",
        "i + 1",
        "d * decimal128(9,2) '2.00'",
        "nested.leg",
        "coalesce(s, 'none')",
        "case when i > 0 then 'up' else 'down' end",
        "year(t)",
        "concat(s, '!')",
        "substring(s, 2, 3)",
    ] {
        let bound = text
            .parse::<Expression>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
            .bind(&schema)
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let vectorized = bound.evaluate(&batch).unwrap();
        for (position, row) in rows.iter().enumerate() {
            let scalar = bound.eval(row).unwrap();
            let held = crate::arrow::value::value_from_array(
                bound.field().dtype(),
                vectorized.as_ref(),
                position,
            )
            .unwrap();
            assert_eq!(scalar, held, "{text} disagreed on row {position}");
        }
    }
}

fn batch_of(schema: &Field, rows: &[Scalar]) -> arrow_array::RecordBatch {
    let arrow_schema = crate::arrow::arrow_schema_from_field(schema).unwrap();
    let columns = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let values: Vec<&Scalar> = rows
                .iter()
                .map(|row| &row.as_sequence().unwrap()[index])
                .collect();
            crate::arrow::value::array_from_values(field, &values).unwrap()
        })
        .collect();
    arrow_array::RecordBatch::try_new(arrow_schema, columns).unwrap()
}

// ---------------------------------------------------------------------------
// Zero copy
// ---------------------------------------------------------------------------

#[test]
fn a_mask_that_keeps_everything_keeps_the_batch_itself() {
    let schema = rows_schema();
    let batch = batch_of(&schema, &rows());
    let bound = "n = 2024 or n <> 2024 or n is null"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let filtered = bound.filter(&batch).unwrap();
    assert_eq!(filtered.num_rows(), batch.num_rows());
    for (left, right) in batch.columns().iter().zip(filtered.columns()) {
        assert!(
            Arc::ptr_eq(left, right),
            "a filter that dropped nothing still moved a buffer"
        );
    }
}

#[test]
fn a_projection_reorders_without_touching_a_buffer() {
    let schema = rows_schema();
    let batch = batch_of(&schema, &rows());
    let statement: Statement = "select s, i".parse().unwrap();
    let bound = statement.bind(&schema).unwrap();
    let projected = bound.project(&batch).unwrap();
    assert_eq!(projected.num_columns(), 2);
    assert!(Arc::ptr_eq(projected.column(0), batch.column(3)));
    assert!(Arc::ptr_eq(projected.column(1), batch.column(0)));
}

#[test]
fn a_reader_filters_and_projects_in_one_pass() {
    let schema = rows_schema();
    let batch = batch_of(&schema, &rows());
    let arrow_schema = batch.schema();
    let statement: Statement = "select i where i is not null limit 2".parse().unwrap();
    let reader = statement
        .bind(&schema)
        .unwrap()
        .project_reader(crate::arrow::batch_reader(arrow_schema, [batch]))
        .unwrap();
    let rows: usize = reader.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2);
}

// ---------------------------------------------------------------------------
// Attributes and their cost
// ---------------------------------------------------------------------------

/// A handle that counts every stat it is asked for.
struct Counting {
    url: Url,
    media_type: MediaType,
    stats: Cell<usize>,
}

impl Counting {
    fn new(url: &str) -> Self {
        Self {
            url: Url::from_str(url).unwrap(),
            media_type: MediaType::default(),
            stats: Cell::new(0),
        }
    }
}

impl crate::IOMedia for Counting {
    crate::impl_default_iomedia!();
}

impl crate::IOBase for Counting {
    fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn pwrite(&mut self, _offset: u64, _bytes: &[u8]) -> Result<usize> {
        Ok(0)
    }

    fn size(&self) -> u64 {
        self.stats.set(self.stats.get() + 1);
        4_096
    }

    fn capacity(&self) -> u64 {
        4_096
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Ok(())
    }

    fn truncate(&mut self, _size: u64) -> Result<()> {
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.media_type = media_type;
    }
}

#[test]
fn a_free_attribute_answers_without_a_single_stat() {
    let schema = rows_schema();
    // Written stat-first on purpose: bind is what puts the free test in front.
    let bound = "&holder.size > 0 and &holder.partition['year'] = '2023'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let handle = Counting::new("file:///lake/year=2024/part-0.parquet");
    assert!(!bound.matches_holder(&super::Handle(&handle)).unwrap());
    assert_eq!(
        handle.stats.get(),
        0,
        "a predicate settled by the path still cost a stat"
    );

    // A holder the free test does not rule out pays for the stat, once.
    let matching = Counting::new("file:///lake/year=2023/part-0.parquet");
    assert!(bound.matches_holder(&super::Handle(&matching)).unwrap());
    assert_eq!(matching.stats.get(), 1);
}

#[test]
fn a_row_predicate_rules_no_holder_out() {
    let schema = rows_schema();
    let bound = "i > 1000000"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let handle = Counting::new("file:///lake/part-0.parquet");
    assert!(
        bound.matches_holder(&super::Handle(&handle)).unwrap(),
        "a listing filter may never discard a file it has not read"
    );
}

#[test]
fn every_selector_declares_a_cost_and_a_type() {
    for selector in Selector::ALL {
        let field = selector.field();
        assert!(field.is_nullable());
        assert_eq!(field.name(), format!("&holder.{selector}"));
        // A free selector is answerable from a URL alone; a stat one is not.
        let url = Url::from_str("file:///lake/year=2024/part-0.parquet").unwrap();
        let answered = selector.read_url(&url);
        assert_eq!(
            answered.is_null(),
            matches!(selector.cost(), super::Cost::Stat),
            "{selector} disagreed with its own cost class"
        );
    }
    let partition = Selector::Partition("year".into());
    assert_eq!(partition.cost(), super::Cost::Free);
    let url = Url::from_str("file:///lake/year=2024/part-0.parquet").unwrap();
    assert_eq!(partition.read_url(&url), Scalar::from("2024"));
}

// ---------------------------------------------------------------------------
// Pushdown
// ---------------------------------------------------------------------------

#[test]
fn pruning_never_loses_a_row() {
    let schema = rows_schema();
    let rows = rows();
    let bounds = bounds_of(&schema, &rows);
    for text in AGREEMENT {
        let bound: Bound = text.parse::<Expression>().unwrap().bind(&schema).unwrap();
        if !bound.is_predicate() {
            continue;
        }
        let any_matches = rows.iter().any(|row| bound.matches(row).unwrap());
        if any_matches {
            assert!(
                bound.statistics_prune(&bounds),
                "{text} pruned a container that holds a matching row"
            );
        }
    }
}

#[test]
fn pruning_actually_prunes_what_it_can_prove() {
    let schema = rows_schema();
    let bounds = bounds_of(&schema, &rows());
    for text in ["i > 1000", "i < -1000", "n = 1999", "s > 'zzz'"] {
        let bound = text.parse::<Expression>().unwrap().bind(&schema).unwrap();
        assert!(
            !bound.statistics_prune(&bounds),
            "{text} should have been provably empty"
        );
    }
}

#[test]
fn a_partition_path_is_the_tightest_statistic_there_is() {
    let schema = rows_schema();
    let partitions = vec![("n".to_owned(), "2024".to_owned())];
    let bounds = Bounds::from_partitions(&schema, &partitions);
    let keep = "n = 2024"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let skip = "n = 2023"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert!(keep.statistics_prune(&bounds));
    assert!(!skip.statistics_prune(&bounds));
}

#[test]
fn a_split_conjoins_back_to_what_it_split() {
    let schema = rows_schema();
    let bound = "n = 2024 and i > 1 and s like 'a%'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let residual = bound.partition_split();
    assert_eq!(residual.answerable().to_string(), "n = int32 '2024'");
    assert!(!residual.is_complete());
    let rejoined = residual
        .answerable()
        .clone()
        .and(residual.remaining().clone());
    let mut left = rejoined.conjuncts();
    let mut right = bound.expression().conjuncts();
    left.sort();
    right.sort();
    assert_eq!(left, right);
}

/// The statistics the fixture rows actually have, computed rather than guessed.
fn bounds_of(schema: &Field, rows: &[Scalar]) -> Bounds {
    let mut bounds = Bounds::new(Some(rows.len() as u64));
    for (index, field) in schema.fields().iter().enumerate() {
        let mut minimum: Option<Scalar> = None;
        let mut maximum: Option<Scalar> = None;
        let mut nulls = 0_u64;
        for row in rows {
            let value = &row.as_sequence().unwrap()[index];
            if value.is_null() {
                nulls += 1;
                continue;
            }
            let ordered = |held: &Option<Scalar>, keep_greater: bool| match held {
                None => Some(value.clone()),
                Some(held) => match super::eval::order(field.dtype(), value, held) {
                    Some(std::cmp::Ordering::Greater) if keep_greater => Some(value.clone()),
                    Some(std::cmp::Ordering::Less) if !keep_greater => Some(value.clone()),
                    _ => None,
                },
            };
            if let Some(next) = ordered(&minimum, false) {
                minimum = Some(next);
            }
            if let Some(next) = ordered(&maximum, true) {
                maximum = Some(next);
            }
        }
        bounds = bounds.with_column(field.name(), minimum, maximum, Some(nulls));
    }
    bounds
}

// ---------------------------------------------------------------------------
// Bind
// ---------------------------------------------------------------------------

#[test]
fn substring_takes_the_window_the_standard_names() {
    let schema = rows_schema();
    for (text, expected) in [
        ("substring(s, 1, 5)", "alpha"),
        // The window starts before the string, and the part before it is not
        // there to take: four characters, not five.
        ("substring(s, 0, 5)", "alph"),
        ("substring(s, 2)", "lpha"),
        ("substring(s, -3, 5)", "pha"),
        ("substring(s, 9, 4)", ""),
    ] {
        let bound = text.parse::<Expression>().unwrap().bind(&schema).unwrap();
        assert_eq!(
            bound.eval(&rows()[0]).unwrap(),
            Scalar::from(expected),
            "{text}"
        );
    }
    let bound = "substring(s, 1, -1)"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert!(bound.eval(&rows()[0]).is_err());
}

#[test]
fn a_pattern_with_no_wildcard_becomes_an_equality() {
    let schema = rows_schema();
    let bound = "s like 'alpha'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.expression().to_string(), "s = 'alpha'");
    assert!(bound.matches(&rows()[0]).unwrap());
    // An escaped wildcard is a literal, so it folds too.
    let escaped = "s like 'a!%b' escape '!'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(escaped.expression().to_string(), "s = 'a%b'");
}

#[test]
fn a_column_named_twice_in_two_cases_is_ambiguous() {
    let schema = Field::new(
        "rows",
        DataType::from_fields([
            Field::new("Value", DataType::Int64, true),
            Field::new("value", DataType::Int64, true),
        ])
        .unwrap(),
        false,
    );
    let error = "value = 1".parse::<Expression>().unwrap().bind(&schema);
    let message = format!("{}", error.unwrap_err());
    assert!(message.contains("one column"), "{message}");
    assert!(message.contains("quote the one meant"), "{message}");
}

#[test]
fn an_exact_quotient_keeps_room_to_be_a_quotient() {
    let schema = rows_schema();
    let bound = "d / decimal128(9,2) '3.00'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(
        bound.field().dtype().to_string(),
        "decimal128(15,6)",
        "a quotient at the operands' own scale would be a rounding"
    );
    // 1.50 / 3.00 is exactly 0.5, and it stays exact.
    assert_eq!(bound.eval(&rows()[0]).unwrap(), Scalar::d128(500_000, 6));
}

#[test]
fn binds_and_evaluates_rows() {
    let schema = Field::new(
        "trades",
        DataType::from_fields([
            Field::new("ccy", DataType::Utf8, true),
            Field::new("price", DataType::decimal128(9, 2).unwrap(), true),
            Field::new("size", DataType::Int32, true),
        ])
        .unwrap(),
        false,
    );
    let bound = "ccy = 'EUR' and price > 100 and size is not null"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert!(bound.is_predicate());
    assert_eq!(bound.column_names(), vec!["ccy", "price", "size"]);

    let row = |ccy: &str, price: i128, size: Option<i32>| {
        Scalar::from_sequence([
            Scalar::from(ccy),
            Scalar::d128(price, 2),
            size.map_or(Scalar::Null, Scalar::from),
        ])
    };
    assert!(bound.matches(&row("EUR", 15_000, Some(5))).unwrap());
    assert!(!bound.matches(&row("USD", 15_000, Some(5))).unwrap());
    assert!(!bound.matches(&row("EUR", 5_000, Some(5))).unwrap());
    assert!(!bound.matches(&row("EUR", 15_000, None)).unwrap());
}

#[test]
fn a_struct_expression_produces_and_reprints_a_row_sequence() {
    let schema = rows_schema();
    let bound = "struct(1 as id, 'XNAS' as venue)"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let expected = Scalar::from_sequence([Scalar::from(1), Scalar::from("XNAS")]);
    assert_eq!(bound.eval(&rows()[0]).unwrap(), expected);

    // Constant folding retains the datatype on TypedScalar rather than on the
    // row. Display must use that schema to reconstruct the named expression.
    let printed = bound.expression().to_string();
    assert!(printed.contains("struct("), "{printed}");
    let reparsed = printed
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(reparsed.eval(&rows()[0]).unwrap(), expected);
}

#[test]
fn unknown_is_not_true() {
    let schema = rows_schema();
    let bound = "i > 1"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let row = &rows()[2];
    assert_eq!(bound.eval(row).unwrap(), Scalar::Null);
    assert!(!bound.matches(row).unwrap());
}

#[test]
fn a_literal_is_converted_once_into_the_column_it_meets() {
    let schema = rows_schema();
    let bound = "d > 100"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    // The bound expression prints the literal in the column's own type, which
    // is how a caller sees that the comparison is exact rather than floating.
    assert_eq!(
        bound.expression().to_string(),
        "d > decimal128(9,2) '100.00'"
    );
}

#[test]
fn a_constant_subtree_is_folded_by_evaluating_it() {
    let schema = rows_schema();
    let bound = "i > 2 * 3 + 1"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.expression().to_string(), "i > 7");
}

#[test]
fn parameters_are_supplied_at_bind_and_never_again() {
    let schema = rows_schema();
    let expression: Expression = "i >= :floor".parse().unwrap();
    assert_eq!(expression.parameters(), vec!["floor".to_owned()]);
    assert!(expression.bind(&schema).is_err());
    let bound = expression
        .bind_with(&schema, &[("floor", Scalar::from(100))])
        .unwrap();
    assert_eq!(bound.expression().to_string(), "i >= 100");
    assert!(bound.matches(&rows()[3]).unwrap());
}

#[test]
fn an_unknown_column_names_the_ones_there_are() {
    let schema = rows_schema();
    let error = "nope = 1".parse::<Expression>().unwrap().bind(&schema);
    let message = format!("{}", error.unwrap_err());
    assert!(message.contains("nope"), "{message}");
    assert!(message.contains('i'), "{message}");
}

#[test]
fn two_operands_with_no_common_type_are_refused() {
    let schema = rows_schema();
    let error = "s > 1".parse::<Expression>().unwrap().bind(&schema);
    let message = format!("{}", error.unwrap_err());
    assert!(message.contains("comparable"), "{message}");
}

#[test]
fn cheapest_first_is_stable_when_costs_tie() {
    let schema = rows_schema();
    let bound = "s = 'a' and i = 1"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.expression().to_string(), "s = 'a' and i = 1");
}
