//! The optimizer's own properties, which is where a silent wrong answer would
//! come from.

use yggdryl::{DataType, Expr, Field, Value};

/// A schema with one nullable and one required column of each shape.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.nullable_field("a"),
        DataType::Int64.required_field("required"),
        DataType::Utf8.nullable_field("text"),
        DataType::Int32.nullable_field("narrow"),
    ])
    .expect("a four-column struct")
    .required_field("row")
}

/// Rows covering the interesting values including nulls.
fn rows() -> Vec<Value> {
    let data_type = schema().data_type().clone();
    [
        [
            Value::I64(1),
            Value::I64(1),
            Value::from("aa"),
            Value::I32(1),
        ],
        [
            Value::I64(2),
            Value::I64(2),
            Value::from("bb"),
            Value::I32(2),
        ],
        [
            Value::I64(3),
            Value::I64(3),
            Value::from("cc"),
            Value::I32(3),
        ],
        [Value::Null, Value::I64(4), Value::Null, Value::Null],
    ]
    .into_iter()
    .map(|values| Value::record(data_type.clone(), values).expect("a row"))
    .collect()
}

/// Every row a predicate keeps, through the row evaluator.
fn kept(text: &str) -> Vec<bool> {
    let schema = schema();
    let predicate = text
        .parse::<Expr>()
        .unwrap_or_else(|error| panic!("{text}: {error}"))
        .bind(&schema)
        .unwrap_or_else(|error| panic!("{text}: {error}"))
        .into_predicate()
        .unwrap_or_else(|error| panic!("{text}: {error}"));
    rows()
        .iter()
        .map(|row| predicate.matches(row).expect("evaluates"))
        .collect()
}

#[test]
fn every_rule_preserves_what_the_predicate_selects() {
    for text in [
        "a = 1 OR a = 2 OR a = 3",
        "a > 1 AND a > 2",
        "a >= 1 AND a <= 3",
        "a IN (1, 2) AND a IN (2, 3)",
        "a BETWEEN 1 AND 3",
        "a NOT BETWEEN 1 AND 3",
        "NOT (a = 1 OR a = 2)",
        "NOT NOT (a = 1)",
        "NOT (a IS NULL)",
        "a = 1 AND (a = 1 OR a = 2)",
        "a = 1 OR (a = 1 AND a = 2)",
        "text LIKE 'a%'",
        "text LIKE 'aa'",
        "1 < a",
        "a = 1 AND TRUE",
        "a = 1 OR FALSE",
        "CAST(narrow AS int64) = 2",
        "CAST(narrow AS int64) > 2",
        "(a = 1 AND text = 'aa') OR (a = 1 AND text = 'bb')",
    ] {
        let written = text.parse::<Expr>().expect("parses");
        let simplified = written.simplify();
        assert_eq!(
            kept(text),
            kept(&simplified.to_string()),
            "{text:?} changed meaning when simplified to {simplified}"
        );
    }
}

#[test]
fn a_reflexive_comparison_is_never_folded() {
    // Both are unknown when the column is null, and unknown is not a constant.
    assert_eq!(
        "a = a"
            .parse::<Expr>()
            .expect("parses")
            .simplify()
            .to_string(),
        "a = a"
    );
    assert_eq!(
        "a <> a"
            .parse::<Expr>()
            .expect("parses")
            .simplify()
            .to_string(),
        "a <> a"
    );
    // The row with a null must not be selected by either.
    assert_eq!(kept("a = a"), vec![true, true, true, false]);
    assert_eq!(kept("a <> a"), vec![false, false, false, false]);
}

#[test]
fn a_contradiction_folds_only_where_the_schema_proves_no_null() {
    let schema = schema();
    // Nullable: the pair stays, because `FALSE` would be wrong on a null.
    let nullable = "a > 5 AND a < 3"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds");
    assert!(!nullable.is_always_false(), "{}", nullable.to_expr());
    assert!(
        nullable.explain().contains("declined contradictory range"),
        "{}",
        nullable.explain()
    );

    // Required: no null is possible, so the fold is sound and fires.
    let required = "required > 5 AND required < 3"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds");
    assert!(required.is_always_false(), "{}", required.to_expr());
}

#[test]
fn the_cast_on_a_column_moves_to_the_literal_where_that_is_exact() {
    let schema = schema();
    // A widening cast with a literal that fits: the cast moves, so the column
    // is compared directly and statistics can still bound it.
    let moved = "CAST(narrow AS int64) = 2"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds");
    assert!(
        moved
            .explain()
            .contains("fired cast moved from column to literal"),
        "{}",
        moved.explain()
    );
    assert!(
        !moved.to_expr().to_string().contains("CAST"),
        "{}",
        moved.to_expr()
    );

    // A literal the column's own width cannot hold: the rule declines, and the
    // cast stays where it is rather than changing what is asked.
    let held = format!("CAST(narrow AS int64) = {}", i64::from(i32::MAX) + 1)
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds");
    assert!(
        held.explain()
            .contains("declined cast moved from column to literal"),
        "{}",
        held.explain()
    );
}

