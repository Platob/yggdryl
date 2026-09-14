//! The edge cases this module is built to get right.
//!
//! Five properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, a free attribute never costs a backend
//! call, and a pruning decision never loses a row.

use std::cell::Cell;
use std::hash::Hash;
use std::sync::Arc;

use yggdryl::expression::{
    Attribute, Bound, Bounds, ColumnBounds, Cost, Expression, Filter, Literal, Projection,
    Residual, Selector, Term,
};
use yggdryl::{DataType, Field, MediaType, Result, Scalar, TimeUnit, Timezone, Url};

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// Every spelling the grammar accepts, one of each shape.
const CORPUS: [&str; 33] = [
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
    "trade.legs[1:3]",
    "trade.legs[:-1]",
    "lower(name)[0]",
    "-x + 3 * 2 - 1",
    ":since <= ts",
];

#[test]
fn text_round_trips() {
    for text in CORPUS {
        let parsed: Term = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let printed = parsed.to_string();
        let again: Term = printed
            .parse()
            .unwrap_or_else(|error| panic!("{printed}: {error}"));
        assert_eq!(parsed, again, "{text} printed as {printed}");
    }
}

#[test]
fn terms_and_expressions_have_core_total_order_and_stable_hash() {
    fn assert_value_traits<T: Clone + Eq + Hash + Ord>() {}
    assert_value_traits::<ColumnBounds>();
    assert_value_traits::<Bounds>();
    assert_value_traits::<Residual>();
    assert_value_traits::<Term>();
    assert_value_traits::<Filter>();
    assert_value_traits::<Projection>();
    assert_value_traits::<Selector>();
    assert_value_traits::<Expression>();

    let first: Term = "a = 1".parse().unwrap();
    let equal: Term = first.to_string().parse().unwrap();
    let later: Term = "b = 1".parse().unwrap();
    assert_eq!(first.stable_hash(), equal.stable_hash());
    assert!(first < later);

    let first: Expression = "select a".parse().unwrap();
    let equal: Expression = first.to_string().parse().unwrap();
    let later: Expression = "where a > 1".parse().unwrap();
    assert_eq!(first.stable_hash(), equal.stable_hash());
    assert!(first < later, "a selector orders before a filter");
}

#[test]
fn quoted_names_survive_every_encapsulator() {
    for text in ["\"odd name\" = 1", "`odd name` = 1"] {
        let parsed: Term = text.parse().unwrap();
        assert_eq!(parsed.columns(), vec!["odd name".to_owned()]);
        assert_eq!(parsed.to_string(), "\"odd name\" = 1");
    }
    // A doubled quote inside a quoted name is one quote, as SQL spells it.
    let parsed: Term = "\"say \"\"hi\"\"\" = 1".parse().unwrap();
    assert_eq!(parsed.columns(), vec!["say \"hi\"".to_owned()]);
    assert_eq!(parsed.to_string().parse::<Term>().unwrap(), parsed);
    // A reserved word is a column only when quoted, and prints quoted.
    let parsed: Term = "\"select\" = 1".parse().unwrap();
    assert_eq!(parsed.columns(), vec!["select".to_owned()]);
    assert_eq!(parsed.to_string(), "\"select\" = 1");
}

#[test]
fn expressions_round_trip_and_name_their_clause() {
    for text in [
        "select *",
        "select a, b as c",
        "select lower(name) as name, price * 2 as doubled",
        "select i as total int64 not null, s utf8 null",
        "select nested.leg as leg, xs[1:3] as middle",
        "where a > 1",
        "where a = 1 and b is not null",
    ] {
        let parsed: Expression = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let printed = parsed.to_string();
        assert_eq!(printed, text, "{text} printed as {printed}");
        let again: Expression = printed.parse().unwrap();
        assert_eq!(parsed, again);
        let document = parsed.clone().into_json().unwrap();
        assert_eq!(Expression::from_json(&document).unwrap(), parsed, "{text}");
    }
    // Each clause is also its own type, and the keyword is optional there.
    let selector: Selector = "a, b as c".parse().unwrap();
    assert_eq!(selector.to_string(), "a, b as c");
    assert_eq!("select a, b as c".parse::<Selector>().unwrap(), selector);
    let filter: Filter = "a > 1".parse().unwrap();
    assert_eq!(filter.to_string(), "a > 1");
    assert_eq!("where a > 1".parse::<Filter>().unwrap(), filter);
    // An expression has to say which clause it is.
    let error = "a > 1".parse::<Expression>().unwrap_err().to_string();
    assert!(error.contains("select"), "{error}");
    assert!(error.contains("where"), "{error}");
    let error = "select".parse::<Expression>().unwrap_err().to_string();
    assert!(error.contains("projection"), "{error}");
}

