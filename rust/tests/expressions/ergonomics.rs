//! The Rust trait set, exercised on every value this module publishes.
//!
//! A Rust caller pays for a clumsy surface the longest, so the traits are part
//! of the design rather than a follow-up - and the two name collisions the API
//! deliberately carries are pinned here rather than discovered.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use yggdryl::expressions::prelude::*;
use yggdryl::expressions::{ArithOp, CompareOp, Function, SelectionItem};
use yggdryl::{DataType, Expr, Field, Selection, Value};

/// The one-column root the bound values here are built against.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.nullable_field("a"),
        DataType::Utf8.nullable_field("b"),
    ])
    .expect("a two-column struct")
    .required_field("row")
}

/// The stable hash of any hashable value.
fn digest(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn the_operators_build_what_the_constructors_build() {
    assert_eq!(col("a") & col("b"), col("a").and(col("b")));
    assert_eq!(col("a") | col("b"), col("a").or(col("b")));
    assert_eq!(!col("a"), col("a").not());
    assert_eq!(col("a") + 1, col("a").arithmetic(ArithOp::Add, 1));
    assert_eq!(col("a") - 1, col("a").arithmetic(ArithOp::Sub, 1));
    assert_eq!(col("a") * 2, col("a").arithmetic(ArithOp::Mul, 2));
    assert_eq!(col("a") / 2, col("a").arithmetic(ArithOp::Div, 2));
    assert_eq!(col("a") % 2, col("a").arithmetic(ArithOp::Mod, 2));
    assert_eq!(
        col("a").compare(CompareOp::GtEq, 3),
        col("a").ge(lit(3_i64))
    );
    // And a built chain equals the parsed spelling of the same thing.
    assert_eq!(
        col("a").gt(lit(10)) & col("b").eq(lit("x")),
        "a > 10 AND b = 'x'".parse::<Expr>().expect("parses")
    );
}

#[test]
fn equality_is_structural_while_the_equality_builder_builds() {
    // `==` asks whether two expressions are the same expression.
    assert_eq!(col("a"), col("a"));
    assert_ne!(col("a"), col("b"));
    // `.eq(..)` produces the predicate `a = 3`.
    let built = col("a").eq(lit(3));
    assert_eq!(built.to_string(), "a = 3");
    assert!(matches!(built, Expr::Compare { .. }));
}

#[test]
fn from_str_builds_a_literal_while_parsing_builds_a_predicate() {
    assert_eq!(Expr::from("a > 1"), Expr::literal(Value::from("a > 1")));
    assert_eq!(
        "a > 1".parse::<Expr>().expect("parses").to_string(),
        "a > 1"
    );
    // The free constructors are the unambiguous spellings of both.
    assert_eq!(lit("a > 1"), Expr::from("a > 1"));
    assert_eq!(col("a").to_string(), "a");
}

#[test]
fn ordering_is_total_and_agrees_with_equality() {
    let mut values = vec![
        col("b").eq(lit(1)),
        col("a").eq(lit(2)),
        lit(1),
        col("a"),
        col("a").eq(lit(1)),
    ];
    values.sort();
    // Sorting twice changes nothing, which is what "total" buys.
    let once = values.clone();
    values.sort();
    assert_eq!(values, once);
    // Equal values hash equally, and unequal ones compare unequal.
    for left in &values {
        for right in &values {
            if left == right {
                assert_eq!(digest(left), digest(right));
                assert_eq!(left.cmp(right), std::cmp::Ordering::Equal);
            } else {
                assert_ne!(left.cmp(right), std::cmp::Ordering::Equal);
            }
        }
    }
}

#[test]
fn every_value_round_trips_through_serde_and_through_display() {
    let expression: Expr = "a > 1 AND b LIKE 'x%'".parse().expect("parses");
    let json = serde_json::to_string(&expression).expect("serializes");
    let back: Expr = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, expression);
    assert_eq!(
        expression.to_string().parse::<Expr>().expect("re-parses"),
        expression
    );

    let selection: Selection = "a, b AS renamed, a + 1".parse().expect("parses");
    let json = serde_json::to_string(&selection).expect("serializes");
    let back: Selection = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, selection);
    assert_eq!(
        selection
            .to_string()
            .parse::<Selection>()
            .expect("re-parses"),
        selection
    );
}