#[test]
fn optimizing_an_optimized_plan_changes_nothing() {
    for text in [
        "a = 1 OR a = 2 OR a = 3",
        "a > 1 AND a > 2 AND text LIKE 'a%'",
        "NOT (a = 1 AND (text = 'x' OR a = 2))",
        "a BETWEEN 1 AND 5 AND a IN (1, 2, 3)",
        "(a = 1 AND text = 'aa') OR (a = 1 AND text = 'bb')",
    ] {
        let once = text.parse::<Expr>().expect("parses").simplify();
        assert_eq!(once.simplify(), once, "{text:?} is not a fixed point");
        // And the same input gives the same plan every run.
        assert_eq!(once, text.parse::<Expr>().expect("parses").simplify());
    }
}

#[test]
fn a_repeated_subexpression_is_one_node_and_is_computed_once() {
    let schema = schema();
    let bound = "a * 2 > 1 AND a * 2 < 9 AND a * 2 <> 4"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds");
    let plan = bound.plan();
    let products = plan
        .reachable(bound.root())
        .into_iter()
        .filter(|id| {
            matches!(
                plan.get(*id),
                Some(yggdryl::expressions::Node::Arithmetic { .. })
            )
        })
        .count();
    assert_eq!(products, 1, "{}", bound.explain());
    // The one node is read by three parents, which is what "computed once"
    // means in a DAG.
    let product = plan
        .reachable(bound.root())
        .into_iter()
        .find(|id| {
            matches!(
                plan.get(*id),
                Some(yggdryl::expressions::Node::Arithmetic { .. })
            )
        })
        .expect("the product node");
    assert_eq!(plan.parents(product).len(), 3, "{}", bound.explain());
    assert!(plan.is_shared(product));
}

#[test]
fn a_dead_node_never_reaches_the_evaluator() {
    let schema = schema();
    let bound = "a = 1 AND TRUE"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds");
    let plan = bound.plan();
    let reachable = plan.reachable(bound.root());
    // The literal TRUE was absorbed, so nothing reads it any more.
    assert!(
        !reachable.iter().any(|id| matches!(
            plan.get(*id),
            Some(yggdryl::expressions::Node::Literal(Value::Bool(true)))
        )),
        "{}",
        plan.explain_from(bound.root())
    );
}

#[test]
fn a_wide_disjunction_reports_the_guard_rather_than_exploding() {
    // Twelve two-operand conjunctions would distribute into 4096 clauses; the
    // guard keeps the original shape and says so instead.
    let wide = (0..12)
        .map(|index| format!("(a = {index} AND required = {index})"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let bound = wide
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema())
        .expect("binds");
    assert!(
        bound
            .explain()
            .contains("declined disjunction distributed to CNF"),
        "{}",
        bound.explain()
    );
    // And it still answers correctly.
    assert_eq!(kept(&wide), vec![true, true, true, false]);
}

#[test]
fn the_optimizer_terminates_on_a_deliberately_awkward_plan() {
    // Alternating shapes that several rules all want to touch, to prove the
    // fixed-point driver settles rather than ping-ponging.
    let awkward = "NOT (NOT (a = 1 OR a = 2)) AND a BETWEEN 1 AND 3 AND a IN (1, 2, 3) \
                   AND NOT (a = 9) AND (a = 1 OR (a = 1 AND a = 2))";
    let bound = awkward
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema())
        .expect("binds");
    assert!(
        !bound.explain().contains("fixed-point cap reached"),
        "{}",
        bound.explain()
    );
}

#[test]
fn a_ten_thousand_step_expression_is_refused_as_a_typed_error() {
    let deep = std::iter::repeat_n("a = 1", 10_000)
        .collect::<Vec<_>>()
        .join(" AND ");
    // A wide conjunction is not deep, so this parses and simplifies to one
    // conjunct - which is what interning is for.
    let parsed: Expr = deep.parse().expect("a wide conjunction parses");
    assert_eq!(parsed.simplify().to_string(), "a = 1");

    // Depth, on the other hand, is bounded and refused by name.
    let nested = format!("{}a = 1{}", "NOT (".repeat(10_000), ")".repeat(10_000));
    let error = nested.parse::<Expr>().expect_err("over-deep");
    assert!(error.to_string().contains("hard limit"), "{error}");
}
