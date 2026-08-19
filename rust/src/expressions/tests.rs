//! Edge cases of the expression module itself.
//!
//! The broad matrices - every datatype, every operator, row against vectorized
//! parity - live in `rust/tests/expressions.rs`; what is here is what only the
//! module's own internals can reach, plus the handful of properties the rest of
//! the crate depends on being true.

use std::sync::Arc;

use super::{Accessor, Bound, Certainty, ColumnStats, Expr, Selection, StatsSource, col, lit};
use crate::expressions::BoundColumn;
use crate::{DataType, Field, Result, Value};

/// The schema most of these cases bind against.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Utf8.nullable_field("venue"),
        DataType::Decimal128 {
            precision: 10,
            scale: 2,
        }
        .nullable_field("price"),
        DataType::Int32.required_field("n"),
        DataType::Int64.nullable_field("id"),
    ])
    .expect("a struct of four columns")
    .required_field("row")
}

/// One row in field order, which is what a record is.
fn row(values: [Value; 4]) -> Value {
    Value::record(schema().data_type().clone(), values).expect("a row of four values")
}

/// A statistics source that answers from a fixed table, by column name.
struct Table(Vec<(&'static str, ColumnStats)>);

impl StatsSource for Table {
    fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
        self.0
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(column.name()))
            .map(|(_, stats)| stats.clone())
    }
}

#[test]
fn display_round_trips_through_the_parser() -> Result<()> {
    // The property the whole grammar rests on: rendering, re-reading, and
    // rendering again is a fixed point.
    for text in [
        "venue = 'XNAS'",
        "price > 10.50",
        "n BETWEEN 1 AND 5",
        "venue IN ('XNAS', 'XNYS')",
        "venue IS NULL",
        "NOT venue IS NULL",
        "n + 1 * 2 > 3",
        "(n + 1) * 2 > 3",
        "n - (id - 1) > 0",
        "CAST(venue AS int64) = 1",
        "TRY_CAST(venue AS int64) = 1",
        "CASE WHEN n > 1 THEN 'big' ELSE 'small' END = 'big'",
        "lower(venue) = 'xnas'",
        "year(ts) = 2024",
        "venue LIKE 'X%'",
        "venue NOT LIKE 'X\\_%' ESCAPE '\\'",
        "venue ILIKE 'x%'",
        "\"total amount\" = 3",
        "a.b[0]['k'][1:3] = 1",
        "a[-1] = 1",
        "a[:3] = 1",
        "a[1:] = 1",
        "id = DATE '2024-01-01'",
        "id = TIMESTAMP '2024-01-01T00:00:00Z'",
        "id = TIME '12:30:00'",
        "id = INTERVAL 'PT1H'",
        "id = X'DEADBEEF'",
        "id = 1e0",
        "id = NAN",
        "id = -INFINITY",
        "id = -1",
        "n AS total",
    ] {
        let parsed: Expr = text.parse()?;
        let once = parsed.to_string();
        let twice: Expr = once.parse()?;
        assert_eq!(once, twice.to_string(), "round trip of {text:?}");
    }
    Ok(())
}

#[test]
fn a_double_quoted_token_is_an_identifier_and_a_single_quoted_one_is_text() -> Result<()> {
    let column: Expr = "\"venue\" = 'XNAS'".parse()?;
    assert!(matches!(column, Expr::Compare { ref left, .. } if left.as_column().is_some()));
    let strings: Expr = "'venue' = 'XNAS'".parse()?;
    // Two literals fold, and they are not equal, so the whole thing is FALSE.
    assert!(strings.simplify().is_always_false());
    Ok(())
}

#[test]
fn every_encapsulator_reads_and_one_spelling_writes() -> Result<()> {
    for text in [
        "\"total amount\" = 1",
        "`total amount` = 1",
        "[total amount] = 1",
    ] {
        let parsed: Expr = text.parse()?;
        assert_eq!(parsed.to_string(), "\"total amount\" = 1", "from {text:?}");
    }
    Ok(())
}

#[test]
fn a_doubled_closer_embeds_the_delimiter() -> Result<()> {
    assert_eq!(
        "\"say \"\"hi\"\" now\"".parse::<Expr>()?.to_string(),
        "\"say \"\"hi\"\" now\""
    );
    assert_eq!(
        "`back``tick`".parse::<Expr>()?.columns(),
        vec!["back`tick".to_owned()]
    );
    assert_eq!(
        "[bra]]cket]".parse::<Expr>()?.columns(),
        vec!["bra]cket".to_owned()]
    );
    Ok(())
}

