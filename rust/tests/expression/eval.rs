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
        let row = Scalar::from_sequence([Scalar::d256(wide.units(), 18)]);
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
                one(Scalar::d128(115, 2)),
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
                one(Scalar::d128(13, 2)),
            ),
            (
                -0.125,
                "cast(f as decimal(10, 2)) as l",
                one(Scalar::d128(-13, 2)),
            ),
            (
                2.5,
                "cast(f as decimal(10, 0)) as l",
                one(Scalar::d128(3, 0)),
            ),
            (
                -2.5,
                "cast(f as decimal(10, 0)) as l",
                one(Scalar::d128(-3, 0)),
            ),
            (
                0.124,
                "cast(f as decimal(10, 2)) as l",
                one(Scalar::d128(12, 2)),
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
}
