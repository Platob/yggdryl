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
            (DataType::Ccy, Scalar::from("USD"), DataTypeId::Ccy),
        ];

        for (target, input, id) in cases {
            let converted = convert(&target, &input, Safety::Strict).unwrap();
            assert_eq!(converted.id(), id, "cast to {target}");
        }
    }

    #[test]
    fn text_enters_a_flag_a_number_or_a_decimal_as_a_column_of_text_does() {
        let strict =
            |target: &DataType, text: &str| convert(target, &Scalar::from(text), Safety::Strict);
        let safe =
            |target: &DataType, text: &str| convert(target, &Scalar::from(text), Safety::Safe);

        // A flag reads the one table the value door and a column cast read.
        for (text, expected) in [
            ("yes", true),
            (" TRUE ", true),
            ("Y", true),
            ("1", true),
            ("no", false),
            ("n", false),
            ("Off", false),
            ("0", false),
        ] {
            assert_eq!(
                strict(&DataType::Boolean, text).unwrap(),
                Scalar::from(expected),
                "{text}"
            );
        }
        assert!(strict(&DataType::Boolean, "maybe").is_err());
        assert_eq!(safe(&DataType::Boolean, "maybe").unwrap(), Scalar::Null);
        // A code is its registry, never a flag: `NO` is Norway.
        let norway = Scalar::from(yggdryl::Country::new("NO").unwrap());
        assert!(convert(&DataType::Boolean, &norway, Safety::Strict).is_err());

        // An integer reads its signed, trimmed spelling at the declared width.
        assert_eq!(
            strict(&DataType::Int32, " 5 ").unwrap(),
            Scalar::from(5_i32)
        );
        assert_eq!(strict(&DataType::Int64, "+7").unwrap(), Scalar::from(7_i64));
        for text in ["1.0", "x", "99999999999"] {
            assert!(strict(&DataType::Int32, text).is_err(), "{text}");
            assert_eq!(
                safe(&DataType::Int32, text).unwrap(),
                Scalar::Null,
                "{text}"
            );
        }
        // A code is its registry, never a number: the digits of a CUSIP are
        // not the integer they spell.
        let apple = Scalar::from(yggdryl::Cusip::new("037833100").unwrap());
        assert!(convert(&DataType::Int64, &apple, Safety::Strict).is_err());

        // A float reads what a column of text is cast through, the
        // infinities included.
        assert_eq!(
            strict(&DataType::Float64, " 1.5 ").unwrap(),
            Scalar::from(1.5_f64)
        );
        assert_eq!(
            strict(&DataType::Float32, "1e3").unwrap(),
            Scalar::from(1000.0_f32)
        );
        assert_eq!(
            strict(&DataType::Float64, "inf").unwrap(),
            Scalar::from(f64::INFINITY)
        );
        assert!(strict(&DataType::Float64, "x").is_err());
        assert_eq!(safe(&DataType::Float64, "x").unwrap(), Scalar::Null);

        // A decimal reads its exponent form at the declared scale, and a digit
        // past the scale or the precision is refused, never rounded.
        let price = DataType::decimal128(10, 2).unwrap();
        assert_eq!(strict(&price, "1.50").unwrap(), Scalar::decimal128(150, 2));
        assert_eq!(
            strict(&price, "1e2").unwrap(),
            Scalar::decimal128(10_000, 2)
        );
        assert_eq!(strict(&price, " 1.5 ").unwrap(), Scalar::decimal128(150, 2));
        assert!(strict(&price, "1.555").is_err());
        assert_eq!(safe(&price, "1.555").unwrap(), Scalar::Null);
        assert!(strict(&DataType::decimal128(5, 2).unwrap(), "1234.56").is_err());
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

    use yggdryl::expression::{Selector, Term};
    use yggdryl::{DataType, Field, Scalar, Serie, StructType, TimeUnit, Timezone};

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
                Scalar::decimal128(150, 2),
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
                Scalar::decimal128(-25, 2),
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
                Scalar::decimal128(10_000, 2),
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
                Scalar::decimal128(0, 2),
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

    // ---------------------------------------------------------------------------
    // A serie held as a column
    // ---------------------------------------------------------------------------

    /// `row` with the serie at `index` replaced by a column of `items` under
    /// `item`.
    fn with_column(row: &Scalar, index: usize, item: Field, items: Vec<Scalar>) -> Scalar {
        let mut cells = row.sequence_rows().unwrap().into_owned();
        cells[index] = Scalar::from(Serie::from_scalars(item, items).unwrap());
        Scalar::from_sequence(cells)
    }

    #[test]
    fn slice_of_a_column_is_a_window_of_it() {
        let schema = rows_schema();
        let row = with_column(
            &rows()[0],
            9,
            DataType::Int64.nullable_field("item"),
            (1..=4_i64).map(Scalar::from).collect(),
        );
        let answer = "slice(xs, 1, 3) as s"
            .parse::<Selector>()
            .unwrap()
            .apply_scalar(&schema, &row)
            .unwrap();
        let cell = answer.sequence_rows().unwrap()[0].clone();
        assert_eq!(
            cell,
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::from(3_i64)])
        );
        assert!(
            cell.as_serie().is_some_and(Serie::is_column),
            "a window of a column is a column: {cell:?}"
        );
    }

    #[test]
    fn a_predicate_keeps_the_same_elements_of_a_column_as_of_a_run() {
        let schema = rows_schema();
        let bound = "legs[size > 1]"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        for (position, row) in rows().iter().enumerate() {
            let cells = row.sequence_rows().unwrap();
            let Some(legs) = cells[10].sequence_rows().map(|legs| legs.into_owned()) else {
                continue;
            };
            let column = with_column(row, 10, leg_field(), legs);
            assert_eq!(
                bound.eval(&column).unwrap(),
                bound.eval(row).unwrap(),
                "row {position}"
            );
        }
    }
}