#[test]
fn whitespace_inside_an_encapsulator_is_data() -> Result<()> {
    let parsed: Expr = "\"  a  \" = 1".parse()?;
    assert_eq!(parsed.columns(), vec!["  a  ".to_owned()]);
    assert_eq!(parsed.to_string(), "\"  a  \" = 1");
    Ok(())
}

#[test]
fn an_unterminated_delimiter_names_the_opener() {
    for (text, opener) in [
        ("a = \"unterminated", 4_usize),
        ("a = 'unterminated", 4),
        ("a = `unterminated", 4),
        ("`unterminated", 0),
    ] {
        let error = text.parse::<Expr>().expect_err("unterminated");
        let crate::Error::Parse { position, .. } = error else {
            panic!("expected a parse error, got {error}");
        };
        assert_eq!(position, opener, "for {text:?}");
    }
}

#[test]
fn a_bare_name_inside_a_subscript_names_both_readings() {
    let error = "a[b] = 1".parse::<Expr>().expect_err("ambiguous subscript");
    let message = error.to_string();
    assert!(message.contains("['b']"), "{message}");
    assert!(message.contains("[\"b\"]"), "{message}");
}

#[test]
fn a_bracket_in_primary_position_is_a_name_and_after_a_primary_is_a_subscript() -> Result<()> {
    assert_eq!(
        "[my col]".parse::<Expr>()?.columns(),
        vec!["my col".to_owned()]
    );
    let subscript: Expr = "a[0]".parse()?;
    let column = subscript.as_column().expect("a column path");
    assert_eq!(column.path(), &[Accessor::Index(0)]);
    // Whitespace never changes what a token is.
    assert_eq!("a [0]".parse::<Expr>()?, subscript);
    Ok(())
}

#[test]
fn comments_are_skipped_outside_and_inert_inside_a_name() -> Result<()> {
    assert_eq!("a = 1 -- trailing\n".parse::<Expr>()?.to_string(), "a = 1");
    assert_eq!("a /* mid */ = 1".parse::<Expr>()?.to_string(), "a = 1");
    assert_eq!(
        "\"a -- b\" = 1".parse::<Expr>()?.columns(),
        vec!["a -- b".to_owned()]
    );
    Ok(())
}

#[test]
fn adversarial_input_is_refused_with_a_byte_position() {
    for text in [
        "a = ",
        "a = 1)",
        "(a = 1",
        "a IN ()",
        "a BETWEEN 1 2",
        "nosuch(a)",
        "a::nosuchtype",
        "a = 1 b",
        "CASE END",
        "lower()",
        "substring(a)",
    ] {
        let error = text.parse::<Expr>().expect_err(text);
        assert!(
            matches!(error, crate::Error::Parse { .. }),
            "{text:?} gave {error}"
        );
    }
}

#[test]
fn nesting_past_the_limit_is_a_typed_error_not_a_stack_overflow() {
    let deep = format!(
        "{}a = 1{}",
        "(".repeat(super::RECURSION_LIMIT + 8),
        ")".repeat(super::RECURSION_LIMIT + 8)
    );
    let error = deep.parse::<Expr>().expect_err("over-deep expression");
    assert!(error.to_string().contains("hard limit"), "{error}");
}

#[test]
fn a_ten_thousand_element_in_list_is_refused_or_parsed_without_panic() -> Result<()> {
    let values = (0..10_000)
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    let text = format!("a IN ({})", values.join(", "));
    let parsed: Expr = text.parse()?;
    assert_eq!(parsed.columns(), vec!["a".to_owned()]);
    Ok(())
}

#[test]
fn a_fractional_literal_keeps_the_scale_it_was_written_with() -> Result<()> {
    let parsed: Expr = "a = 10.50".parse()?;
    let Expr::Compare { right, .. } = parsed else {
        panic!("expected a comparison");
    };
    assert_eq!(right.as_literal(), Some(&Value::Decimal(1050, 2)));
    Ok(())
}

#[test]
fn structural_equality_and_the_equality_builder_are_different_things() {
    let left = col("a");
    let right = col("a");
    // `==` compares two expressions.
    assert_eq!(left, right);
    // `.eq(..)` builds one.
    assert_eq!(col("a").eq(lit(3)).to_string(), "a = 3");
    // And a string is a literal, not a parsed predicate.
    assert_eq!(Expr::from("a > 1"), Expr::literal(Value::from("a > 1")));
    assert_eq!(
        "a > 1".parse::<Expr>().expect("parses").to_string(),
        "a > 1"
    );
}

