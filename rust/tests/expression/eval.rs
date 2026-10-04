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
    fn calendar_functions_keep_local_time_and_null_semantics() {
        let east: Timezone = "+02:00".parse().unwrap();
        let schema = DataType::from(
            StructType::from_fields([
                DataType::date32().required_field("date"),
                DataType::datetime64(TimeUnit::Second, Timezone::NAIVE)
                    .unwrap()
                    .required_field("before_epoch"),
                DataType::datetime64(TimeUnit::Second, east)
                    .unwrap()
                    .required_field("local"),
            ])
            .unwrap(),
        )
        .required_field("row");
        let row = Scalar::from_sequence([
            Scalar::date32(0),
            Scalar::datetime64(-1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::datetime64(0, TimeUnit::Second, east).unwrap(),
        ]);
        for (text, expected) in [
            ("year(date)", 1970),
            ("month(before_epoch)", 12),
            ("day(before_epoch)", 31),
            ("hour(date)", 0),
            ("hour(before_epoch)", 23),
            ("hour(local)", 2),
        ] {
            let bound = text.parse::<Term>().unwrap().bind(&schema).unwrap();
            assert_eq!(bound.eval(&row).unwrap(), Scalar::from(expected), "{text}");
            assert_eq!(
                bound
                    .eval(&Scalar::from_sequence([
                        Scalar::Null,
                        Scalar::Null,
                        Scalar::Null,
                    ]))
                    .unwrap(),
                Scalar::Null,
                "null {text}"
            );
        }
    }

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

    #[test]
    fn pure_math_bound_float64_answers_match_row_and_arrow() {
        let schema = root([DataType::Float32.nullable_field("x")]);
        for (name, expected) in [
            ("exp", 1.0_f64.exp()),
            ("ln", 1.0_f64.ln()),
            ("log10", 1.0_f64.log10()),
            ("degrees", 1.0_f64.to_degrees()),
            ("radians", 1.0_f64.to_radians()),
        ] {
            let selector = format!("{name}(x) as answer");
            let row = Scalar::from_sequence([Scalar::from(1.0_f32)]);
            let (by_row, by_batch) = both_tiers(&selector, &schema, &row);
            let result = &by_row.as_sequence().unwrap()[0];
            assert_eq!(
                result.as_f64().unwrap().to_bits(),
                expected.to_bits(),
                "{name}"
            );
            assert_eq!(by_batch, by_row, "{name}");
            let null = Scalar::from_sequence([Scalar::Null]);
            let (by_row, by_batch) = both_tiers(&selector, &schema, &null);
            assert_eq!(by_row, Scalar::from_sequence([Scalar::Null]), "{name}");
            assert_eq!(by_batch, by_row, "{name}");
        }
    }

    #[test]
    fn trigonometry_bound_float64_answers_match_row_and_arrow() {
        let schema = root([DataType::Float32.nullable_field("x")]);
        for (expression, expected) in [
            ("cos(x)", 0.5_f64.cos()),
            ("asin(x)", 0.5_f64.asin()),
            ("sin(x)", 0.5_f64.sin()),
            ("tan(x)", 0.5_f64.tan()),
            ("acos(x)", 0.5_f64.acos()),
            ("atan(x)", 0.5_f64.atan2(1.0)),
            ("atan2(x,1)", 0.5_f64.atan2(1.0)),
        ] {
            let selector = format!("{expression} as answer");
            let (by_row, by_batch) = both_tiers(
                &selector,
                &schema,
                &Scalar::from_sequence([Scalar::from(0.5_f32)]),
            );
            assert_eq!(
                by_row.as_sequence().unwrap()[0].as_f64().unwrap().to_bits(),
                expected.to_bits()
            );
            assert_eq!(by_batch, by_row);
            let (by_row, by_batch) =
                both_tiers(&selector, &schema, &Scalar::from_sequence([Scalar::Null]));
            assert_eq!(by_row, Scalar::from_sequence([Scalar::Null]));
            assert_eq!(by_batch, by_row);
        }
    }

    #[test]
    fn bound_power_fallback_matches_scalar_and_arrow_rows() {
        let schema = root([
            DataType::Float32.nullable_field("base"),
            DataType::Int64.required_field("exponent"),
        ]);
        for (base, expected) in [
            (Scalar::from(2.0_f32), Scalar::from(8.0_f64)),
            (Scalar::Null, Scalar::Null),
        ] {
            let row = Scalar::from_sequence([base, Scalar::from(3_i64)]);
            let (by_row, by_batch) = both_tiers("pow(base, exponent) as answer", &schema, &row);
            assert_eq!(by_row, Scalar::from_sequence([expected]));
            assert_eq!(by_batch, by_row);
        }
    }

    #[test]
    fn integer_math_bound_row_and_arrow_agree() {
        let schema = root([
            DataType::Int64.nullable_field("left"),
            DataType::UInt64.nullable_field("right"),
        ]);
        for (source, left, right, expected) in [
            (
                "gcd(left,right) as answer",
                Scalar::from(-12_i64),
                Scalar::from(18_u64),
                Scalar::from(6_u64),
            ),
            (
                "lcm(left,right) as answer",
                Scalar::from(12_i64),
                Scalar::from(18_u64),
                Scalar::from(36_u64),
            ),
            (
                "gcd(left,right) as answer",
                Scalar::Null,
                Scalar::from(18_u64),
                Scalar::Null,
            ),
        ] {
            let row = Scalar::from_sequence([left, right]);
            let (by_row, by_batch) = both_tiers(source, &schema, &row);
            assert_eq!(by_row, Scalar::from_sequence([expected]), "{source}");
            assert_eq!(by_batch, by_row, "{source}");
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

mod absolute_function_values {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Error, Scalar, StructType};

    fn absolute(dtype: DataType, value: Scalar) -> yggdryl::Result<Scalar> {
        let schema = StructType::from_fields([dtype.nullable_field("x")])
            .map(DataType::from)?
            .required_field("row");
        "abs(x)"
            .parse::<Term>()?
            .bind(&schema)?
            .eval(&Scalar::from_sequence([value]))
    }

    #[test]
    fn abs_uses_the_existing_checked_scalar_primitive() {
        for (dtype, value) in [
            (DataType::Int64, Scalar::from(-17_i64)),
            (DataType::Int64, Scalar::Null),
            (DataType::Float64, Scalar::from(-1.25_f64)),
            (DataType::Float64, Scalar::from(-0.0_f64)),
            (
                DataType::decimal128(9, 2).unwrap(),
                Scalar::decimal128(-125, 2),
            ),
        ] {
            assert_eq!(
                absolute(dtype, value.clone()).unwrap(),
                value.checked_abs().unwrap()
            );
        }
        assert!(matches!(
            absolute(DataType::Int64, Scalar::from(i64::MIN)),
            Err(Error::ArithmeticOverflow {
                operation: "absolute value",
                kind: "i64"
            }),
        ));
    }
}

#[test]
fn sqrt_uses_bound_float64_conversion_and_ieee_values() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};

    let schema = StructType::from_fields([DataType::Float32.nullable_field("x")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let root = Scalar::from_sequence([Scalar::from(9.0_f32)]);
    let bound = "sqrt(x)".parse::<Term>().unwrap().bind(&schema).unwrap();
    assert_eq!(bound.field().dtype(), &DataType::Float64);
    assert_eq!(bound.eval(&root).unwrap(), Scalar::from(3.0_f64));
    let null = Scalar::from_sequence([Scalar::Null]);
    assert_eq!(bound.eval(&null).unwrap(), Scalar::Null);
    let negative = Scalar::from_sequence([Scalar::from(-1.0_f32)]);
    assert!(bound.eval(&negative).unwrap().as_f64().unwrap().is_nan());
    let zero = Scalar::from_sequence([Scalar::from(0.0_f32)]);
    assert_eq!(bound.eval(&zero).unwrap(), Scalar::from(0.0_f64));

    let empty = StructType::from_fields([])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let literal = "sqrt(4)".parse::<Term>().unwrap().bind(&empty).unwrap();
    assert_eq!(
        literal.eval(&Scalar::from_sequence([])).unwrap(),
        Scalar::from(2.0_f64)
    );
}

#[test]
fn sqrt_uses_the_existing_float64_cast_for_large_integers() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};
    let schema = StructType::from_fields([DataType::Int64.required_field("x")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let bound = "sqrt(x)".parse::<Term>().unwrap().bind(&schema).unwrap();
    let original = Scalar::from(i64::MAX);
    let cast = DataType::Float64.cast_scalar(&original).unwrap();
    let expected = cast.as_f64().unwrap().sqrt();
    assert_eq!(
        bound.eval(&Scalar::from_sequence([original])).unwrap(),
        Scalar::from(expected)
    );
}

#[test]
fn decimal_scalar_float64_cast_matches_arrow_for_exact_value() {
    use yggdryl::{ArrowCastOptions, DataType, Scalar, Serie};

    let source = DataType::decimal128(9, 2).unwrap().required_field("amount");
    let target = DataType::Float64.required_field("amount");
    let value = Scalar::decimal128(225, 2);
    let column = Serie::from_scalars(source, [value.clone()]).unwrap();
    let arrow = column
        .cast(&target, ArrowCastOptions::new().with_safe(false))
        .unwrap()
        .scalar(0)
        .unwrap();
    assert_eq!(arrow, Scalar::from(2.25_f64), "Arrow's cast baseline");
    assert_eq!(
        target.dtype().cast_scalar(&value).unwrap(),
        arrow,
        "scalar cast disagrees with the already supported column cast"
    );
}

#[test]
fn all_decimal_scalar_layouts_cast_225_to_float64() {
    use yggdryl::{BigDecimal, DataType, Decimal, Decimal32, Decimal64, Scalar};
    for (dtype, value) in [
        (
            DataType::decimal32(9, 2).unwrap(),
            Scalar::Decimal32(Decimal32::new(225, 2)),
        ),
        (
            DataType::decimal64(18, 2).unwrap(),
            Scalar::Decimal64(Decimal64::new(225, 2)),
        ),
        (
            DataType::decimal128(38, 2).unwrap(),
            Scalar::decimal128(225, 2),
        ),
        (
            DataType::decimal256(76, 2).unwrap(),
            Scalar::decimal256(225_i128.into(), 2),
        ),
        (
            DataType::Decimal,
            Scalar::Decimal("2.25".parse::<Decimal>().unwrap()),
        ),
        (
            DataType::BigDecimal,
            Scalar::BigDecimal("2.25".parse::<BigDecimal>().unwrap()),
        ),
    ] {
        assert_eq!(value.id(), dtype.id(), "{dtype}");
        assert_eq!(
            DataType::Float64.cast_scalar(&value).unwrap(),
            Scalar::from(2.25_f64),
            "{dtype}"
        );
    }
}

#[test]
fn decimal256_wide_scalar_float_cast_matches_arrow() {
    use yggdryl::{ArrowCastOptions, DataType, Scalar, Serie, i256};
    let source = DataType::decimal256(76, 2)
        .unwrap()
        .required_field("amount");
    let target = DataType::Float64.required_field("amount");
    let value = Scalar::decimal256(
        "1234567890123456789012345678901234567890"
            .parse::<i256>()
            .unwrap(),
        2,
    );
    let column = Serie::from_scalars(source, [value.clone()]).unwrap();
    let arrow = column
        .cast(&target, ArrowCastOptions::new().with_safe(false))
        .unwrap()
        .scalar(0)
        .unwrap();
    assert_eq!(target.dtype().cast_scalar(&value).unwrap(), arrow);
}

#[test]
fn parameterized_decimal_float64_scalar_cast_matches_arrow_across_scales() {
    use yggdryl::{ArrowCastOptions, DataType, Decimal32, Decimal64, Scalar, Serie, i256};
    for scale in [-2, 0, 2, 4] {
        let rows = [225_i128, -225, 0];
        let layouts = [
            (
                DataType::decimal32(9, scale).unwrap(),
                rows.map(|n| Scalar::Decimal32(Decimal32::new(n as i32, scale))),
            ),
            (
                DataType::decimal64(18, scale).unwrap(),
                rows.map(|n| Scalar::Decimal64(Decimal64::new(n as i64, scale))),
            ),
            (
                DataType::decimal128(38, scale).unwrap(),
                rows.map(|n| Scalar::decimal128(n, scale)),
            ),
            (
                DataType::decimal256(76, scale).unwrap(),
                rows.map(|n| Scalar::decimal256(i256::from_i128(n), scale)),
            ),
        ];
        for (dtype, values) in layouts {
            let source = dtype.clone().required_field("amount");
            let target = DataType::Float64.required_field("amount");
            let column = Serie::from_scalars(source, values.clone()).unwrap();
            let arrow = column
                .cast(&target, ArrowCastOptions::new().with_safe(false))
                .unwrap();
            for (index, value) in values.into_iter().enumerate() {
                let expected = arrow.scalar(index).unwrap();
                let actual = target.dtype().cast_scalar(&value).unwrap();
                assert_eq!(actual, expected, "scale {scale}, index {index}, {dtype}");
            }
        }
    }
}

#[test]
fn decimal256_wide_signed_scalar_float_cast_matches_arrow() {
    use yggdryl::{ArrowCastOptions, DataType, Scalar, Serie, i256};
    let source = DataType::decimal256(76, 2)
        .unwrap()
        .required_field("amount");
    let target = DataType::Float64.required_field("amount");
    for text in [
        "1234567890123456789012345678901234567890",
        "-1234567890123456789012345678901234567890",
    ] {
        let value = Scalar::decimal256(text.parse::<i256>().unwrap(), 2);
        let column = Serie::from_scalars(source.clone(), [value.clone()]).unwrap();
        let arrow = column
            .cast(&target, ArrowCastOptions::new().with_safe(false))
            .unwrap()
            .scalar(0)
            .unwrap();
        assert_eq!(target.dtype().cast_scalar(&value).unwrap(), arrow, "{text}");
    }
}

#[test]
fn decimal_float_completion_does_not_parse_untyped_text_or_boolean() {
    use yggdryl::{DataType, Scalar};
    for value in [
        Scalar::from("2.25"),
        Scalar::from("not a number"),
        Scalar::from(true),
    ] {
        assert!(DataType::Float64.cast_scalar(&value).is_err(), "{value:?}");
        assert_eq!(DataType::Float64.try_cast_scalar(&value), Scalar::Null);
    }
    assert_eq!(
        DataType::Float64.cast_scalar(&Scalar::Null).unwrap(),
        Scalar::Null
    );
}

#[test]
fn trim_keeps_unicode_whitespace_semantics_and_nulls() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};

    let schema = StructType::from_fields([DataType::utf8().nullable_field("s")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let bound = "trim(s)".parse::<Term>().unwrap().bind(&schema).unwrap();
    for (input, expected) in [
        (Scalar::from("  a\u{2003}b \t"), Scalar::from("a\u{2003}b")),
        (Scalar::from("\u{2003}\t"), Scalar::from("")),
        (Scalar::Null, Scalar::Null),
    ] {
        let row = Scalar::from_sequence([input]);
        assert_eq!(bound.eval(&row).unwrap(), expected);
    }
}

#[test]
fn power_uses_the_bound_float64_kernel_and_preserves_ieee() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};

    let empty = StructType::from_fields([])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let row = Scalar::from_sequence([]);
    for text in ["pow(2,3)", "power(2,3)"] {
        let bound = text.parse::<Term>().unwrap().bind(&empty).unwrap();
        assert_eq!(bound.field().dtype(), &DataType::Float64);
        assert_eq!(bound.eval(&row).unwrap(), Scalar::from(8.0_f64));
    }
    let zero = "pow(0,0)".parse::<Term>().unwrap().bind(&empty).unwrap();
    assert_eq!(zero.eval(&row).unwrap(), Scalar::from(1.0_f64));
    let negative = "pow(-8,0.5)".parse::<Term>().unwrap().bind(&empty).unwrap();
    assert!(negative.eval(&row).unwrap().as_f64().unwrap().is_nan());
    assert!("pow(2)".parse::<Term>().is_err());

    let schema = StructType::from_fields([
        DataType::Float32.nullable_field("base"),
        DataType::Int64.required_field("exponent"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let bound = "pow(base, exponent)"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(
        bound
            .eval(&Scalar::from_sequence([
                Scalar::from(2.0_f32),
                Scalar::from(3_i64),
            ]))
            .unwrap(),
        Scalar::from(8.0_f64)
    );
    assert_eq!(
        bound
            .eval(&Scalar::from_sequence([Scalar::Null, Scalar::from(3_i64),]))
            .unwrap(),
        Scalar::Null
    );
}

#[test]
fn shared_pure_math_float64_functions_bind_once_and_evaluate() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};
    let schema = StructType::from_fields([DataType::Float32.nullable_field("x")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let row = Scalar::from_sequence([Scalar::from(1.0_f32)]);
    for (name, expected) in [
        ("exp", 1.0_f64.exp()),
        ("ln", 1.0_f64.ln()),
        ("log10", 1.0_f64.log10()),
        ("degrees", 1.0_f64.to_degrees()),
        ("radians", 1.0_f64.to_radians()),
    ] {
        let expression = format!("{name}(x)");
        let bound = expression.parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(bound.field().dtype(), &DataType::Float64, "{name}");
        assert_eq!(
            bound.eval(&row).unwrap().as_f64().unwrap().to_bits(),
            expected.to_bits(),
            "{name}"
        );
        assert_eq!(
            bound.eval(&Scalar::from_sequence([Scalar::Null])).unwrap(),
            Scalar::Null,
            "{name}"
        );
    }
}

#[test]
fn shared_trigonometry_binds_numeric_inputs_once() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};
    let schema = StructType::from_fields([DataType::Float32.nullable_field("x")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    for (name, expected) in [("cos", 0.5_f64.cos()), ("asin", 0.5_f64.asin())] {
        let bound = format!("{name}(x)")
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(bound.field().dtype(), &DataType::Float64);
        assert_eq!(
            bound
                .eval(&Scalar::from_sequence([Scalar::from(0.5_f32)]))
                .unwrap()
                .as_f64()
                .unwrap()
                .to_bits(),
            expected.to_bits()
        );
        assert_eq!(
            bound.eval(&Scalar::from_sequence([Scalar::Null])).unwrap(),
            Scalar::Null
        );
    }
}

#[test]
fn shared_remaining_trigonometry_binds_and_evaluates() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};

    let schema = StructType::from_fields([
        DataType::Float32.nullable_field("y"),
        DataType::Float32.nullable_field("x"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let row = Scalar::from_sequence([Scalar::from(0.5_f32), Scalar::from(1.0_f32)]);
    for (expression, expected) in [
        ("sin(y)", 0.5_f64.sin()),
        ("tan(y)", 0.5_f64.tan()),
        ("acos(y)", 0.5_f64.acos()),
        ("atan(y)", 0.5_f64.atan2(1.0)),
        ("atan2(y,x)", 0.5_f64.atan2(1.0)),
    ] {
        let bound = expression.parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(bound.field().dtype(), &DataType::Float64, "{expression}");
        let actual = bound.eval(&row).unwrap().as_f64().unwrap();
        assert_eq!(actual.to_bits(), expected.to_bits(), "{expression}");
        let null_row = Scalar::from_sequence([Scalar::Null, Scalar::from(1.0_f32)]);
        assert_eq!(bound.eval(&null_row).unwrap(), Scalar::Null, "{expression}");
    }
    assert!("atan2(y)".parse::<Term>().is_err());
    assert!("sin(y,x)".parse::<Term>().is_err());
}

#[test]
fn casing_keeps_rendered_version_and_null_expression_contracts() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType, Version};
    let schema = StructType::from_fields([yggdryl::Field::new("s", DataType::Version, true)])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    for expression in ["lower(s)", "upper(s)"] {
        let bound = expression.parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(
            bound
                .eval(&Scalar::from_sequence([Scalar::from(
                    "5.0.2".parse::<Version>().unwrap()
                )]))
                .unwrap(),
            Scalar::from("5.0.2")
        );
        assert_eq!(
            bound.eval(&Scalar::from_sequence([Scalar::Null])).unwrap(),
            Scalar::Null
        );
    }
}

#[test]
fn generic_integer_gcd_lcm_are_exact() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};

    let schema = StructType::from_fields([
        DataType::Int64.nullable_field("left"),
        DataType::UInt64.nullable_field("right"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    for (source, left, right, expected) in [
        (
            "gcd(left,right)",
            Scalar::from(-12_i64),
            Scalar::from(18_u64),
            Scalar::from(6_u64),
        ),
        (
            "lcm(left,right)",
            Scalar::from(12_i64),
            Scalar::from(18_u64),
            Scalar::from(36_u64),
        ),
        (
            "gcd(left,right)",
            Scalar::from(0_i64),
            Scalar::from(0_u64),
            Scalar::from(0_u64),
        ),
        (
            "lcm(left,right)",
            Scalar::from(0_i64),
            Scalar::from(18_u64),
            Scalar::from(0_u64),
        ),
        (
            "gcd(left,right)",
            Scalar::Null,
            Scalar::from(18_u64),
            Scalar::Null,
        ),
    ] {
        let bound = source.parse::<Term>().unwrap().bind(&schema).unwrap();
        assert_eq!(bound.field().dtype(), &DataType::UInt64, "{source}");
        let row = Scalar::from_sequence([left, right]);
        assert_eq!(bound.eval(&row).unwrap(), expected, "{source}");
    }
}

#[test]
fn factorial_binds_exact_integer_inputs_and_returns_float64() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};
    let schema = StructType::from_fields([DataType::Int64.nullable_field("n")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let bound = "factorial(n)"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.field().dtype(), &DataType::Float64);
    for (source, expected) in [
        (0, 1.0_f64),
        (5, 120.0),
        (20, 2_432_902_008_176_640_000.0),
        (170, 7.257415615308004e306),
    ] {
        let value = bound
            .eval(&Scalar::from_sequence([Scalar::from(source as i64)]))
            .unwrap();
        assert_eq!(value.as_f64().unwrap().to_bits(), expected.to_bits());
    }
    assert_eq!(
        bound.eval(&Scalar::from_sequence([Scalar::Null])).unwrap(),
        Scalar::Null
    );
    assert!(
        bound
            .eval(&Scalar::from_sequence([Scalar::from(-1_i64)]))
            .is_err()
    );
    assert!(
        bound
            .eval(&Scalar::from_sequence([Scalar::from(171_i64)]))
            .unwrap()
            .as_f64()
            .unwrap()
            .is_infinite()
    );
}

#[test]
fn exact_integer_math_required_null_is_nullable() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, StructType};
    let schema = StructType::from_fields([
        DataType::Null.required_field("n"),
        DataType::UInt64.required_field("k"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    for text in ["gcd(n,k)", "lcm(n,k)"] {
        assert!(
            text.parse::<Term>()
                .unwrap()
                .bind(&schema)
                .unwrap()
                .field()
                .is_nullable(),
            "{text}"
        );
    }
}

#[test]
fn factorial_required_null_field_stays_nullable() {
    use yggdryl::expression::Term;
    use yggdryl::{DataType, Scalar, StructType};
    let schema = StructType::from_fields([DataType::Null.required_field("n")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let bound = "factorial(n)"
        .parse::<Term>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.field().dtype(), &DataType::Float64);
    assert!(bound.field().is_nullable());
    assert_eq!(
        bound.eval(&Scalar::from_sequence([Scalar::Null])).unwrap(),
        Scalar::Null
    );
    for invalid in [DataType::Float64, DataType::utf8(), DataType::Boolean] {
        let schema = StructType::from_fields([invalid.required_field("n")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        assert!(
            "factorial(n)"
                .parse::<Term>()
                .unwrap()
                .bind(&schema)
                .is_err()
        );
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

/// `time_bucket(width, x)`: a date or a timestamp floored to a multiple of a
/// constant width from DuckDB's origin, in `x`'s own datatype, the same by
/// row and by batch.
mod time_bucket {
    use yggdryl::{
        ArrowCastOptions, DataType, Field, Scalar, Selector, Serie, StructType, Term, TimeUnit,
        Timezone,
    };

    const MINUTE_NS: i64 = 60_000_000_000;

    fn instant(unit: TimeUnit) -> DataType {
        DataType::DateTime64 {
            unit,
            timezone: Timezone::UTC,
        }
    }

    /// One column per unit, zoned and naive, two dates and a whole number.
    fn schema() -> Field {
        StructType::from_fields([
            instant(TimeUnit::Second).nullable_field("s"),
            DataType::DateTime64 {
                unit: TimeUnit::Millisecond,
                timezone: Timezone::NAIVE,
            }
            .nullable_field("ms"),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: Timezone::from_str("Europe/Paris").unwrap(),
            }
            .nullable_field("us"),
            instant(TimeUnit::Nanosecond).nullable_field("ns"),
            DataType::date32().nullable_field("d"),
            DataType::date64().nullable_field("e"),
            DataType::Int64.nullable_field("w"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    /// A row holding the one value `column` names, every other cell null.
    fn row(schema: &Field, column: &str, value: Scalar) -> Scalar {
        Scalar::from_sequence(schema.fields().iter().map(|field| {
            if field.name() == column {
                value.clone()
            } else {
                Scalar::Null
            }
        }))
    }

    /// What one projection answers over rows, by row and by batch, which
    /// must be the same values.
    fn answers(text: &str, rows: &[Scalar]) -> Vec<Scalar> {
        let schema = schema();
        let selector: Selector = text.parse().unwrap();
        let by_row: Vec<Scalar> = rows
            .iter()
            .map(|held| {
                selector
                    .apply_scalar(&schema, held)
                    .unwrap()
                    .as_sequence()
                    .unwrap()[0]
                    .clone()
            })
            .collect();
        let batch = Serie::from_scalars(schema.clone(), rows.to_vec())
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        let answered = Serie::from_arrow_batch(
            None,
            &selector.apply_arrow_batch(&batch).unwrap(),
            ArrowCastOptions::new(),
        )
        .unwrap();
        let by_batch: Vec<Scalar> = (0..rows.len())
            .map(|position| answered.scalar(position).unwrap().as_sequence().unwrap()[0].clone())
            .collect();
        assert_eq!(by_row, by_batch, "{text}");
        by_row
    }

    fn nanos(count: i64) -> Scalar {
        Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::UTC).unwrap()
    }

    #[test]
    fn an_instant_floors_on_the_bucket_one_nanosecond_before_and_before_the_epoch() {
        let schema = schema();
        let quarter = 15 * MINUTE_NS;
        let cases = [
            (0, 0),
            (quarter, quarter),
            (quarter - 1, 0),
            (quarter + 1, quarter),
            (-1, -quarter),
            (-quarter, -quarter),
            (-quarter - 1, -2 * quarter),
            // 2024-01-01T00:00:00.000000001.
            (1_704_067_200_000_000_001, 1_704_067_200_000_000_000),
        ];
        let rows: Vec<Scalar> = cases
            .iter()
            .map(|(count, _)| row(&schema, "ns", nanos(*count)))
            .chain([row(&schema, "ns", Scalar::Null)])
            .collect();
        let expected: Vec<Scalar> = cases
            .iter()
            .map(|(_, floored)| nanos(*floored))
            .chain([Scalar::Null])
            .collect();
        assert_eq!(answers("time_bucket('15 minutes', ns)", &rows), expected);
    }

    #[test]
    fn every_unit_and_zone_keeps_its_datatype_and_floors_the_same_instant() {
        let schema = schema();
        // 2017-11-16T22:31:08.123456789, which floors to 22:30:00.
        let (seconds, floored) = (1_510_871_468_i64, 1_510_871_400_i64);
        let paris = Timezone::from_str("Europe/Paris").unwrap();
        for (column, value, expected) in [
            (
                "s",
                Scalar::datetime64(seconds, TimeUnit::Second, Timezone::UTC).unwrap(),
                Scalar::datetime64(floored, TimeUnit::Second, Timezone::UTC).unwrap(),
            ),
            (
                "ms",
                Scalar::datetime64(
                    seconds * 1_000 + 123,
                    TimeUnit::Millisecond,
                    Timezone::NAIVE,
                )
                .unwrap(),
                Scalar::datetime64(floored * 1_000, TimeUnit::Millisecond, Timezone::NAIVE)
                    .unwrap(),
            ),
            (
                "us",
                Scalar::datetime64(seconds * 1_000_000 + 123_456, TimeUnit::Microsecond, paris)
                    .unwrap(),
                Scalar::datetime64(floored * 1_000_000, TimeUnit::Microsecond, paris).unwrap(),
            ),
            (
                "ns",
                nanos(seconds * 1_000_000_000 + 123_456_789),
                nanos(floored * 1_000_000_000),
            ),
        ] {
            let text = format!("time_bucket('15 minutes', {column})");
            assert_eq!(
                answers(&text, &[row(&schema, column, value)]),
                vec![expected],
                "{column}"
            );
            // The answer is the column's own datatype, unit and zone kept.
            let typed = text
                .parse::<Selector>()
                .unwrap()
                .apply_field(&schema)
                .unwrap();
            assert_eq!(
                typed.fields()[0].dtype(),
                schema.get_field_by_path(column).unwrap().dtype(),
                "{column}"
            );
        }
    }

    #[test]
    fn a_width_spells_one_length_many_ways() {
        let schema = schema();
        let at = 1_704_068_999_999_999_999_i64; // 2024-01-01T00:29:59.999999999
        let rows = [row(&schema, "ns", nanos(at))];
        let expected = vec![nanos(1_704_068_100_000_000_000)];
        for width in [
            "'15 minutes'",
            "'15 MINUTES'",
            "'15minutes'",
            "' 15 min '",
            "'15 mins'",
            "'900s'",
            "'900 seconds'",
            "'0.25h'",
            "'0.25 hours'",
            "'900000 ms'",
            "'PT15M'",
            "'00:15:00'",
            "duration64(s) 'PT900S'",
            "duration32(ms) 'PT15M'",
        ] {
            assert_eq!(
                answers(&format!("time_bucket({width}, ns)"), &rows),
                expected,
                "{width}"
            );
        }
        // The bucket keeps the spelling it was written in.
        for text in [
            "time_bucket('15 minutes', currunix)",
            "time_bucket('PT15M', currunix)",
        ] {
            assert_eq!(text.parse::<Term>().unwrap().to_string(), text);
        }
    }

    #[test]
    fn day_and_week_widths_start_at_duckdbs_monday_origin() {
        let schema = schema();
        // 1970-01-01 is a Thursday: its week starts Monday 1969-12-29.
        let day = 86_400_i64;
        let seconds =
            |count: i64| Scalar::datetime64(count, TimeUnit::Second, Timezone::UTC).unwrap();
        assert_eq!(
            answers(
                "time_bucket('1 week', s)",
                &[
                    row(&schema, "s", seconds(0)),
                    row(&schema, "s", seconds(4 * day)),
                    row(&schema, "s", seconds(4 * day - 1)),
                ]
            ),
            vec![seconds(-3 * day), seconds(4 * day), seconds(-3 * day)]
        );
        // A width that divides a day starts at the epoch whatever the origin.
        assert_eq!(
            answers(
                "time_bucket('1 day', s)",
                &[row(&schema, "s", seconds(day + 1))]
            ),
            vec![seconds(day)]
        );
        // A date floors in whole days: 2024-02-14 is a Wednesday, its week
        // starts on Monday 2024-02-12, day 19_765 - and 19_779 is Monday
        // 2024-02-26.
        assert_eq!(
            answers(
                "time_bucket('7 days', d)",
                &[
                    row(&schema, "d", Scalar::date32(19_767)),
                    row(&schema, "d", Scalar::date32(19_779)),
                    row(&schema, "d", Scalar::date32(-1)),
                ]
            ),
            vec![
                Scalar::date32(19_765),
                Scalar::date32(19_779),
                Scalar::date32(-3)
            ]
        );
        let millis = |days: i64| {
            Scalar::date64_in(days * 86_400_000, TimeUnit::Millisecond, Timezone::NAIVE).unwrap()
        };
        assert_eq!(
            answers(
                "time_bucket('1 week', e)",
                &[row(&schema, "e", millis(19_767))]
            ),
            vec![millis(19_765)]
        );
    }

    #[test]
    fn a_width_that_is_no_fixed_positive_constant_is_refused_naming_it() {
        let schema = schema();
        let refused = |text: &str| -> String {
            text.parse::<Selector>()
                .and_then(|selector| selector.apply_field(&schema))
                .expect_err(text)
                .to_string()
        };
        for width in [
            "'15m'",
            "'0 minutes'",
            "'-15 minutes'",
            "'1 month'",
            "'1 year'",
            "'quarter'",
            "'15'",
            "'1.5ns'",
            "15",
            "duration64(s) 'PT0S'",
        ] {
            let error = refused(&format!("time_bucket({width}, ns)"));
            assert!(
                error.contains("width of time_bucket(width, x)"),
                "{width}: {error}"
            );
        }
        // A width finer than the unit, a clock under a date, a computed
        // width, and an argument that is no date or timestamp.
        let error = refused("time_bucket('1 ms', s)");
        assert!(error.contains("whole number of s"), "{error}");
        let error = refused("time_bucket('1500 ms', s)");
        assert!(error.contains("whole number of s"), "{error}");
        let error = refused("time_bucket('1 hour', d)");
        assert!(error.contains("whole days"), "{error}");
        let error = refused("time_bucket('12 hours', e)");
        assert!(error.contains("whole days"), "{error}");
        let error = refused("time_bucket(w, ns)");
        assert!(error.contains("to be a constant"), "{error}");
        let error = refused("time_bucket(null, ns)");
        assert!(error.contains("to be a constant"), "{error}");
        let error = refused("time_bucket('15 minutes', w)");
        assert!(error.contains("a date or a timestamp"), "{error}");
        // A bucket starting before the first count a timestamp holds is
        // refused by both tiers, never wrapped.
        let selector: Selector = "time_bucket('15 minutes', ns)".parse().unwrap();
        let low = row(&schema, "ns", nanos(i64::MIN));
        assert!(selector.apply_scalar(&schema, &low).is_err());
        let batch = Serie::from_scalars(schema.clone(), [low])
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        assert!(selector.apply_arrow_batch(&batch).is_err());
    }

    #[test]
    fn a_batch_floors_every_present_row_and_keeps_every_null() {
        let schema = schema();
        let hour = 60 * MINUTE_NS;
        let rows: Vec<Scalar> = (0..1_000_i64)
            .map(|index| {
                let value = if index % 7 == 0 {
                    Scalar::Null
                } else {
                    nanos(index * 7 * MINUTE_NS - 500 * MINUTE_NS)
                };
                row(&schema, "ns", value)
            })
            .collect();
        let expected: Vec<Scalar> = (0..1_000_i64)
            .map(|index| {
                if index % 7 == 0 {
                    Scalar::Null
                } else {
                    nanos((index * 7 * MINUTE_NS - 500 * MINUTE_NS).div_euclid(hour) * hour)
                }
            })
            .collect();
        assert_eq!(answers("time_bucket('1 hour', ns)", &rows), expected);
    }
}