#[test]
fn documents_round_trip() {
    for text in CORPUS {
        let parsed: Term = text.parse().unwrap();
        let document = parsed.clone().into_json().unwrap();
        assert_eq!(Term::from_json(&document).unwrap(), parsed, "{text}");
    }
    let selector: Selector = "a as b int64 not null, c + 1".parse().unwrap();
    let document = selector.clone().into_json().unwrap();
    assert_eq!(Selector::from_json(&document).unwrap(), selector);
    let filter: Filter = "a > 1".parse().unwrap();
    let document = filter.clone().into_json().unwrap();
    assert_eq!(Filter::from_json(&document).unwrap(), filter);
}

#[test]
fn a_parse_failure_names_where_it_stopped() {
    let error = "a = ".parse::<Term>().unwrap_err();
    assert!(
        format!("{error}").contains("at byte 4"),
        "expected a byte position, got {error}"
    );
    let error = "a === 1".parse::<Term>().unwrap_err();
    assert!(format!("{error}").contains("at byte "), "{error}");
    let error = "nosuchfn(a)".parse::<Term>().unwrap_err();
    assert!(format!("{error}").contains("lower"), "{error}");
    let error = "&holder.nosuch".parse::<Term>().unwrap_err();
    assert!(format!("{error}").contains("partition"), "{error}");
    let error = "a in ()".parse::<Term>().unwrap_err();
    assert!(format!("{error}").contains("at least one"), "{error}");
    let error = "select a as".parse::<Expression>().unwrap_err();
    assert!(format!("{error}").contains("at byte "), "{error}");
}

#[test]
fn nesting_past_the_limit_is_refused_not_crashed() {
    let deep = format!(
        "{}a{}",
        "(".repeat(yggdryl::expression::RECURSION_LIMIT + 8),
        ")".repeat(yggdryl::expression::RECURSION_LIMIT + 8)
    );
    let error = deep.parse::<Term>().unwrap_err();
    assert!(format!("{error}").contains("hard limit"), "{error}");
}

#[test]
fn a_pattern_that_changes_per_row_is_refused_at_bind() {
    let schema = rows_schema();
    let error = "s like s".parse::<Term>().unwrap().bind(&schema);
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
            Field::new("s", DataType::utf8(), true),
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
                DataType::from_fields([Field::new("leg", DataType::utf8(), true)]).unwrap(),
                true,
            ),
            // Temporal text, so a cast into and out of a temporal is one of
            // the pairs the two tiers are compared on.
            Field::new("clock", DataType::utf8(), true),
            // A list, so a position and a run are compared on both tiers.
            Field::new(
                "xs",
                DataType::list(DataType::Int64.nullable_field("item")),
                true,
            ),
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
    let list = |items: &[i64]| Scalar::from_sequence(items.iter().map(|item| Scalar::from(*item)));
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
            list(&[1, 2, 3]),
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
            list(&[]),
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
            list(&[7]),
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
            list(&[0, -1]),
        ]),
    ]
}

/// The predicates the two tiers are compared on, all evaluable per row.
const AGREEMENT: [&str; 29] = [
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
    "i = 1 or i = 100 or i = 0",
    "not (i in (1, 100) and s like 'a%')",
    "xs[0] = 1",
    // Text entering a temporal and a temporal leaving as text: the vectorized
    // tier reads and spells with the code the row tier reads and spells with,
    // a zone name and an hour past the end of the day included.
    "cast(clock as time64(microsecond))",
    "cast(clock as duration64(millisecond))",
    "cast(t as string)",
    "try_cast(clock as time32(second))",
];

/// Evaluate one term on both tiers and assert they agree on every row.
fn assert_tiers_agree(text: &str, schema: &Field, rows: &[Scalar]) -> Bound {
    let batch = batch_of(schema, rows);
    let bound = text
        .parse::<Term>()
        .unwrap_or_else(|error| panic!("{text}: {error}"))
        .bind(schema)
        .unwrap_or_else(|error| panic!("{text}: {error}"));
    let vectorized = bound.evaluate(&batch).unwrap();
    for (position, row) in rows.iter().enumerate() {
        let scalar = bound.eval(row).unwrap();
        // One row out of the vectorized column, through the public boundary:
        // a one-element slice is the scalar it holds.
        let held = yggdryl::arrow::scalar_value(
            &bound.field().clone().with_nullable(true),
            vectorized.slice(position, 1).as_ref(),
        )
        .unwrap();
        assert_eq!(
            scalar, held,
            "{text} disagreed on row {position}: scalar {scalar:?}, vectorized {held:?}"
        );
    }
    bound
}

#[test]
fn scalar_and_vectorized_agree() {
    let schema = rows_schema();
    let rows = rows();
    for text in AGREEMENT {
        assert_tiers_agree(text, &schema, &rows);
    }
}