#[test]
fn operators_build_what_the_constructors_build() -> Result<()> {
    assert_eq!(col("a") & col("b"), col("a").and(col("b")));
    assert_eq!(col("a") | col("b"), col("a").or(col("b")));
    assert_eq!(!col("a"), col("a").not());
    assert_eq!(col("a") + 1, col("a").arithmetic(super::ArithOp::Add, 1));
    assert_eq!(-col("a"), Expr::Neg(Arc::new(col("a"))));
    Ok(())
}

#[test]
fn simplification_never_folds_a_reflexive_comparison() -> Result<()> {
    // `a = a` is unknown when `a` is null, so it is not TRUE - the mistake
    // every optimizer makes once.
    assert_eq!("a = a".parse::<Expr>()?.simplify().to_string(), "a = a");
    assert_eq!("a <> a".parse::<Expr>()?.simplify().to_string(), "a <> a");
    Ok(())
}

#[test]
fn a_long_or_of_equalities_becomes_one_in_list() -> Result<()> {
    let wide: Expr = "a = 3 OR a = 1 OR a = 2 OR a = 1".parse()?;
    assert_eq!(wide.simplify().to_string(), "a IN (1, 2, 3)");
    Ok(())
}

#[test]
fn overlapping_comparisons_become_the_tightest_bound() -> Result<()> {
    assert_eq!(
        "a > 1 AND a > 3".parse::<Expr>()?.simplify().to_string(),
        "a > 3"
    );
    assert_eq!(
        "a < 9 AND a <= 4".parse::<Expr>()?.simplify().to_string(),
        "a <= 4"
    );
    assert_eq!(
        "a >= 2 AND a > 2".parse::<Expr>()?.simplify().to_string(),
        "a > 2"
    );
    Ok(())
}

#[test]
fn a_contradiction_declines_to_fold_where_a_null_is_possible() -> Result<()> {
    // Without a schema nothing proves the column non-nullable, so the pair
    // stays: `FALSE` would be wrong when `a` is null.
    let held: Expr = "a > 5 AND a < 3".parse()?;
    assert!(!held.simplify().is_always_false());

    // With a schema that proves it, the fold is sound and fires.
    let bound = "n > 5 AND n < 3".parse::<Expr>()?.bind(&schema())?;
    assert!(bound.is_always_false(), "{}", bound.to_expr());
    Ok(())
}

#[test]
fn between_lowers_and_not_pushes_to_the_leaves() -> Result<()> {
    assert_eq!(
        "a BETWEEN 1 AND 5".parse::<Expr>()?.simplify().to_string(),
        "a >= 1 AND a <= 5"
    );
    assert_eq!(
        "NOT (a = 1 AND b = 2)"
            .parse::<Expr>()?
            .simplify()
            .to_string(),
        "a <> 1 OR b <> 2"
    );
    Ok(())
}

#[test]
fn a_prefix_like_folds_to_a_prunable_prefix_test() -> Result<()> {
    let folded = "a LIKE 'XN%'".parse::<Expr>()?.simplify();
    assert!(matches!(folded, Expr::StartsWith { .. }), "{folded}");
    assert_eq!(folded.to_string(), "a LIKE 'XN%'");
    // A wildcard anywhere else keeps the general matcher.
    let held = "a LIKE 'X%S'".parse::<Expr>()?.simplify();
    assert!(matches!(held, Expr::Like { .. }), "{held}");
    Ok(())
}

#[test]
fn optimization_is_deterministic_and_idempotent() -> Result<()> {
    for text in [
        "a = 1 OR a = 2 OR a = 3",
        "a > 1 AND a > 3 AND b = 'x'",
        "NOT (a = 1 AND (b = 2 OR c = 3))",
        "a BETWEEN 1 AND 5 AND a IN (1, 2, 3)",
    ] {
        let once = text.parse::<Expr>()?.simplify();
        assert_eq!(
            once,
            text.parse::<Expr>()?.simplify(),
            "determinism, {text}"
        );
        assert_eq!(once.simplify(), once, "idempotence, {text}");
    }
    Ok(())
}

