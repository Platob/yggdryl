//! `rust/src/expression/typing.rs`: the edge cases this module is built to
//! get right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, and a pruning decision never loses a
//! row.

mod grammar {

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
                    DataType::DateTime64 {
                        unit: TimeUnit::Microsecond,
                        timezone: Timezone::UTC,
                    },
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
                // A serie, so a position and a run are compared on both tiers.
                Field::new(
                    "xs",
                    DataType::serie(DataType::Int64.nullable_field("item")),
                    true,
                ),
                // A serie of structs holding a serie of structs, so a predicate
                // segment and one nested in another are compared on both tiers.
                Field::new("legs", DataType::serie(leg_field()), true),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        )
    }

    /// One leg: a currency, a size, and notes that are themselves a serie of
    /// structs.
    fn leg_field() -> Field {
        StructType::from_fields([
            DataType::utf8().nullable_field("ccy"),
            DataType::Int64.nullable_field("size"),
            DataType::serie(
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
        let serie =
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
                serie(&[1, 2, 3]),
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
                serie(&[]),
                // An empty serie keeps nothing and is not null.
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
                // A null serie stays null through every predicate.
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
                serie(&[7]),
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
                serie(&[0, -1]),
                Scalar::from_sequence([
                    leg(Some("eur"), Some(10), Some(&[("c", 3)])),
                    leg(Some("GBP"), Some(0), Some(&[("z", 0)])),
                ]),
            ]),
        ]
    }

    #[test]
    fn a_run_of_a_serie_is_typed_and_bounded_like_the_grammar_says() {
        let schema = rows_schema();
        let rows = rows();
        let middle = "xs[1:3]".parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(middle.field().dtype(), &schema.fields()[9].dtype().clone());
        let serie =
            |items: &[i64]| Scalar::from_sequence(items.iter().map(|item| Scalar::from(*item)));
        assert_eq!(middle.eval(&rows[0]).unwrap(), serie(&[2, 3]));
        assert_eq!(middle.eval(&rows[1]).unwrap(), serie(&[]));
        assert_eq!(middle.eval(&rows[2]).unwrap(), Scalar::Null);
        assert_eq!(middle.eval(&rows[3]).unwrap(), serie(&[]));
        let tail = "xs[-2:]".parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(tail.eval(&rows[0]).unwrap(), serie(&[2, 3]));
        assert_eq!(tail.eval(&rows[3]).unwrap(), serie(&[7]));
        let last = "xs[-1]".parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(last.eval(&rows[0]).unwrap(), Scalar::from(3_i64));
        assert_eq!(last.eval(&rows[1]).unwrap(), Scalar::Null);
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
}

mod fixed_leaves {
    //! Arithmetic over a fixed decimal leaf types as the leaf, by the rule
    //! `Scalar` arithmetic states.

    use yggdryl::expression::Term;
    use yggdryl::{DataType, Field, StructType};

    fn schema() -> Field {
        StructType::from_fields([
            Field::new("px", DataType::Decimal, true),
            Field::new("qty", DataType::Decimal, false),
            Field::new("n", DataType::BigDecimal, true),
            Field::new("i", DataType::Int64, false),
            Field::new("u", DataType::UInt64, false),
            Field::new("d", DataType::decimal128(9, 2).unwrap(), false),
            Field::new("w", DataType::decimal256(76, 0).unwrap(), false),
            Field::new("f", DataType::Float64, false),
            Field::new("s", DataType::utf8(), false),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    #[test]
    fn a_fixed_leaf_keeps_itself_and_the_wide_side_wins() {
        let schema = schema();
        for (text, expected) in [
            ("px * qty", DataType::Decimal),
            ("px + qty", DataType::Decimal),
            ("px - 1", DataType::Decimal),
            ("px / qty", DataType::Decimal),
            ("px % qty", DataType::Decimal),
            ("px * i", DataType::Decimal),
            ("u + px", DataType::Decimal),
            ("px * d", DataType::Decimal),
            ("-px", DataType::Decimal),
            ("n + 1", DataType::BigDecimal),
            ("n * n", DataType::BigDecimal),
            ("px * n", DataType::BigDecimal),
            ("px + w", DataType::BigDecimal),
            ("w % px", DataType::BigDecimal),
        ] {
            let field = text.parse::<Term>().unwrap().field(&schema).unwrap();
            assert_eq!(field.dtype(), &expected, "{text}");
        }
        // Nullability is the operands', as for every arithmetic.
        let field = "qty * i".parse::<Term>().unwrap().field(&schema).unwrap();
        assert!(!field.is_nullable());
        // An approximate or a non-numeric operand meets no fixed leaf.
        for text in ["px * f", "n + 1.5", "px + s"] {
            assert!(
                text.parse::<Term>().unwrap().field(&schema).is_err(),
                "{text}"
            );
        }
    }
}