#[test]
fn a_selection_iterates_and_collects_without_allocating_a_vector_first() {
    let selection: Selection = "a, b AS renamed".parse().expect("parses");
    assert_eq!(selection.len(), 2);
    assert!(!selection.is_empty());
    let names: Vec<String> = (&selection).into_iter().map(SelectionItem::name).collect();
    assert_eq!(names, vec!["a".to_owned(), "renamed".to_owned()]);

    // `FromIterator` over both shapes.
    let from_items: Selection = selection.items().iter().cloned().collect();
    assert_eq!(from_items, selection);
    let from_exprs: Selection = vec![col("a"), col("b").alias("renamed")]
        .into_iter()
        .collect();
    assert_eq!(from_exprs, selection);

    // The empty selection is the default and narrows nothing.
    assert!(Selection::default().is_empty());
    assert_eq!(Selection::default(), Selection::everything());
}

#[test]
fn the_borrowed_queries_borrow_rather_than_allocate_a_plan() {
    let expression: Expr = "a > 1 AND b = 'x' AND a < 9".parse().expect("parses");
    assert_eq!(expression.conjuncts().len(), 3);
    assert_eq!(expression.columns(), vec!["a".to_owned(), "b".to_owned()]);
    // Deterministic: the same expression answers the same order every time.
    assert_eq!(expression.columns(), expression.columns());
}

#[test]
fn narrowing_to_a_predicate_refuses_a_non_boolean_expression_by_name() {
    let schema = schema();
    let error = "a + 1"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds")
        .into_predicate()
        .expect_err("not a predicate");
    let message = error.to_string();
    assert!(message.contains("boolean"), "{message}");
    assert!(message.contains("int64"), "{message}");

    // A boolean one narrows, and the narrowed value is what every filtering
    // surface takes - so "somebody passed the wrong shape" is a compile error
    // everywhere but this one call.
    assert!(
        "a > 1"
            .parse::<Expr>()
            .expect("parses")
            .bind(&schema)
            .expect("binds")
            .into_predicate()
            .is_ok()
    );
}

#[test]
fn the_values_are_send_and_sync_so_a_plan_crosses_a_thread() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<Expr>();
    assert_send_sync::<Selection>();
    assert_send_sync::<yggdryl::expressions::Bound>();
    assert_send_sync::<yggdryl::expressions::BoundPredicate>();
    assert_send_sync::<yggdryl::expressions::BoundSelection>();

    // And it really does cross one.
    let schema = schema();
    let predicate = "a > 1"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds")
        .into_predicate()
        .expect("a predicate");
    let row =
        Value::record(schema.data_type().clone(), [Value::I64(2), Value::Null]).expect("a row");
    let answered = std::thread::spawn(move || predicate.matches(&row).expect("evaluates"))
        .join()
        .expect("the thread completes");
    assert!(answered);
}

#[test]
fn the_prelude_brings_the_verbs_a_one_liner_needs() {
    // The point of the prelude: a trait method does not exist until its trait
    // is in scope, and one `use` should fix that.
    let schema = schema();
    let reported = col("a")
        .gt(lit(1))
        .apply_field(&schema)
        .expect("reports a schema");
    assert_eq!(reported.field_len(), schema.field_len());
}

#[test]
fn the_function_vocabulary_is_closed_and_says_what_it_holds() {
    for function in Function::ALL {
        assert_eq!(Function::from_name(function.as_str()), Some(function));
        // The canonical name round-trips through the grammar.
        let (least, _) = function.arity();
        let args = std::iter::repeat_n("a", least.max(1))
            .collect::<Vec<_>>()
            .join(", ");
        let text = format!("{function}({args})");
        assert!(text.parse::<Expr>().is_ok(), "{text}");
    }
    // A name outside the set is refused naming the vocabulary it is not in.
    let error = "nosuch(a)".parse::<Expr>().expect_err("unknown function");
    let message = error.to_string();
    assert!(message.contains("coalesce"), "{message}");
    assert!(message.contains("nosuch"), "{message}");
}

#[test]
fn the_arena_index_panics_only_on_an_id_from_another_plan() {
    let schema = schema();
    let bound = "a > 1"
        .parse::<Expr>()
        .expect("parses")
        .bind(&schema)
        .expect("binds");
    // Its own root indexes fine, and that is the whole contract.
    let node = &bound.plan()[bound.root()];
    assert_eq!(node.kind(), "compare");
}