#[test]
fn scalar_arithmetic_propagates_checked_failures() {
    let bound = "n / i"
        .parse::<Term>()
        .unwrap()
        .bind(&rows_schema())
        .unwrap();
    assert!(matches!(
        bound.eval(&rows()[4]),
        Err(yggdryl::Error::DivisionByZero { .. })
    ));
    assert_eq!(bound.eval(&rows()[2]).unwrap(), Scalar::Null);

    let schema = Field::new(
        "rows",
        DataType::from_fields([Field::new("small", DataType::Int8, false)]).unwrap(),
        false,
    );
    let negated = "-small".parse::<Term>().unwrap().bind(&schema).unwrap();
    assert!(matches!(
        negated.eval(&Scalar::from_sequence([Scalar::from(i8::MIN)])),
        Err(yggdryl::Error::ArithmeticOverflow {
            operation: "negation",
            ..
        })
    ));
}

#[test]
fn projections_agree_between_the_tiers() {
    let schema = rows_schema();
    let rows = rows();
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
        "xs[0]",
        "xs[-1]",
        "xs[5]",
        "xs[1:3]",
        "xs[:1]",
        "xs[1:]",
        "xs[-2:]",
        "slice(xs, 1, null)",
        "get(nested, 'leg')",
    ] {
        assert_tiers_agree(text, &schema, &rows);
    }
}

#[test]
fn a_run_of_a_list_is_typed_and_bounded_like_the_grammar_says() {
    let schema = rows_schema();
    let rows = rows();
    let middle = "xs[1:3]".parse::<Term>().unwrap().bind(&schema).unwrap();
    assert_eq!(middle.field().dtype(), &schema.fields()[9].dtype().clone());
    let list = |items: &[i64]| Scalar::from_sequence(items.iter().map(|item| Scalar::from(*item)));
    assert_eq!(middle.eval(&rows[0]).unwrap(), list(&[2, 3]));
    assert_eq!(middle.eval(&rows[1]).unwrap(), list(&[]));
    assert_eq!(middle.eval(&rows[2]).unwrap(), Scalar::Null);
    assert_eq!(middle.eval(&rows[3]).unwrap(), list(&[]));
    let tail = "xs[-2:]".parse::<Term>().unwrap().bind(&schema).unwrap();
    assert_eq!(tail.eval(&rows[0]).unwrap(), list(&[2, 3]));
    assert_eq!(tail.eval(&rows[3]).unwrap(), list(&[7]));
    let last = "xs[-1]".parse::<Term>().unwrap().bind(&schema).unwrap();
    assert_eq!(last.eval(&rows[0]).unwrap(), Scalar::from(3_i64));
    assert_eq!(last.eval(&rows[1]).unwrap(), Scalar::Null);
}

fn batch_of(schema: &Field, rows: &[Scalar]) -> arrow_array::RecordBatch {
    let arrow_schema = schema.clone().into_arrow_schema().unwrap();
    let columns = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let values: Vec<Scalar> = rows
                .iter()
                .map(|row| row.as_sequence().unwrap()[index].clone())
                .collect();
            yggdryl::arrow::array_from_value(field, &yggdryl::Scalar::from_sequence(values))
                .unwrap()
        })
        .collect();
    arrow_array::RecordBatch::try_new(arrow_schema, columns).unwrap()
}

// ---------------------------------------------------------------------------
// Simplification
// ---------------------------------------------------------------------------

#[test]
fn a_simplification_has_fewer_nodes_and_one_shape() {
    for (text, expected) in [
        ("a = 1 or a = 2", "a in (1, 2)"),
        (
            "a = 1 or a = 2 or b = 3 or a in (4)",
            "a in (1, 2, 4) or b = 3",
        ),
        ("1 = a or a = 1", "a = 1"),
        ("a in (1)", "a = 1"),
        ("a in (1, 2) and a in (2, 3)", "a = 2"),
        ("a in (1, 2) and a in (3, 4)", "a in (1, 2) and a in (3, 4)"),
        ("not (a = 1)", "a <> 1"),
        ("not (a < 1)", "a >= 1"),
        ("not (a is null)", "a is not null"),
        ("not (a = 1 and b = 2)", "a <> 1 or b <> 2"),
        ("not (a = 1 or a = 2)", "not a in (1, 2)"),
        ("not not a", "a"),
        ("a and true", "a"),
        ("a and false", "false"),
        ("a or true", "true"),
        ("a or false", "a"),
        ("a and (b and c)", "a and b and c"),
        ("a and a", "a"),
        ("a = null", "null"),
        ("null <> a", "null"),
        ("a is distinct from null", "a is not null"),
        ("a is not distinct from null", "a is null"),
        ("not (a like 'x%')", "not a like 'x%'"),
    ] {
        let parsed: Term = text.parse().unwrap();
        let simplified = parsed.simplify();
        assert_eq!(simplified.to_string(), expected, "{text}");
        assert!(
            simplified.node_count() <= parsed.node_count(),
            "{text} grew from {} to {} nodes",
            parsed.node_count(),
            simplified.node_count()
        );
        assert_eq!(
            simplified.simplify(),
            simplified,
            "{text} is not a fixed point"
        );
    }
}

