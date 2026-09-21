//! `rust/src/expression/bind.rs`: the compile step - what binding decides
//! before the first row, and what it refuses there.

mod grammar {

    use yggdryl::DateTimeType;
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Field, Scalar, StructType, TimeUnit, Timezone};

    // ---------------------------------------------------------------------------
    // The shared fixture
    // ---------------------------------------------------------------------------

    /// A schema that covers one column of every family a comparison can meet.
    fn rows_schema() -> Field {
        Field::new(
            "rows",
            StructType::from_fields([
                Field::new("i", DataType::Int64, true),
                Field::new("f", DataType::Float64, true),
                Field::new("d", DataType::decimal128(9, 2).unwrap(), true),
                Field::new("s", DataType::utf8(), true),
                Field::new("b", DataType::Boolean, true),
                Field::new(
                    "t",
                    DataType::DateTime(DateTimeType::DateTime64 {
                        unit: TimeUnit::Microsecond,
                        timezone: Timezone::UTC,
                    }),
                    true,
                ),
                Field::new("n", DataType::Int32, true).with_partition(true),
                Field::new(
                    "nested",
                    DataType::from(
                        StructType::from_fields([Field::new("leg", DataType::utf8(), true)])
                            .unwrap(),
                    ),
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
                // A list of structs holding a list of structs, so a predicate
                // segment and one nested in another are compared on both tiers.
                Field::new("legs", DataType::list(leg_field()), true),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        )
    }

    /// One leg: a currency, a size, and notes that are themselves a list of
    /// structs.
    fn leg_field() -> Field {
        StructType::from_fields([
            DataType::utf8().nullable_field("ccy"),
            DataType::Int64.nullable_field("size"),
            DataType::list(
                StructType::from_fields([
                    DataType::utf8().nullable_field("k"),
                    DataType::Int64.nullable_field("v"),
                ])
                .map(DataType::from)
                .unwrap()
                .nullable_field("item"),
            )
            .nullable_field("notes"),
        ])
        .map(DataType::from)
        .unwrap()
        .nullable_field("item")
    }

    fn leg(ccy: Option<&str>, size: Option<i64>, notes: Option<&[(&str, i64)]>) -> Scalar {
        Scalar::from_sequence([
            ccy.map_or(Scalar::Null, Scalar::from),
            size.map_or(Scalar::Null, Scalar::from),
            notes.map_or(Scalar::Null, |notes| {
                Scalar::from_sequence(
                    notes
                        .iter()
                        .map(|(k, v)| Scalar::from_sequence([Scalar::from(*k), Scalar::from(*v)])),
                )
            }),
        ])
    }

    /// Rows chosen so every operator meets a null, a `nan`, and a boundary.
    fn rows() -> Vec<Scalar> {
        let stamp =
            |micros: i64| Scalar::datetime64(micros, TimeUnit::Microsecond, Timezone::UTC).unwrap();
        let nested =
            |leg: Option<&str>| Scalar::from_sequence([leg.map_or(Scalar::Null, Scalar::from)]);
        let list =
            |items: &[i64]| Scalar::from_sequence(items.iter().map(|item| Scalar::from(*item)));
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
                Scalar::from_sequence([
                    leg(Some("EUR"), Some(1), Some(&[("a", 1), ("b", 2)])),
                    leg(Some("USD"), Some(2), Some(&[])),
                    leg(Some("EUR"), Some(3), None),
                ]),
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
                // An empty list keeps nothing and is not null.
                Scalar::from_sequence([]),
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
                // A null list stays null through every predicate.
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
                // A null element is dropped; a null size makes a size test unknown.
                Scalar::from_sequence([Scalar::Null, leg(Some("EUR"), None, Some(&[("a", 5)]))]),
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
                Scalar::from_sequence([
                    leg(Some("eur"), Some(10), Some(&[("c", 3)])),
                    leg(Some("GBP"), Some(0), Some(&[("z", 0)])),
                ]),
            ]),
        ]
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

    #[test]
    fn a_pattern_that_changes_per_row_is_refused_at_bind() {
        let schema = rows_schema();
        let error = "s like s".parse::<Term>().unwrap().bind(&schema);
        let message = format!("{}", error.unwrap_err());
        assert!(message.contains("constant"), "{message}");
    }

    #[test]
    fn a_parameter_inside_a_predicate_is_supplied_at_bind() {
        let schema = rows_schema();
        let rows = rows();
        let term: Term = "legs[ccy = :ccy and size >= :floor][0].size"
            .parse()
            .unwrap();
        assert_eq!(
            term.parameters(),
            vec!["ccy".to_owned(), "floor".to_owned()]
        );
        assert!(
            term.bind(&schema).is_err(),
            "a parameter has to be supplied"
        );
        let bound = term
            .bind_with(
                &schema,
                &[("ccy", Scalar::from("EUR")), ("floor", Scalar::from(2_i64))],
            )
            .unwrap();
        assert_eq!(
            bound.term().to_string(),
            "legs[ccy = 'EUR' and size >= 2][0].size"
        );
        assert_eq!(bound.eval(&rows[0]).unwrap(), Scalar::from(3_i64));
        let batch = batch_of(&schema, &rows);
        let column = bound.evaluate(&batch).unwrap();
        assert_eq!(
            yggdryl::arrow::scalar_value(
                &bound.field().clone().with_nullable(true),
                column.slice(0, 1).as_ref()
            )
            .unwrap(),
            Scalar::from(3_i64)
        );
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
            StructType::from_fields([
                Field::new("Value", DataType::Int64, true),
                Field::new("value", DataType::Int64, true),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        );
        let error = "value = 1".parse::<Term>().unwrap().bind(&schema);
        let message = format!("{}", error.unwrap_err());
        assert!(message.contains("one column"), "{message}");
        assert!(message.contains("quote the one meant"), "{message}");
    }

    #[test]
    fn binds_and_evaluates_rows() {
        let schema = Field::new(
            "trades",
            StructType::from_fields([
                Field::new("ccy", DataType::utf8(), true),
                Field::new("price", DataType::decimal128(9, 2).unwrap(), true),
                Field::new("size", DataType::Int32, true),
            ])
            .map(DataType::from)
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
}