#[test]
fn a_literal_is_folded_into_the_columns_own_type_once() -> Result<()> {
    // Text against a decimal column compares two decimals after binding, not
    // a string and a decimal - which is the whole reason folding exists.
    let bound = "price > '10.5'".parse::<Expr>()?.bind(&schema())?;
    let Expr::Compare { right, .. } = bound.to_expr() else {
        panic!("expected a comparison, got {}", bound.to_expr());
    };
    assert_eq!(right.as_literal(), Some(&Value::Decimal(1050, 2)));
    Ok(())
}

#[test]
fn an_unknown_column_names_the_columns_the_schema_has() {
    let error = "prise > 1"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema())
        .expect_err("unknown column");
    let message = error.to_string();
    assert!(message.contains("prise"), "{message}");
    assert!(message.contains("price"), "{message}");
}

#[test]
fn the_tolerant_mode_absorbs_what_the_pair_vocabulary_has_always_absorbed() -> Result<()> {
    // A column the rows do not carry drops out rather than failing the read.
    let bound = "nosuch = 'x'".parse::<Expr>()?.bind_tolerant(&schema())?;
    assert!(
        bound
            .evaluate(&row([
                Value::from("XNAS"),
                Value::Decimal(100, 2),
                Value::I32(1),
                Value::I64(7),
            ]))?
            .is_null()
    );
    Ok(())
}

#[test]
fn three_valued_logic_holds_over_rows() -> Result<()> {
    let null_venue = row([
        Value::Null,
        Value::Decimal(100, 2),
        Value::I32(1),
        Value::I64(7),
    ]);
    let predicate = "venue <> 'XNAS'"
        .parse::<Expr>()?
        .bind(&schema())?
        .into_predicate()?;
    // Not false - unknown - and unknown does not keep a row.
    assert!(!predicate.matches(&null_venue)?);
    let asked = "venue IS NULL"
        .parse::<Expr>()?
        .bind(&schema())?
        .into_predicate()?;
    assert!(asked.matches(&null_venue)?);
    Ok(())
}

#[test]
fn accessors_read_the_same_way_the_documentation_says() -> Result<()> {
    let nested = DataType::from_fields([
        DataType::list(DataType::Int64.nullable_field("item")).nullable_field("tags"),
        DataType::Utf8.nullable_field("path"),
    ])?
    .required_field("row");
    let values = row_of(
        &nested,
        [
            Value::from_sequence([Value::I64(10), Value::I64(20), Value::I64(30)]),
            Value::from("abcdef"),
        ],
    )?;
    let read =
        |text: &str| -> Result<Value> { text.parse::<Expr>()?.bind(&nested)?.evaluate(&values) };
    // 0-based indexing, and a negative index counts from the end.
    assert_eq!(read("tags[0]")?, Value::I64(10));
    assert_eq!(read("tags[-1]")?, Value::I64(30));
    // Out of range is null, never an error.
    assert_eq!(read("tags[99]")?, Value::Null);
    // Half-open ranges, clamped rather than refused.
    assert_eq!(
        read("tags[1:3]")?,
        Value::from_sequence([Value::I64(20), Value::I64(30)])
    );
    assert_eq!(read("tags[1:99]")?.len(), 2);
    assert_eq!(read("tags[3:1]")?.len(), 0);
    // Text slices characters.
    assert_eq!(read("path[1:3]")?, Value::from("bc"));
    Ok(())
}

#[test]
fn a_text_range_never_splits_a_character_while_a_binary_one_slices_bytes() -> Result<()> {
    let nested = DataType::from_fields([
        DataType::Utf8.nullable_field("text"),
        DataType::Binary.nullable_field("bytes"),
    ])?
    .required_field("row");
    let values = row_of(
        &nested,
        [
            Value::from("héllo"),
            Value::Bytes(Arc::from("héllo".as_bytes())),
        ],
    )?;
    let text = "text[0:2]"
        .parse::<Expr>()?
        .bind(&nested)?
        .evaluate(&values)?;
    assert_eq!(text, Value::from("hé"));
    let bytes = "bytes[0:2]"
        .parse::<Expr>()?
        .bind(&nested)?
        .evaluate(&values)?;
    assert_eq!(bytes, Value::Bytes(Arc::from(&b"h\xc3"[..])));
    Ok(())
}

#[test]
fn an_accessor_the_datatype_cannot_answer_is_refused_by_name() -> Result<()> {
    let error = "n[0] = 1"
        .parse::<Expr>()?
        .bind(&schema())
        .expect_err("indexing an integer");
    let message = error.to_string();
    assert!(message.contains("int32"), "{message}");
    assert!(message.contains("[0]"), "{message}");
    Ok(())
}