#[test]
fn a_simplification_answers_what_the_original_answered() {
    let schema = rows_schema();
    let rows = rows();
    for text in [
        "i = 1 or i = 100 or i = 0",
        "not (i = 1)",
        "not (i < 0)",
        "not (i is null)",
        "not (i = 1 and s = 'alpha')",
        "not (i in (1, 100) or s like 'a%')",
        "i in (1, 100) and i in (100, 0)",
        "i in (1, 100) and i in (0, -3)",
        "i = null",
        "i is distinct from null",
        "f = f or f is null",
        "not (f > 1.0)",
        "b and true",
        "b or false",
    ] {
        let original: Term = text.parse().unwrap();
        let simplified = original.simplify();
        let held = original.bind(&schema).unwrap();
        let reduced = simplified.bind(&schema).unwrap();
        for (position, row) in rows.iter().enumerate() {
            assert_eq!(
                held.eval(row).unwrap(),
                reduced.eval(row).unwrap(),
                "{text} simplified to {simplified} changed row {position}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

#[test]
fn a_selector_publishes_the_field_it_computes() {
    let schema = rows_schema();
    let selector: Selector = "i, i + 1 as next, s as name string not null, nested.leg"
        .parse()
        .unwrap();
    let output = selector.apply_field(&schema).unwrap();
    assert_eq!(output.name(), "rows");
    let names: Vec<&str> = output.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["i", "next", "name", "leg"]);
    assert_eq!(output.fields()[1].dtype(), &DataType::Int64);
    assert!(!output.fields()[2].is_nullable());
    assert_eq!(output.fields()[3].dtype(), &DataType::utf8());
    assert!(output.fields()[3].is_nullable());
    assert_eq!(selector.columns(), vec!["i", "s", "nested"]);
    assert_eq!(Selector::all().apply_field(&schema).unwrap(), schema);

    let twice: Selector = "i, i".parse().unwrap();
    let error = twice.apply_field(&schema).unwrap_err().to_string();
    assert!(error.contains("twice"), "{error}");
    let missing: Selector = "nope".parse().unwrap();
    assert!(missing.apply_field(&schema).is_err());
}

#[test]
fn a_selector_computes_rows_on_both_tiers() {
    let schema = rows_schema();
    let rows = rows();
    let batch = batch_of(&schema, &rows);
    let selector: Selector = "i * 2 as doubled, coalesce(s, '-') as name string not null, n int64"
        .parse()
        .unwrap();
    let projected = selector.apply_arrow_batch(&batch).unwrap();
    let output = selector.apply_field(&schema).unwrap();
    assert_eq!(
        projected.schema().as_ref(),
        output.clone().into_arrow_schema().unwrap().as_ref()
    );
    let bound = selector.bind(&schema).unwrap();
    for (position, row) in rows.iter().enumerate() {
        let scalar = bound.apply_scalar(row).unwrap();
        let held: Vec<Scalar> = output
            .fields()
            .iter()
            .zip(projected.columns())
            .map(|(field, column)| {
                yggdryl::arrow::scalar_value(
                    &field.clone().with_nullable(true),
                    column.slice(position, 1).as_ref(),
                )
                .unwrap()
            })
            .collect();
        assert_eq!(scalar, Scalar::from_sequence(held), "row {position}");
    }
    // The declared type is what the column carries, on both tiers.
    assert_eq!(
        bound.apply_scalar(&rows[0]).unwrap(),
        Scalar::from_sequence([
            Scalar::from(2_i64),
            Scalar::from("alpha"),
            Scalar::from(2024_i64)
        ])
    );
}

#[test]
fn a_projection_reorders_without_touching_a_buffer() {
    let schema = rows_schema();
    let batch = batch_of(&schema, &rows());
    let selector: Selector = "select s, i".parse().unwrap();
    let projected = selector.apply_arrow_batch(&batch).unwrap();
    assert_eq!(projected.num_columns(), 2);
    assert!(Arc::ptr_eq(projected.column(0), batch.column(3)));
    assert!(Arc::ptr_eq(projected.column(1), batch.column(0)));
    let everything = Selector::all().apply_arrow_batch(&batch).unwrap();
    for (left, right) in batch.columns().iter().zip(everything.columns()) {
        assert!(Arc::ptr_eq(left, right));
    }
}

#[test]
fn a_declared_column_is_enforced_when_it_is_applied() {
    let schema = rows_schema();
    let rows = rows();
    let batch = batch_of(&schema, &rows);
    // A required column that computes a null is refused by name.
    let required: Selector = "s as name string not null".parse().unwrap();
    let error = required.apply_arrow_batch(&batch).unwrap_err().to_string();
    assert!(error.contains("name"), "{error}");
    assert!(required.apply_scalar(&schema, &rows[2]).is_err());
    assert_eq!(
        required.apply_scalar(&schema, &rows[0]).unwrap(),
        Scalar::from_sequence([Scalar::from("alpha")])
    );
    // A declared type the value does not fit answers null - the best-effort
    // reading - unless the column is required, where the value is refused
    // by name.
    let narrow: Selector = "n as small int8".parse().unwrap();
    assert_eq!(
        narrow
            .apply_arrow_batch(&batch)
            .unwrap()
            .column(0)
            .null_count(),
        batch.num_rows()
    );
    assert_eq!(
        narrow.apply_scalar(&schema, &rows[0]).unwrap(),
        Scalar::from_sequence([Scalar::Null])
    );
    let required: Selector = "n as small int8 not null".parse().unwrap();
    let error = required.apply_arrow_batch(&batch).unwrap_err().to_string();
    assert!(error.contains("small"), "{error}");
    assert!(required.apply_scalar(&schema, &rows[0]).is_err());
    let fits: Selector = "i as small int8".parse().unwrap();
    assert_eq!(
        fits.apply_scalar(&schema, &rows[0]).unwrap(),
        Scalar::from_sequence([Scalar::from(1_i8)])
    );
}

#[test]
fn a_field_holds_a_selector_and_gives_it_back() {
    let schema = rows_schema();
    let selector: Selector = "i, i + 1 as next, lower(s) as name".parse().unwrap();
    let plan = selector.into_field(&schema).unwrap();
    assert_eq!(plan.fields()[0].get_metadata("transform:expression"), None);
    assert_eq!(
        plan.fields()[1].get_metadata("transform:expression"),
        Some("i + 1")
    );
    // A call over plain columns is stored as the function and its sources,
    // the shape a signature and a partition spec share.
    assert_eq!(plan.fields()[2].get_metadata("transform:expression"), None);
    assert_eq!(
        plan.fields()[2].get_metadata("transform:function"),
        Some("lower")
    );
    assert_eq!(
        plan.fields()[2].get_metadata("transform:sources"),
        Some(r#"["s"]"#)
    );
    assert!(plan.as_transform().declares_derivation());

    // The reading is the `create table` one: every column with its type.
    let read = Selector::from_field(&plan);
    assert_eq!(
        read.to_string(),
        "i int64 null, i + 1 as next int64 null, lower(s) as name utf8 null"
    );
    let shape = |field: &Field| -> Vec<(String, DataType, bool)> {
        field
            .fields()
            .iter()
            .map(|child| {
                (
                    child.name().to_owned(),
                    child.dtype().clone(),
                    child.is_nullable(),
                )
            })
            .collect()
    };
    assert_eq!(shape(&read.apply_field(&schema).unwrap()), shape(&plan));
    // Applying the plan derives the same columns the selector computes.
    let batch = batch_of(&schema, &rows());
    let derived = plan.as_transform().apply_arrow_batch(&batch).unwrap();
    let projected = selector.apply_arrow_batch(&batch).unwrap();
    assert_eq!(
        derived.column_by_name("next").unwrap(),
        projected.column_by_name("next").unwrap()
    );
    assert_eq!(
        derived.column_by_name("name").unwrap(),
        projected.column_by_name("name").unwrap()
    );
    // A stored declaration is canonical text, and a malformed one is refused.
    let mut broken = plan.fields()[1].clone();
    broken
        .insert_metadata("transform:expression", "i +")
        .unwrap_err();
    assert_eq!(broken.get_metadata("transform:expression"), Some("i + 1"));
    broken
        .insert_metadata("transform:expression", "I   +  2")
        .unwrap();
    assert_eq!(broken.get_metadata("transform:expression"), Some("I + 2"));
}

#[test]
fn a_partition_declaration_is_a_transform() {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut().set_sources(["event"]).unwrap();
    year.as_partition_mut()
        .set_transform(yggdryl::expression::Function::Year)
        .unwrap();
    assert!(year.as_transform().is_derived());
    assert_eq!(
        year.as_transform()
            .term()
            .unwrap()
            .map(|term| term.to_string()),
        Some("year(event)".to_owned())
    );
    // An explicit term answers first, so a plan can override the pair.
    year.as_transform_mut()
        .set_term(&"year(event) + 1".parse().unwrap())
        .unwrap();
    assert_eq!(
        year.as_transform()
            .term()
            .unwrap()
            .map(|term| term.to_string()),
        Some("year(event) + 1".to_owned())
    );
    assert_eq!(
        year.as_transform_mut().remove_term(),
        Some("year(event) + 1".to_owned())
    );
    assert!(year.as_transform().is_derived());
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[test]
fn a_filter_keeps_the_rows_it_answers_true_for() {
    let schema = rows_schema();
    let rows = rows();
    let batch = batch_of(&schema, &rows);
    let filter: Filter = "i > 0 or s = ''".parse().unwrap();
    assert_eq!(filter.apply_field(&schema).unwrap(), schema);
    let kept = filter.apply_arrow_batch(&batch).unwrap();
    assert_eq!(kept.num_rows(), 3);
    let expected: Vec<bool> = rows
        .iter()
        .map(|row| filter.apply_scalar(&schema, row).unwrap())
        .collect();
    assert_eq!(expected, vec![true, false, false, true, true]);
    // A filter over a bare batch binds against the batch's own schema.
    let expression: Expression = "where i is null".parse().unwrap();
    assert_eq!(expression.apply_arrow_batch(&batch).unwrap().num_rows(), 1);
    assert_eq!(expression.apply_field(&schema).unwrap(), schema);
}

#[test]
fn a_filter_has_to_be_a_predicate() {
    let schema = rows_schema();
    let filter: Filter = "i + 1".parse().unwrap();
    let error = filter.apply_field(&schema).unwrap_err().to_string();
    assert!(error.contains("boolean"), "{error}");
    assert!(filter.bind(&schema).is_err());
    assert!(Filter::always_true().is_always_true());
    assert!(Filter::always_false().is_always_false());
    assert!(Filter::all([]).is_always_true());
    assert!(Filter::any([]).is_always_false());
}

#[test]
fn a_mask_that_keeps_everything_keeps_the_batch_itself() {
    let schema = rows_schema();
    let batch = batch_of(&schema, &rows());
    let bound = "n = 2024 or n <> 2024 or n is null"
        .parse::<Term>()
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
fn a_reader_filters_and_projects_in_one_pass() {
    let schema = rows_schema();
    let batch = batch_of(&schema, &rows());
    let arrow_schema = batch.schema();
    let filter: Expression = "where i is not null".parse().unwrap();
    let selector: Expression = "select i".parse().unwrap();
    let reader = yggdryl::arrow::batch_reader(arrow_schema, [batch]);
    let reader = selector
        .apply_arrow_reader(filter.apply_arrow_reader(reader).unwrap())
        .unwrap();
    assert_eq!(reader.schema().fields().len(), 1);
    let rows: usize = reader.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 4);
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

impl yggdryl::IOMedia for Counting {
    yggdryl::impl_default_iomedia!();
}

impl yggdryl::IOBase for Counting {
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
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let handle = Counting::new("file:///lake/year=2024/part-0.parquet");
    assert!(
        !bound
            .matches_holder(&yggdryl::expression::Handle(&handle))
            .unwrap()
    );
    assert_eq!(
        handle.stats.get(),
        0,
        "a predicate settled by the path still cost a stat"
    );

    // A holder the free test does not rule out pays for the stat, once.
    let matching = Counting::new("file:///lake/year=2023/part-0.parquet");
    assert!(
        bound
            .matches_holder(&yggdryl::expression::Handle(&matching))
            .unwrap()
    );
    assert_eq!(matching.stats.get(), 1);
}

#[test]
fn a_row_predicate_rules_no_holder_out() {
    let schema = rows_schema();
    let bound = "i > 1000000"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let handle = Counting::new("file:///lake/part-0.parquet");
    assert!(
        bound
            .matches_holder(&yggdryl::expression::Handle(&handle))
            .unwrap(),
        "a listing filter may never discard a file it has not read"
    );
}

#[test]
fn every_attribute_declares_a_cost_and_a_type() {
    for attribute in Attribute::ALL {
        let field = attribute.field();
        assert!(field.is_nullable());
        assert_eq!(field.name(), format!("&holder.{attribute}"));
        // A free attribute is answerable from a URL alone; a stat one is not.
        let url = Url::from_str("file:///lake/year=2024/part-0.parquet").unwrap();
        let answered = attribute.read_url(&url);
        assert_eq!(
            answered.is_null(),
            matches!(attribute.cost(), Cost::Stat),
            "{attribute} disagreed with its own cost class"
        );
    }
    let partition = Attribute::Partition("year".into());
    assert_eq!(partition.cost(), Cost::Free);
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
        let bound: Bound = text.parse::<Term>().unwrap().bind(&schema).unwrap();
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
    for text in [
        "i > 1000",
        "i < -1000",
        "n = 1999",
        "s > 'zzz'",
        "n = 1999 or n = 1998",
        "not (n >= 2000)",
    ] {
        let bound = text.parse::<Term>().unwrap().bind(&schema).unwrap();
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
    let keep = "n = 2024".parse::<Term>().unwrap().bind(&schema).unwrap();
    let skip = "n = 2023".parse::<Term>().unwrap().bind(&schema).unwrap();
    assert!(keep.statistics_prune(&bounds));
    assert!(!skip.statistics_prune(&bounds));
}

#[test]
fn a_split_conjoins_back_to_what_it_split() {
    let schema = rows_schema();
    let bound = "n = 2024 and i > 1 and s like 'a%'"
        .parse::<Term>()
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
    let mut left: Vec<Term> = rejoined
        .conjuncts()
        .into_iter()
        .map(Filter::into_term)
        .collect();
    let mut right = bound.term().conjuncts();
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
                // The fixture orders values the way the scalar itself does;
                // the rows below hold one width per column, so the datatype
                // directed ordering the pushdown uses agrees with it.
                Some(held) => match Some(value.cmp(held)) {
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
        let bound = text.parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(
            bound.eval(&rows()[0]).unwrap(),
            Scalar::from(expected),
            "{text}"
        );
    }
    let bound = "substring(s, 1, -1)"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert!(bound.eval(&rows()[0]).is_err());
}

#[test]
fn a_pattern_with_no_wildcard_becomes_an_equality() {
    let schema = rows_schema();
    let bound = "s like 'alpha'"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.term().to_string(), "s = 'alpha'");
    assert!(bound.matches(&rows()[0]).unwrap());
    // An escaped wildcard is a literal, so it folds too.
    let escaped = "s like 'a!%b' escape '!'"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(escaped.term().to_string(), "s = 'a%b'");
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
    let error = "value = 1".parse::<Term>().unwrap().bind(&schema);
    let message = format!("{}", error.unwrap_err());
    assert!(message.contains("one column"), "{message}");
    assert!(message.contains("quote the one meant"), "{message}");
}

#[test]
fn an_exact_quotient_keeps_room_to_be_a_quotient() {
    let schema = rows_schema();
    let bound = "d / decimal128(9,2) '3.00'"
        .parse::<Term>()
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
            Field::new("ccy", DataType::utf8(), true),
            Field::new("price", DataType::decimal128(9, 2).unwrap(), true),
            Field::new("size", DataType::Int32, true),
        ])
        .unwrap(),
        false,
    );
    let bound = "ccy = 'EUR' and price > 100 and size is not null"
        .parse::<Term>()
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
fn a_struct_term_produces_and_reprints_a_row_sequence() {
    let schema = rows_schema();
    let bound = "struct(1 as id, 'XNAS' as venue)"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let expected = Scalar::from_sequence([Scalar::from(1), Scalar::from("XNAS")]);
    assert_eq!(bound.eval(&rows()[0]).unwrap(), expected);

    // Constant folding retains the datatype on the Literal rather than on the
    // row. Display must use that schema to reconstruct the named term.
    let printed = bound.term().to_string();
    assert!(printed.contains("struct("), "{printed}");
    let reparsed = printed.parse::<Term>().unwrap().bind(&schema).unwrap();
    assert_eq!(reparsed.eval(&rows()[0]).unwrap(), expected);
}

#[test]
fn unknown_is_not_true() {
    let schema = rows_schema();
    let bound = "i > 1".parse::<Term>().unwrap().bind(&schema).unwrap();
    let row = &rows()[2];
    assert_eq!(bound.eval(row).unwrap(), Scalar::Null);
    assert!(!bound.matches(row).unwrap());
}

#[test]
fn a_literal_is_converted_once_into_the_column_it_meets() {
    let schema = rows_schema();
    let bound = "d > 100".parse::<Term>().unwrap().bind(&schema).unwrap();
    // The bound term prints the literal in the column's own type, which is
    // how a caller sees that the comparison is exact rather than floating.
    assert_eq!(bound.term().to_string(), "d > decimal128(9,2) '100.00'");
}

#[test]
fn a_constant_subtree_is_folded_by_evaluating_it() {
    let schema = rows_schema();
    let bound = "i > 2 * 3 + 1"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.term().to_string(), "i > 7");
}

#[test]
fn binding_simplifies_before_it_lowers() {
    let schema = rows_schema();
    let bound = "i = 1 or i = 2 or i = 3"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.term().to_string(), "i in (1, 2, 3)");
    let bound = "not (i is null) and true"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.term().to_string(), "i is not null");
}

#[test]
fn parameters_are_supplied_at_bind_and_never_again() {
    let schema = rows_schema();
    let term: Term = "i >= :floor".parse().unwrap();
    assert_eq!(term.parameters(), vec!["floor".to_owned()]);
    assert!(term.bind(&schema).is_err());
    let bound = term
        .bind_with(&schema, &[("floor", Scalar::from(100))])
        .unwrap();
    assert_eq!(bound.term().to_string(), "i >= 100");
    assert!(bound.matches(&rows()[3]).unwrap());
}

#[test]
fn an_unknown_column_names_the_ones_there_are() {
    let schema = rows_schema();
    let error = "nope = 1".parse::<Term>().unwrap().bind(&schema);
    let message = format!("{}", error.unwrap_err());
    assert!(message.contains("nope"), "{message}");
    assert!(message.contains('i'), "{message}");
}

#[test]
fn operands_meet_in_the_column_type_or_as_text() {
    let schema = rows_schema();
    // A constant the column holds exactly is read in the column's type.
    let bound = "i = '1'".parse::<Term>().unwrap().bind(&schema).unwrap();
    assert_eq!(bound.term().to_string(), "i = 1");
    assert!(bound.matches(&rows()[0]).unwrap());
    let bound = "t > '2023-11-14T22:13:20Z'"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert!(bound.matches(&rows()[4]).unwrap());
    assert!(!bound.matches(&rows()[1]).unwrap());
    // A number against text compares as text rather than being refused.
    let bound = "s > 1".parse::<Term>().unwrap().bind(&schema).unwrap();
    assert_eq!(bound.term().to_string(), "s > '1'");
    assert!(bound.matches(&rows()[0]).unwrap());
    // A constant subtree is settled before the column type is chosen, so
    // the column is never cast to meet it.
    let bound = "n > 2 * 1000"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.term().to_string(), "n > int32 '2000'");
}

