//! `rust/src/expression/eval.rs`: the value cast no caller can name.
//!
//! `convert` is the crate-private cast every `cast` operator and every bound
//! projection goes through; what it answers per target leaf, and what it
//! refuses, is the contract, so it is reached through `yggdryl::internals`.
//! What a caller can observe is beside it here and in the other files under
//! `rust/tests/expression/`.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::expression::Safety;
    use yggdryl::internals::expression_eval::{convert, order};
    use yggdryl::{DataType, DataTypeId, Scalar, StructType, Version};

    #[test]
    fn scalar_casts_return_the_exact_target_leaf() {
        let cases = [
            (
                DataType::decimal32(9, 2).unwrap(),
                Scalar::from(125),
                DataTypeId::Decimal32,
            ),
            (
                DataType::decimal64(18, 2).unwrap(),
                Scalar::from(125),
                DataTypeId::Decimal64,
            ),
            (
                DataType::Float32,
                Scalar::from(1.5_f64),
                DataTypeId::Float32,
            ),
            (
                DataType::large_utf8(),
                Scalar::from("value"),
                DataTypeId::LargeUtf8String,
            ),
            (
                DataType::utf8_view(),
                Scalar::from("value"),
                DataTypeId::Utf8StringView,
            ),
            (
                DataType::large_binary(),
                Scalar::from("value"),
                DataTypeId::LargeBinary,
            ),
            (
                DataType::binary_view(),
                Scalar::from("value"),
                DataTypeId::BinaryView,
            ),
            (
                DataType::fixed_ascii(4).unwrap(),
                Scalar::from("FIX"),
                DataTypeId::FixedAsciiString,
            ),
            (
                DataType::Currency,
                Scalar::from("USD"),
                DataTypeId::Currency,
            ),
        ];

        for (target, input, id) in cases {
            let converted = convert(&target, &input, Safety::Strict).unwrap();
            assert_eq!(converted.id(), id, "cast to {target}");
        }
    }

    #[test]
    fn versions_do_not_fall_through_text_or_numeric_expression_paths() {
        let patch2 = Scalar::from("5.0.2".parse::<Version>().unwrap());
        let patch10 = Scalar::from("5.0.10".parse::<Version>().unwrap());

        assert_eq!(
            convert(
                &DataType::Version,
                &Scalar::from("005.000.002"),
                Safety::Strict
            )
            .unwrap(),
            patch2
        );
        assert_eq!(
            convert(&DataType::Version, &patch2, Safety::Strict).unwrap(),
            patch2
        );
        assert_eq!(
            convert(&DataType::utf8(), &patch2, Safety::Strict).unwrap(),
            Scalar::from("5.0.2")
        );
        assert!(convert(&DataType::Version, &Scalar::from(5), Safety::Strict).is_err());
        assert_eq!(
            order(&DataType::Version, &patch2, &patch10),
            Some(std::cmp::Ordering::Less)
        );

        let schema = StructType::from_fields([DataType::Version.required_field("v")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let row = Scalar::from_sequence([patch2]);
        for (text, expected) in [
            ("v < version '5.0.10'", Scalar::from(true)),
            ("length(v)", Scalar::from(5_i64)),
            ("v like '5.0%'", Scalar::from(true)),
        ] {
            let answer = text
                .parse::<yggdryl::expression::Term>()
                .unwrap()
                .bind(&schema)
                .unwrap()
                .eval(&row)
                .unwrap();
            assert_eq!(answer, expected, "{text}");
        }
    }
}

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
            DataType::from(
                StructType::from_fields([Field::new("small", DataType::Int8, false)]).unwrap(),
            ),
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
    fn unknown_is_not_true() {
        let schema = rows_schema();
        let bound = "i > 1".parse::<Term>().unwrap().bind(&schema).unwrap();
        let row = &rows()[2];
        assert_eq!(bound.eval(row).unwrap(), Scalar::Null);
        assert!(!bound.matches(row).unwrap());
    }
}