#[test]
fn statistics_prune_only_what_they_prove() -> Result<()> {
    let predicate = "price > 100.00"
        .parse::<Expr>()?
        .bind(&schema())?
        .into_predicate()?;
    // Every stored value is below the bound, so no row can match.
    let below = Table(vec![(
        "price",
        ColumnStats::range(Value::Decimal(0, 2), Value::Decimal(5000, 2)),
    )]);
    assert_eq!(predicate.evaluate_stats(&below), Certainty::AlwaysFalse);
    // Every stored value is above it and none is null, so every row matches.
    let above = Table(vec![(
        "price",
        ColumnStats::range(Value::Decimal(20_000, 2), Value::Decimal(30_000, 2)),
    )]);
    assert_eq!(predicate.evaluate_stats(&above), Certainty::AlwaysTrue);
    // Coarse statistics leave room for both.
    let coarse = Table(vec![(
        "price",
        ColumnStats::range(Value::Decimal(0, 2), Value::Decimal(30_000, 2)),
    )]);
    assert_eq!(predicate.evaluate_stats(&coarse), Certainty::Maybe);
    // No statistics at all can never prune.
    assert_eq!(predicate.evaluate_stats(&()), Certainty::Maybe);
    Ok(())
}

#[test]
fn a_null_anywhere_stops_a_statistic_proving_a_comparison_true() -> Result<()> {
    let predicate = "price > 100.00"
        .parse::<Expr>()?
        .bind(&schema())?
        .into_predicate()?;
    let with_null = Table(vec![(
        "price",
        ColumnStats::range(Value::Decimal(20_000, 2), Value::Decimal(30_000, 2)).with_null_count(1),
    )]);
    // One null makes that row's answer unknown, so "every row matches" is not
    // provable - `AlwaysTrue` here would drop a conjunct that still has work.
    assert_eq!(predicate.evaluate_stats(&with_null), Certainty::Maybe);
    Ok(())
}

#[test]
fn a_constant_column_prunes_exactly_as_a_partition_directory_does() -> Result<()> {
    let predicate = "venue = 'XNAS'"
        .parse::<Expr>()?
        .bind(&schema())?
        .into_predicate()?;
    let matching = Table(vec![("venue", ColumnStats::constant(Value::from("XNAS")))]);
    assert_eq!(predicate.evaluate_stats(&matching), Certainty::AlwaysTrue);
    let other = Table(vec![("venue", ColumnStats::constant(Value::from("XNYS")))]);
    assert_eq!(predicate.evaluate_stats(&other), Certainty::AlwaysFalse);
    Ok(())
}

#[test]
fn a_list_element_never_prunes_however_tempting_the_statistic() -> Result<()> {
    let nested = DataType::from_fields([
        DataType::list(DataType::Int64.nullable_field("item")).nullable_field("tags")
    ])?
    .required_field("row");
    let predicate = "tags[0] > 100"
        .parse::<Expr>()?
        .bind(&nested)?
        .into_predicate()?;
    // A source that reports the *column's* bounds says nothing about one
    // element, so pruning on it would lose rows.
    struct Misleading;
    impl StatsSource for Misleading {
        fn stats(&self, _column: &BoundColumn) -> Option<ColumnStats> {
            Some(ColumnStats::range(Value::I64(0), Value::I64(1)))
        }
    }
    assert_eq!(predicate.evaluate_stats(&Misleading), Certainty::Maybe);
    Ok(())
}

#[test]
fn a_residual_drops_what_statistics_settled_and_keeps_the_rest() -> Result<()> {
    let predicate = "venue = 'XNAS' AND price > 100.00"
        .parse::<Expr>()?
        .bind(&schema())?
        .into_predicate()?;
    let settled = Table(vec![("venue", ColumnStats::constant(Value::from("XNAS")))]);
    let residual = predicate.residual(&settled);
    assert_eq!(residual.len(), 1);
    assert_eq!(residual[0].to_string(), "price > 100.00");
    Ok(())
}

#[test]
fn a_selection_of_bare_columns_produces_the_columns_it_named() -> Result<()> {
    let selection: Selection = "venue, price".parse()?;
    let bound = selection.bind(&schema())?;
    let names: Vec<&str> = bound.root().fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["venue", "price"]);
    assert!(selection.is_projection());
    Ok(())
}