#[test]
fn cheapest_first_is_stable_when_costs_tie() {
    let schema = rows_schema();
    let bound = "s = 'a' and i = 1"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.term().to_string(), "s = 'a' and i = 1");
}

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

#[test]
fn a_literal_holds_what_its_datatype_stores() {
    let narrowed = Literal::new(DataType::Int32, 7_i64).unwrap();
    assert_eq!(narrowed.dtype(), &DataType::Int32);
    assert_eq!(narrowed.value(), &Scalar::from(7_i32));
    assert!(!narrowed.is_null());
    assert_eq!(narrowed.to_string(), "int32 '7'");
    assert_eq!(
        narrowed.clone().into_parts(),
        (DataType::Int32, Scalar::from(7_i32))
    );

    let refused = Literal::new(DataType::Int8, 1_000_i64)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("int8"), "{refused}");
    assert!(Literal::new(DataType::Int64, "seven").is_err());

    let inferred = Literal::infer(Scalar::from("AAPL")).unwrap();
    assert_eq!(inferred.dtype(), &DataType::utf8());
    assert_eq!(inferred.to_string(), "'AAPL'");
    let mixed = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
    assert!(Literal::infer(mixed.clone()).is_err());
    // The term constructor holds what it cannot type as the null it is.
    assert_eq!(Term::literal(mixed).as_literal(), Some(&Literal::null()));

    let null = Literal::new(DataType::Int64, Scalar::Null).unwrap();
    assert!(null.is_null());
    assert_eq!(null.to_string(), "int64 null");
    assert_eq!(Literal::null().dtype(), &DataType::Null);
}