mod fixed_leaves {
    //! The fixed decimal leaves read, compare and cast the same way in a row
    //! and in a batch.

    use yggdryl::expression::{Filter, Selector};
    use yggdryl::{
        ArrowCastOptions, BigDecimal, DataType, Decimal, Field, Scalar, Serie, StructType,
    };

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        StructType::from_fields(fields)
            .map(DataType::from)
            .unwrap()
            .required_field("row")
    }

    /// One row laid out as the one-row batch the batch tier reads.
    fn batch(schema: &Field, row: &Scalar) -> arrow_array::RecordBatch {
        Serie::from_scalars(schema.clone(), [row.clone()])
            .unwrap()
            .into_arrow_batch()
            .unwrap()
    }

    /// The first row a batch answered, read back as a row.
    fn first_row(batch: &arrow_array::RecordBatch) -> Scalar {
        Serie::from_arrow_batch(None, batch, ArrowCastOptions::new())
            .unwrap()
            .scalar(0)
            .unwrap()
    }

    /// What one selector answers for one row, in a row and in a batch.
    fn both_tiers(selector: &str, schema: &Field, row: &Scalar) -> (Scalar, Scalar) {
        let selector = selector.parse::<Selector>().unwrap();
        let by_row = selector.apply_scalar(schema, row).unwrap();
        let by_batch = first_row(&selector.apply_arrow_batch(&batch(schema, row)).unwrap());
        (by_row, by_batch)
    }

    #[test]
    fn a_bigdecimal_past_thirty_eight_digits_compares_in_a_row_as_in_a_batch() {
        let schema = root([Field::new("x", DataType::BigDecimal, true)]);
        let wide: BigDecimal = "123456789012345678901234567890.5".parse().unwrap();
        let row = Scalar::from_sequence([Scalar::BigDecimal(wide)]);
        for text in [
            "x > 1",
            "x = x",
            "x <> 0",
            "x between 1 and x",
            "x in (x, 1)",
        ] {
            let filter = text.parse::<Filter>().unwrap();
            assert!(
                filter.apply_scalar(&schema, &row).unwrap(),
                "{text} keeps the row"
            );
            let records = filter
                .apply_records(Some(&schema), [row.clone()])
                .unwrap()
                .collect_rows()
                .unwrap();
            assert_eq!(
                records,
                std::slice::from_ref(&row),
                "{text} keeps the record"
            );
            let kept = filter.apply_arrow_batch(&batch(&schema, &row)).unwrap();
            assert_eq!(kept.num_rows(), 1, "{text} keeps the batch's row");
        }
        let (by_row, by_batch) = both_tiers("x = x as same", &schema, &row);
        assert_eq!(by_row, Scalar::from_sequence([Scalar::from(true)]));
        assert_eq!(by_batch, by_row);
        // A cast onto its own leaf reads the value whole.
        let (by_row, by_batch) = both_tiers("cast(x as bigdecimal) as y", &schema, &row);
        assert_eq!(by_row, Scalar::from_sequence([Scalar::BigDecimal(wide)]));
        assert_eq!(by_batch, by_row);
        // And a `decimal256` column the same, past what an `i128` holds.
        let schema = root([Field::new("x", DataType::decimal256(76, 18).unwrap(), true)]);
        let row = Scalar::from_sequence([Scalar::decimal256(wide.units(), 18)]);
        let filter = "x > 1".parse::<Filter>().unwrap();
        assert!(filter.apply_scalar(&schema, &row).unwrap());
    }

    #[test]
    fn a_float_entering_a_decimal_is_the_number_it_names_or_refused() {
        let schema = root([Field::new("f", DataType::Float64, true)]);
        let row = |held: f64| Scalar::from_sequence([Scalar::from(held)]);
        let one = |value: Scalar| Scalar::from_sequence([value]);
        let wide = one(Scalar::BigDecimal(
            "10000000000000000000000000".parse().unwrap(),
        ));
        // Folded while binding - the literal coercion - and cast per row, in
        // a row and in a batch.
        let folded = "cast(1e25 as bigdecimal) as l".parse::<Selector>().unwrap();
        assert_eq!(folded.apply_scalar(&schema, &row(1e25)).unwrap(), wide);
        for (held, text, expected) in [
            (1e25, "cast(f as bigdecimal) as l", wide.clone()),
            // A width reads the float's own digits: 1.15 is 1.15, never the
            // 1.14 its binary product with a hundred truncates to or the
            // 1.149999999999999872 its binary fraction is.
            (
                1.15,
                "cast(f as decimal(10, 2)) as l",
                one(Scalar::decimal128(115, 2)),
            ),
            (
                1.15,
                "cast(f as decimal) as l",
                one(Scalar::Decimal("1.15".parse().unwrap())),
            ),
            (
                1.15,
                "cast(f as bigdecimal) as l",
                one(Scalar::BigDecimal("1.15".parse().unwrap())),
            ),
            // At the declared scale a float rounds half away from zero.
            (
                0.125,
                "cast(f as decimal(10, 2)) as l",
                one(Scalar::decimal128(13, 2)),
            ),
            (
                -0.125,
                "cast(f as decimal(10, 2)) as l",
                one(Scalar::decimal128(-13, 2)),
            ),
            (
                2.5,
                "cast(f as decimal(10, 0)) as l",
                one(Scalar::decimal128(3, 0)),
            ),
            (
                -2.5,
                "cast(f as decimal(10, 0)) as l",
                one(Scalar::decimal128(-3, 0)),
            ),
            (
                0.124,
                "cast(f as decimal(10, 2)) as l",
                one(Scalar::decimal128(12, 2)),
            ),
        ] {
            let (by_row, by_batch) = both_tiers(text, &schema, &row(held));
            assert_eq!(by_row, expected, "{held} {text}");
            assert_eq!(by_batch, by_row, "{held} {text}");
        }
        let tiers = |text: &str, held: f64| {
            let selector = text.parse::<Selector>().unwrap();
            (
                selector.apply_scalar(&schema, &row(held)),
                selector.apply_arrow_batch(&batch(&schema, &row(held))),
            )
        };
        // Past what the target holds is a refusal in both tiers, never a
        // saturated number; `try_cast` answers null for it.
        for (text, held) in [
            ("cast(f as decimal) as l", 1e25),
            ("cast(f as decimal256(76, 60)) as l", 1e25),
            ("cast(f as bigdecimal) as l", 1e80),
            ("cast(f as bigdecimal) as l", f64::INFINITY),
            ("cast(f as bigdecimal) as l", f64::NAN),
        ] {
            let (by_row, by_batch) = tiers(text, held);
            assert!(by_row.is_err(), "{held} {text}: {by_row:?}");
            assert!(by_batch.is_err(), "{held} {text}: {by_batch:?}");
        }
        assert!(
            "cast(1e25 as decimal) as l"
                .parse::<Selector>()
                .unwrap()
                .apply_scalar(&schema, &row(1e25))
                .is_err()
        );
        let (by_row, by_batch) = both_tiers("try_cast(f as decimal) as l", &schema, &row(1e25));
        assert_eq!(by_row, one(Scalar::Null));
        assert_eq!(by_batch, by_row);
    }

    #[test]
    fn arithmetic_over_the_leaves_answers_the_leaf_in_both_tiers() {
        let schema = root([
            Field::new("px", DataType::Decimal, true),
            Field::new("qty", DataType::Decimal, true),
            Field::new("n", DataType::BigDecimal, true),
        ]);
        let decimal = |text: &str| Scalar::Decimal(text.parse::<Decimal>().unwrap());
        let big = |text: &str| Scalar::BigDecimal(text.parse::<BigDecimal>().unwrap());
        let row = Scalar::from_sequence([
            decimal("82.5"),
            decimal("1000"),
            big("123456789012345678901234567890.5"),
        ]);
        for (text, expected) in [
            ("px * qty as v", decimal("82500")),
            ("px + 1 as v", decimal("83.5")),
            ("px - qty as v", decimal("-917.5")),
            ("qty / px as v", decimal("12.121212121212121212")),
            ("px % 2 as v", decimal("0.5")),
            ("-px as v", decimal("-82.5")),
            ("n + 1 as v", big("123456789012345678901234567891.5")),
            ("n * 2 as v", big("246913578024691357802469135781")),
            ("px * n as v", big("10185185093518518509351851850966.25")),
        ] {
            let (by_row, by_batch) = both_tiers(text, &schema, &row);
            assert_eq!(by_row, Scalar::from_sequence([expected]), "{text}");
            assert_eq!(by_batch, by_row, "{text}");
        }
        // Past thirty-eight digits the narrow leaf refuses; the wide one holds.
        let row = Scalar::from_sequence([
            decimal("10000000000000000000"),
            decimal("10000000000000000000"),
            big("10000000000000000000"),
        ]);
        assert!(
            "px * qty as v"
                .parse::<Selector>()
                .unwrap()
                .apply_scalar(&schema, &row)
                .is_err()
        );
        assert_eq!(
            both_tiers("n * n as v", &schema, &row).0,
            Scalar::from_sequence([big("100000000000000000000000000000000000000")])
        );
    }

    #[test]
    fn text_cast_into_a_flag_a_number_or_a_decimal_reads_in_a_row_as_in_a_batch() {
        let schema = root([Field::new("s", DataType::utf8(), true)]);
        let one = |value: Scalar| Scalar::from_sequence([value]);
        for (text, cell, expected) in [
            ("cast(s as boolean) as v", "yes", Scalar::from(true)),
            ("cast(s as boolean) as v", " N ", Scalar::from(false)),
            ("cast(s as int64) as v", " 7 ", Scalar::from(7_i64)),
            ("cast(s as float64) as v", " 1.5 ", Scalar::from(1.5_f64)),
            (
                "cast(s as decimal(10, 2)) as v",
                "1.50",
                Scalar::decimal128(150, 2),
            ),
            (
                "cast(s as decimal(10, 2)) as v",
                "1e2",
                Scalar::decimal128(10_000, 2),
            ),
            ("try_cast(s as boolean) as v", "maybe", Scalar::Null),
            ("try_cast(s as float64) as v", "x", Scalar::Null),
            ("try_cast(s as decimal(10, 2)) as v", "1.555", Scalar::Null),
        ] {
            let row = one(Scalar::from(cell));
            let (by_row, by_batch) = both_tiers(text, &schema, &row);
            assert_eq!(by_row, one(expected), "{text} {cell:?}");
            assert_eq!(by_batch, by_row, "{text} {cell:?}");
        }

        // A text constant compared with a column is read as the column's own
        // type when that type reads it, so `flag = 'yes'` is a boolean test
        // and `f > '10'` a numeric one, never a comparison of their spellings.
        let flags = root([Field::new("b", DataType::Boolean, true)]);
        let flag = |held: bool| Scalar::from_sequence([Scalar::from(held)]);
        let is_yes = "b = 'yes'".parse::<Filter>().unwrap();
        assert!(is_yes.apply_scalar(&flags, &flag(true)).unwrap());
        assert!(!is_yes.apply_scalar(&flags, &flag(false)).unwrap());
        let numbers = root([Field::new("f", DataType::Float64, true)]);
        let above_ten = "f > '10'".parse::<Filter>().unwrap();
        let number = |held: f64| Scalar::from_sequence([Scalar::from(held)]);
        assert!(!above_ten.apply_scalar(&numbers, &number(9.0)).unwrap());
        assert!(above_ten.apply_scalar(&numbers, &number(11.0)).unwrap());
    }

    #[test]
    fn a_leaf_cast_to_text_spells_its_one_text_in_both_tiers() {
        for (dtype, value) in [
            (DataType::Decimal, Scalar::Decimal("1.125".parse().unwrap())),
            (
                DataType::BigDecimal,
                Scalar::BigDecimal("1.125".parse().unwrap()),
            ),
        ] {
            let schema = root([Field::new("x", dtype, true)]);
            let row = Scalar::from_sequence([value]);
            let (by_row, by_batch) = both_tiers("cast(x as utf8) as t", &schema, &row);
            assert_eq!(by_row, Scalar::from_sequence([Scalar::from("1.125")]));
            assert_eq!(by_batch, by_row);
            let filter = "cast(x as utf8) = '1.125'".parse::<Filter>().unwrap();
            assert!(filter.apply_scalar(&schema, &row).unwrap());
            assert_eq!(
                filter
                    .apply_arrow_batch(&batch(&schema, &row))
                    .unwrap()
                    .num_rows(),
                1
            );
        }
    }

    /// A struct read off a column is positional, and its field names it, so
    /// a row and a batch spell the one object keyed in declaration order -
    /// and read it back.
    #[test]
    fn a_nested_cast_to_text_and_back_spells_one_json_in_both_tiers() {
        let schema = root([Field::new(
            "q",
            "struct<px: decimal128(10, 2), sym: utf8>".parse().unwrap(),
            true,
        )]);
        let row = Scalar::from_sequence([Scalar::from_sequence([
            Scalar::decimal128(150, 2),
            Scalar::from("AAPL"),
        ])]);
        let (by_row, by_batch) = both_tiers("cast(q as utf8) as t", &schema, &row);
        assert_eq!(
            by_row,
            Scalar::from_sequence([Scalar::from(r#"{"px":"1.5","sym":"AAPL"}"#)])
        );
        assert_eq!(by_batch, by_row);
        let (by_row, by_batch) = both_tiers(
            "cast(cast(q as utf8) as struct<px: decimal128(10, 2), sym: utf8>) as q",
            &schema,
            &row,
        );
        assert_eq!(by_row, row);
        assert_eq!(by_batch, by_row);

        // A dictionary of records spells the same object in both tiers.
        let encoded = root([Field::new(
            "q",
            "dictionary(int32, struct<px: int64, sym: utf8>)"
                .parse()
                .unwrap(),
            true,
        )]);
        let row = Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from("AAPL"),
        ])]);
        let (by_row, by_batch) = both_tiers("cast(q as utf8) as t", &encoded, &row);
        assert_eq!(
            by_row,
            Scalar::from_sequence([Scalar::from(r#"{"px":1,"sym":"AAPL"}"#)])
        );
        assert_eq!(by_batch, by_row);
    }

    /// A union's value is a positional pair with no text form, so neither
    /// tier spells it as JSON: the row refuses it, and both null it where the
    /// cast may.
    #[test]
    fn a_union_cast_into_text_spells_no_json_in_either_tier() {
        let schema = root([Field::new(
            "u",
            "variant(number: int64, text: utf8)".parse().unwrap(),
            true,
        )]);
        let row = schema
            .scalar(Scalar::from_sequence([Scalar::from(5_i64)]))
            .unwrap();
        let selector = "cast(u as utf8) as t".parse::<Selector>().unwrap();
        assert!(selector.apply_scalar(&schema, &row).is_err());
        let (by_row, by_batch) = both_tiers("try_cast(u as utf8) as t", &schema, &row);
        assert_eq!(by_row, Scalar::from_sequence([Scalar::Null]));
        assert_eq!(by_batch, by_row);
    }
}

/// A scalar cast meets a nested value and text through JSON, as a cast of
/// their columns does; the value contract, which a document reader runs, is
/// not a cast and keeps them apart.
mod json_casts {
    use yggdryl::{DataType, Field, Scalar};

    fn dtype(expression: &str) -> DataType {
        expression.parse().unwrap()
    }

    #[test]
    fn a_nested_value_cast_into_text_or_bytes_spells_its_json() {
        let record =
            Scalar::from_struct([("b", Scalar::from(1_i64)), ("a", Scalar::from("x"))]).unwrap();
        // A value carries no field: a record keys its object in name order,
        // a run is an array.
        assert_eq!(
            DataType::utf8().cast_scalar(&record).unwrap(),
            Scalar::from(r#"{"a":"x","b":1}"#)
        );
        assert_eq!(
            DataType::large_utf8()
                .cast_scalar(&Scalar::from_sequence([Scalar::from(1_i64), Scalar::Null]))
                .unwrap()
                .as_str(),
            Some("[1,null]")
        );
        let entries = Scalar::from_mapping([(Scalar::from(2_i64), Scalar::from("y"))]).unwrap();
        assert_eq!(
            DataType::utf8().cast_scalar(&entries).unwrap(),
            Scalar::from(r#"{"2":"y"}"#)
        );
        assert_eq!(
            DataType::binary().cast_scalar(&record).unwrap().as_bytes(),
            Some(br#"{"a":"x","b":1}"#.as_slice())
        );
        // The target's own rule runs over the JSON.
        let long = Scalar::from_sequence([Scalar::from("long")]);
        assert!(dtype("sized_utf8(4)").cast_scalar(&long).is_err());
        assert_eq!(dtype("sized_utf8(4)").try_cast_scalar(&long), Scalar::Null);
    }

    #[test]
    fn text_or_bytes_cast_into_a_nested_datatype_reads_the_document_it_holds() {
        let quote = dtype("struct<px: decimal128(10, 2), sym: utf8>");
        let read = Scalar::from_sequence([Scalar::decimal128(150, 2), Scalar::from("AAPL")]);
        assert_eq!(
            quote
                .cast_scalar(&Scalar::from(r#"{"sym":"AAPL","px":"1.50"}"#))
                .unwrap(),
            read
        );
        assert_eq!(
            quote
                .cast_scalar(&Scalar::from(br#"{"sym":"AAPL","px":"1.5"}"#.to_vec()))
                .unwrap(),
            read
        );
        assert_eq!(
            dtype("map<int64, serie<utf8>>")
                .cast_scalar(&Scalar::from(r#"{"1":["a"]}"#))
                .unwrap(),
            Scalar::from_mapping([(
                Scalar::from(1_i64),
                Scalar::from_sequence([Scalar::from("a")])
            )])
            .unwrap()
        );
        // A fixed slot's padding is the slot's.
        let slot = DataType::fixed_binary(5)
            .unwrap()
            .scalar(Scalar::from(b"[1]\0\0".to_vec()))
            .unwrap();
        assert_eq!(
            dtype("serie<int64>").cast_scalar(&slot).unwrap(),
            Scalar::from_sequence([Scalar::from(1_i64)])
        );
        // Absence: the empty cell and the document `null`.
        for absent in ["", " null ", "null"] {
            assert_eq!(
                quote.cast_scalar(&Scalar::from(absent)).unwrap(),
                Scalar::Null
            );
        }
        // A cell that is no document of the target is refused, and null
        // where the cast may null it.
        for refused in ["{", "abc", r#""[1]""#, r#"{"px":"x"}"#] {
            assert!(
                quote.cast_scalar(&Scalar::from(refused)).is_err(),
                "{refused}"
            );
            assert_eq!(quote.try_cast_scalar(&Scalar::from(refused)), Scalar::Null);
        }
    }

    /// The value contract a document reader runs is not a cast: a nested
    /// value has no text spelling there, and text is no nested value, so a
    /// repeated tag never lands in a text column as an array's JSON.
    #[test]
    fn the_value_contract_keeps_a_nested_value_and_text_apart() {
        let list = Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from("MSFT")]);
        assert!(DataType::utf8().scalar(list.clone()).is_err());
        assert!(
            Field::new("symbol", DataType::utf8(), true)
                .scalar(list)
                .is_err()
        );
        assert!(dtype("struct<a: int64>").scalar(r#"{"a":1}"#).is_err());
    }
}

/// The seven epoch functions floor a date and an instant to the period it
/// falls in, before the epoch included, and both tiers agree about it.
mod epoch_functions {
    use yggdryl::{
        ArrowCastOptions, DataType, Field, Scalar, Selector, Serie, StructType, TimeUnit, Timezone,
    };

    fn schema() -> Field {
        StructType::from_fields([
            DataType::date32().nullable_field("d"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::UTC,
            }
            .nullable_field("t"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    /// What one projection answers for one row, by row and by batch, which
    /// must be one value.
    fn answer(text: &str, days: Option<i32>, micros: Option<i64>) -> Scalar {
        let schema = schema();
        let row = Scalar::from_sequence([
            days.map_or(Scalar::Null, Scalar::date32),
            micros.map_or(Scalar::Null, |count| {
                Scalar::datetime64(count, TimeUnit::Microsecond, Timezone::UTC).unwrap()
            }),
        ]);
        let selector: Selector = text.parse().unwrap();
        let by_row = selector.apply_scalar(&schema, &row).unwrap();
        let batch = Serie::from_scalars(schema.clone(), [row])
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        let by_batch = Serie::from_arrow_batch(
            None,
            &selector.apply_arrow_batch(&batch).unwrap(),
            ArrowCastOptions::new(),
        )
        .unwrap()
        .scalar(0)
        .unwrap();
        assert_eq!(by_row, by_batch, "{text} over {days:?} / {micros:?}");
        by_row.as_sequence().unwrap()[0].clone()
    }

    #[test]
    fn a_date_floors_to_its_week_quarter_month_and_year() {
        // Weeks start on a Monday and count from Monday 1969-12-29.
        for (days, week) in [
            (-4, -1),
            (-3, 0),
            (-1, 0),
            (0, 0),
            (3, 0),
            (4, 1),
            (19_782, 2826),
        ] {
            assert_eq!(
                answer("weeks(d)", Some(days), None),
                Scalar::from(week),
                "{days}"
            );
        }
        // Quarters count from 1970-Q1: 1970-03-31 is 0, 1970-04-01 is 1.
        for (days, quarter) in [(-1, -1), (0, 0), (89, 0), (90, 1), (19_782, 216)] {
            assert_eq!(
                answer("quarters(d)", Some(days), None),
                Scalar::from(quarter),
                "{days}"
            );
        }
        for (days, month) in [(-1, -1), (0, 0), (30, 0), (31, 1), (90, 3), (19_782, 649)] {
            assert_eq!(
                answer("months(d)", Some(days), None),
                Scalar::from(month),
                "{days}"
            );
        }
        for (days, year) in [(-1, -1), (0, 0), (364, 0), (365, 1), (19_782, 54)] {
            assert_eq!(
                answer("years(d)", Some(days), None),
                Scalar::from(year),
                "{days}"
            );
        }
        assert_eq!(answer("days(d)", Some(-1), None), Scalar::date32(-1));
        assert_eq!(
            answer("days(d)", Some(19_782), None),
            Scalar::date32(19_782)
        );
        assert_eq!(answer("weeks(d)", None, None), Scalar::Null);
    }

    #[test]
    fn an_instant_floors_to_every_period_before_and_after_the_epoch() {
        // 2017-11-16T22:31:08, the instant Apache Iceberg's own transform
        // fixtures use.
        let at = 1_510_871_468_000_000_i64;
        for (text, expected) in [
            ("years(t)", Scalar::from(47)),
            ("quarters(t)", Scalar::from(191)),
            ("months(t)", Scalar::from(574)),
            ("weeks(t)", Scalar::from(2498)),
            ("days(t)", Scalar::date32(17_486)),
            ("hours(t)", Scalar::from(419_686)),
            ("minutes(t, 30)", Scalar::from(839_373)),
            ("minutes(t, 15)", Scalar::from(1_678_746)),
            ("minutes(t, 1)", Scalar::from(25_181_191)),
            ("minutes(t, 60)", Scalar::from(419_686)),
            ("minutes(t, 1440)", Scalar::from(17_486)),
        ] {
            assert_eq!(answer(text, None, Some(at)), expected, "{text}");
        }
        // One microsecond before the epoch is in the period before it, in
        // every period; the last instant of a period stays in it.
        for (text, expected) in [
            ("years(t)", Scalar::from(-1)),
            ("quarters(t)", Scalar::from(-1)),
            ("months(t)", Scalar::from(-1)),
            ("weeks(t)", Scalar::from(0)),
            ("days(t)", Scalar::date32(-1)),
            ("hours(t)", Scalar::from(-1)),
            ("minutes(t, 30)", Scalar::from(-1)),
            ("minutes(t, 15)", Scalar::from(-1)),
            ("minutes(t, 1)", Scalar::from(-1)),
        ] {
            assert_eq!(answer(text, None, Some(-1)), expected, "{text}");
        }
        for (text, micros, expected) in [
            ("minutes(t, 15)", 899_999_999, 0),
            ("minutes(t, 15)", 900_000_000, 1),
            ("minutes(t, 30)", 1_799_999_999, 0),
            ("minutes(t, 30)", 1_800_000_000, 1),
            ("minutes(t, 1)", 59_999_999, 0),
            ("minutes(t, 1)", 60_000_000, 1),
            ("hours(t)", 3_600_000_000, 1),
            ("minutes(t, 15)", -900_000_000, -1),
            ("minutes(t, 15)", -900_000_001, -2),
        ] {
            assert_eq!(
                answer(text, None, Some(micros)),
                Scalar::from(expected),
                "{text} of {micros}"
            );
        }
        assert_eq!(answer("minutes(t, 15)", None, None), Scalar::Null);
    }

    #[test]
    fn sixty_minutes_are_the_hour_and_the_widest_step_floors_exactly() {
        for micros in [
            1_510_871_468_000_000_i64,
            0,
            -1,
            3_599_999_999,
            3_600_000_000,
            -3_600_000_001,
        ] {
            assert_eq!(
                answer("minutes(t, 60)", None, Some(micros)),
                answer("hours(t)", None, Some(micros)),
                "{micros}"
            );
        }
        // The widest step there is - 257_698_037_700 seconds - still floors
        // exactly: its first period starts on its own length.
        let widest = format!("minutes(t, {})", u32::MAX);
        for (micros, expected) in [
            (257_698_037_700_000_000_i64, 1),
            (257_698_037_699_999_999, 0),
            (-1, -1),
            (-257_698_037_700_000_001, -2),
        ] {
            assert_eq!(
                answer(&widest, None, Some(micros)),
                Scalar::from(expected),
                "{micros}"
            );
        }
    }

    /// A count of seconds names years past `i32`, so a calendar period of
    /// one is read from its exact year: a number that fits `int32` is that
    /// number, and one past it answers null - never a year wrapped into one
    /// that fits, which would also stop the function being monotone, and a
    /// statistic mapped through it pruning by it.
    #[test]
    fn a_second_count_past_int32_answers_null_and_never_a_wrapped_year() {
        let schema = StructType::from_fields([DataType::DateTime64 {
            unit: TimeUnit::Second,
            timezone: Timezone::UTC,
        }
        .nullable_field("t")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let answer = |text: &str, seconds: i64| {
            let row = Scalar::from_sequence([Scalar::datetime64(
                seconds,
                TimeUnit::Second,
                Timezone::UTC,
            )
            .unwrap()]);
            let selector: Selector = text.parse().unwrap();
            let by_row = selector.apply_scalar(&schema, &row).unwrap();
            let batch = Serie::from_scalars(schema.clone(), [row])
                .unwrap()
                .into_arrow_batch()
                .unwrap();
            let by_batch = Serie::from_arrow_batch(
                None,
                &selector.apply_arrow_batch(&batch).unwrap(),
                ArrowCastOptions::new(),
            )
            .unwrap()
            .scalar(0)
            .unwrap();
            assert_eq!(by_row, by_batch, "{text} of {seconds}");
            by_row.as_sequence().unwrap()[0].clone()
        };
        for seconds in [
            i64::MAX,
            i64::MIN,
            68_000_000_000_000_000,
            -68_000_000_000_000_000,
        ] {
            for text in [
                "years(t)",
                "quarters(t)",
                "months(t)",
                "weeks(t)",
                "days(t)",
                "hours(t)",
                "minutes(t, 15)",
            ] {
                assert_eq!(answer(text, seconds), Scalar::Null, "{text} of {seconds}");
            }
        }
        // 6.7e16 seconds is in year 2123147449, whose number of years since
        // 1970 still fits `int32`; its quarters and months do not.
        for (seconds, years) in [
            (67_000_000_000_000_000_i64, 2_123_145_479),
            (-67_000_000_000_000_000, -2_123_145_480),
        ] {
            assert_eq!(
                answer("years(t)", seconds),
                Scalar::from(years),
                "{seconds}"
            );
            assert_eq!(answer("quarters(t)", seconds), Scalar::Null, "{seconds}");
            assert_eq!(answer("months(t)", seconds), Scalar::Null, "{seconds}");
        }
    }
}