#[test]
fn a_computed_column_takes_its_own_canonical_spelling_when_unnamed() -> Result<()> {
    let selection: Selection = "price * 2, price * 2 AS doubled".parse()?;
    let bound = selection.bind(&schema())?;
    let names: Vec<&str> = bound.root().fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["price * 2", "doubled"]);
    assert!(!selection.is_projection());
    Ok(())
}

#[test]
fn two_selected_columns_may_not_claim_one_name() -> Result<()> {
    let error = "venue, venue"
        .parse::<Selection>()?
        .bind(&schema())
        .expect_err("a duplicate name");
    assert!(error.to_string().contains("venue"), "{error}");
    Ok(())
}

#[test]
fn a_plan_shares_one_node_for_a_repeated_subexpression() -> Result<()> {
    let bound: Bound = "price * 2 > 1 AND price * 2 < 9"
        .parse::<Expr>()?
        .bind(&schema())?;
    let plan = bound.plan();
    let repeated = plan
        .reachable(bound.root())
        .into_iter()
        .filter(|id| matches!(plan.get(*id), Some(super::Node::Arithmetic { .. })))
        .count();
    assert_eq!(repeated, 1, "the repeated product is one node");
    Ok(())
}

#[test]
fn the_explanation_names_the_rules_that_fired() -> Result<()> {
    let bound = "n = 1 OR n = 2 OR n = 3".parse::<Expr>()?.bind(&schema())?;
    let explained = bound.explain();
    assert!(
        explained.contains("OR of equalities to an IN list"),
        "{explained}"
    );
    Ok(())
}

#[test]
fn a_non_boolean_expression_is_refused_as_a_filter_by_name() -> Result<()> {
    let error = "price * 2"
        .parse::<Expr>()?
        .bind(&schema())?
        .into_predicate()
        .expect_err("not a predicate");
    let message = error.to_string();
    assert!(message.contains("boolean"), "{message}");
    assert!(message.contains("decimal"), "{message}");
    Ok(())
}

#[test]
fn arithmetic_overflow_is_refused_rather_than_wrapped() -> Result<()> {
    // Two of them still fit the widest exact integer this value model holds,
    // so the refusal is proven with three.
    let bound = "id * id * id".parse::<Expr>()?.bind(&schema())?;
    let value = row([
        Value::Null,
        Value::Null,
        Value::I32(0),
        Value::I64(i64::MAX),
    ]);
    assert!(bound.evaluate(&value).is_err());
    Ok(())
}

#[test]
fn decimal_arithmetic_keeps_the_exact_coefficient_model() -> Result<()> {
    let bound = "price + 0.01".parse::<Expr>()?.bind(&schema())?;
    let value = row([
        Value::Null,
        Value::Decimal(1010, 2),
        Value::I32(0),
        Value::Null,
    ]);
    assert_eq!(bound.evaluate(&value)?, Value::Decimal(1011, 2));
    Ok(())
}

#[test]
fn a_calendar_function_reads_the_calendar_field_not_an_epoch_offset() -> Result<()> {
    let dated =
        DataType::from_fields([DataType::Date32.nullable_field("d")])?.required_field("row");
    let value = row_of(&dated, [Value::Date(19_783)])?;
    assert_eq!(
        "year(d)".parse::<Expr>()?.bind(&dated)?.evaluate(&value)?,
        Value::I32(2024)
    );
    assert_eq!(
        "EXTRACT(MONTH FROM d)"
            .parse::<Expr>()?
            .bind(&dated)?
            .evaluate(&value)?,
        Value::I32(3)
    );
    Ok(())
}

#[test]
fn the_index_panic_is_the_only_one_and_it_names_the_foreign_id() -> Result<()> {
    let bound = "n = 1".parse::<Expr>()?.bind(&schema())?;
    let other = "n = 2".parse::<Expr>()?.bind(&schema())?;
    let outcome = std::panic::catch_unwind(|| {
        let plan = other.plan();
        let _ = &plan[super::NodeId::from_index(usize::MAX)];
    });
    assert!(outcome.is_err());
    // And an id from this plan indexes fine.
    let _ = &bound.plan()[bound.root()];
    Ok(())
}

/// Build a row under an arbitrary schema.
fn row_of<const N: usize>(schema: &Field, values: [Value; N]) -> Result<Value> {
    Value::record(schema.data_type().clone(), values)
}