#[test]
fn literals_order_by_datatype_then_value_and_serialize_as_both_halves() {
    let first = Literal::new(DataType::Int32, 7).unwrap();
    let later_value = Literal::new(DataType::Int32, 8).unwrap();
    let later_type = Literal::new(DataType::Int64, 7).unwrap();
    assert!(first < later_value);
    assert!(first < later_type);
    assert_eq!(first, first.clone());

    let encoded = serde_json::to_vec(&first).unwrap();
    assert_eq!(serde_json::from_slice::<Literal>(&encoded).unwrap(), first);
    let term = Term::Literal(first.clone());
    let encoded = serde_json::to_vec(&term).unwrap();
    assert_eq!(serde_json::from_slice::<Term>(&encoded).unwrap(), term);

    // A literal that never agreed is refused on the way in, not stored.
    let contradiction = br#"{"dtype":{"type":"int64"},"value":{"type":"string","value":"seven"}}"#;
    assert!(serde_json::from_slice::<Literal>(contradiction).is_err());
    // A literal is read back through the value contract, so it holds what
    // the datatype stores even when the text spelled it wider.
    let widened = br#"{"dtype":{"type":"int32"},"value":{"type":"i64","value":7}}"#;
    let literal = serde_json::from_slice::<Literal>(widened).unwrap();
    assert_eq!(literal.value(), &Scalar::from(7_i32));
}
