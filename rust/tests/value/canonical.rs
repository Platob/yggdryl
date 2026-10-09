//! `rust/src/value/canonical.rs`: the value a datatype accepts:
//! canonicalization, readings, and absence.

mod value {
    use yggdryl::{DataType, Field, Map, Scalar, Serie, StructType, TimeUnit, Timezone, UnionMode};

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        DataType::from(StructType::from_fields(fields).unwrap()).required_field("row")
    }

    #[test]
    fn a_record_maps_names_to_schema_order_and_fills_field_defaults() {
        let schema = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("venue"),
        ]);
        let record = Scalar::from_struct([("id", Scalar::from(7))]).unwrap();

        schema.validate_value(&record).unwrap();
        assert_eq!(
            schema.canonicalize_value(record).unwrap(),
            Scalar::from_sequence([Scalar::from(7), Scalar::Null])
        );
    }

    #[test]
    fn a_record_refuses_unknown_names() {
        let schema = root([DataType::Int64.required_field("id")]);
        let record =
            Scalar::from_struct([("id", Scalar::from(7)), ("unknown", Scalar::from(1))]).unwrap();

        let validation = schema.validate_value(&record).unwrap_err().to_string();
        let canonical = schema.canonicalize_value(record).unwrap_err().to_string();
        assert!(validation.contains("unknown field"), "{validation}");
        assert!(canonical.contains("unknown field"), "{canonical}");
    }

    #[test]
    fn canonicalization_diagnostics_keep_field_entry_and_union_locations() {
        let map = DataType::map_of(DataType::Int32, DataType::Int32, false).unwrap();
        let map_root = root([map.required_field("lookup")]);
        for (value, path) in [
            (
                Scalar::Map(Map::new(vec![(
                    Scalar::from("not an integer"),
                    Scalar::from(1_i32),
                )])),
                "$.row.lookup[0].key",
            ),
            (
                Scalar::Map(Map::new(vec![(
                    Scalar::from(1_i32),
                    Scalar::from("not an integer"),
                )])),
                "$.row.lookup[0].value",
            ),
        ] {
            let refused = map_root
                .canonicalize_value(Scalar::from_sequence([value]))
                .unwrap_err()
                .to_string();
            assert!(refused.contains(path), "{refused}");
        }

        let union = DataType::union(
            [(1, DataType::Int32.required_field("integer"))],
            UnionMode::Dense,
        )
        .unwrap();
        let union_root = root([union.required_field("choice")]);
        let refused = union_root
            .canonicalize_value(Scalar::from_sequence([Scalar::from_sequence([
                Scalar::from(1_i64),
                Scalar::from("not an integer"),
            ])]))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("$.row.choice.union[1]"), "{refused}");
    }

    #[test]
    fn integer_canonicalization_preserves_every_declared_width() {
        let schema = root([
            DataType::Int8.required_field("i8"),
            DataType::Int16.required_field("i16"),
            DataType::Int32.required_field("i32"),
            DataType::Int64.required_field("i64"),
            DataType::UInt8.required_field("u8"),
            DataType::UInt16.required_field("u16"),
            DataType::UInt32.required_field("u32"),
            DataType::UInt64.required_field("u64"),
        ]);
        let natural = Scalar::from_sequence([
            Scalar::from(-1),
            Scalar::from(2),
            Scalar::from(-3),
            Scalar::from(-4),
            Scalar::from(1),
            Scalar::from(2),
            Scalar::from(3),
            Scalar::from(4),
        ]);
        let canonical = schema.canonicalize_value(natural).unwrap();
        let values = canonical.as_sequence().unwrap();
        assert!(matches!(values[0], Scalar::Int8(_)));
        assert!(matches!(values[1], Scalar::Int16(_)));
        assert!(matches!(values[2], Scalar::Int32(_)));
        assert!(matches!(values[3], Scalar::Int64(_)));
        assert!(matches!(values[4], Scalar::UInt8(_)));
        assert!(matches!(values[5], Scalar::UInt16(_)));
        assert!(matches!(values[6], Scalar::UInt32(_)));
        assert!(matches!(values[7], Scalar::UInt64(_)));
    }

    #[test]
    fn year_month_interval_canonicalizes_to_the_exact_interval_leaf() {
        let schema = root([DataType::interval(TimeUnit::YearMonth)
            .unwrap()
            .required_field("months")]);

        let canonical = schema
            .canonicalize_value(Scalar::from_sequence([Scalar::from(18)]))
            .unwrap();
        let value = &canonical.as_sequence().unwrap()[0];
        assert!(matches!(
            value,
            Scalar::Interval(interval)
                if interval.months() == 18 && interval.unit() == TimeUnit::YearMonth
        ));
    }

    #[test]
    fn temporal_casts_preserve_family_and_timezone() {
        let schema = root([
            DataType::DateTime64 {
                unit: TimeUnit::Millisecond,
                timezone: Timezone::UTC,
            }
            .required_field("at"),
            DataType::Time32(TimeUnit::Second).required_field("clock"),
            DataType::Duration32(TimeUnit::Millisecond).required_field("elapsed"),
        ]);
        let valid = Scalar::from_sequence([
            Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
            Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration64(3, TimeUnit::Second).unwrap(),
        ]);
        assert_eq!(
            schema.canonicalize_value(valid).unwrap(),
            Scalar::from_sequence([
                Scalar::datetime64(1_000, TimeUnit::Millisecond, Timezone::UTC).unwrap(),
                Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::duration32(3_000, TimeUnit::Millisecond).unwrap(),
            ])
        );

        for invalid in [
            Scalar::from_sequence([
                Scalar::datetime64(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
            ]),
            Scalar::from_sequence([
                Scalar::duration64(1, TimeUnit::Second).unwrap(),
                Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
                Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
            ]),
            Scalar::from_sequence([
                Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
                Scalar::from("not a time"),
                Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
            ]),
        ] {
            assert!(schema.validate_value(&invalid).is_err());
            assert!(schema.canonicalize_value(invalid).is_err());
        }
    }

    #[test]
    fn a_temporal_of_the_column_s_family_that_does_not_fit_says_why() {
        let refusal = |dtype: DataType, value: Scalar| dtype.scalar(value).unwrap_err().to_string();
        let utc = DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap();

        // A naive value into a zoned column: the zone is what is missing.
        let message = refusal(
            utc.clone(),
            Scalar::datetime64(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        );
        assert!(
            message.contains("expected datetime64(ns,\"UTC\")"),
            "{message}"
        );
        assert!(
            message.contains("a naive value states no zone"),
            "{message}"
        );

        // A zoned value into a naive column, and one zone into another.
        let naive = DataType::datetime64(TimeUnit::Second, Timezone::NAIVE).unwrap();
        let message = refusal(
            naive,
            Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
        );
        assert!(message.contains("naive wall-clock times"), "{message}");

        // A count the column's unit cannot state exactly names the count.
        let seconds = DataType::duration64(TimeUnit::Second).unwrap();
        let message = refusal(
            seconds,
            Scalar::duration64(1_500, TimeUnit::Millisecond).unwrap(),
        );
        assert!(
            message.contains("1500 ms is not a whole count of s"),
            "{message}"
        );
    }

    /// The spellings a value takes on the way into a datatype, and the ones it
    /// prints on the way out - the same readings a column takes and prints.
    mod readings {
        use yggdryl::Map;
        use yggdryl::{DataType, Scalar};

        fn dtype(expression: &str) -> DataType {
            expression.parse().unwrap()
        }

        #[test]
        fn text_reads_into_every_family_a_text_column_reads_into() {
            assert_eq!(DataType::Int64.scalar("42").unwrap(), Scalar::from(42_i64));
            assert_eq!(DataType::UInt8.scalar(" 7 ").unwrap(), Scalar::from(7_u8));
            assert_eq!(
                DataType::Float64.scalar("4.5").unwrap(),
                Scalar::from(4.5_f64)
            );
            assert_eq!(
                DataType::Boolean.scalar("TRUE").unwrap(),
                Scalar::from(true)
            );
            assert_eq!(
                dtype("decimal128(10, 2)").scalar("10.50").unwrap(),
                Scalar::decimal128(1_050, 2)
            );
            assert_eq!(
                DataType::date32().scalar("1970-01-02").unwrap(),
                Scalar::date32(1)
            );

            // A magnitude the declared width cannot hold is refused by the width,
            // not wrapped by the reading.
            assert!(DataType::Int8.scalar("200").is_err());
            // A digit the declared scale cannot hold is refused rather than rounded.
            let refused = dtype("decimal128(10, 2)")
                .scalar("1.005")
                .unwrap_err()
                .to_string();
            assert!(refused.contains("fractional digits"), "{refused}");
        }

        #[test]
        fn every_spelling_of_one_number_reaches_a_decimal_column_at_its_scale() {
            let column = dtype("decimal128(12, 2)");
            let hundred = Scalar::decimal128(10_000, 2);

            // A whole number is a decimal of scale zero, so writing it into a
            // column of scale two is one hundred, not one: the coefficient is
            // restated, never taken as though it were already unscaled.
            assert_eq!(column.scalar(100_i64).unwrap(), hundred);
            assert_eq!(column.scalar(Scalar::decimal128(100, 0)).unwrap(), hundred);
            assert_eq!(column.scalar("100.00").unwrap(), hundred);
            assert_eq!(column.scalar(Scalar::from(100_u8)).unwrap(), hundred);

            // The same holds at every declared width, and through a Field.
            for spelling in ["decimal32(9, 2)", "decimal64(12, 2)", "decimal256(40, 2)"] {
                let column = dtype(spelling);
                assert_eq!(
                    column
                        .clone()
                        .required_field("size")
                        .scalar(100_i64)
                        .unwrap(),
                    column.scalar(Scalar::decimal128(100, 0)).unwrap(),
                    "{spelling}"
                );
            }

            // A negative scale removes digits, and only exactly.
            let tens = dtype("decimal128(12, -1)");
            assert_eq!(tens.scalar(100_i64).unwrap(), Scalar::decimal128(10, -1));
            assert!(tens.scalar(105_i64).is_err());
        }

        #[test]
        fn every_value_with_a_spelling_prints_it_into_a_text_column() {
            assert_eq!(DataType::utf8().scalar(7_i64).unwrap(), Scalar::from("7"));
            assert_eq!(
                DataType::utf8().scalar(Scalar::date32(0)).unwrap(),
                Scalar::from("1970-01-01")
            );
            assert_eq!(
                DataType::utf8()
                    .scalar(DataType::Ccy.scalar("USD").unwrap())
                    .unwrap(),
                Scalar::from("USD")
            );
            assert_eq!(
                DataType::utf8()
                    .scalar(Scalar::from(b"AAPL".to_vec()))
                    .unwrap(),
                Scalar::from("AAPL")
            );

            // A payload that was read and refused names the charset and the byte
            // it refused, rather than which kind arrived.
            let refused = DataType::utf8()
                .scalar(Scalar::from(vec![0xFF_u8]))
                .unwrap_err()
                .to_string();
            assert!(refused.contains("utf-8"), "{refused}");
            assert!(refused.contains("0xff"), "{refused}");
        }

        #[test]
        fn a_byte_column_stores_the_payload_a_value_spells() {
            assert_eq!(
                DataType::binary().scalar("hi").unwrap(),
                Scalar::from(b"hi".to_vec())
            );
            // A code spells its characters' bytes, which is the payload its text
            // column stores; so does any other text, and an enum member the
            // bytes of its stored name, and nothing more.
            let code = DataType::Ccy.scalar("USD").unwrap();
            assert_eq!(
                DataType::binary().scalar(code).unwrap().as_bytes(),
                Some(b"USD".as_slice())
            );
            let member = DataType::State.scalar("NEW").unwrap();
            assert_eq!(
                DataType::binary().scalar(member).unwrap().as_bytes(),
                Some(b"NEW".as_slice())
            );
            let text = dtype("fixed_ascii(4)").scalar("US").unwrap();
            assert_eq!(
                DataType::binary().scalar(text).unwrap().as_bytes(),
                Some(b"US".as_slice())
            );
            let uuid = DataType::uuid()
                .scalar("00000000-0000-0000-0000-000000000001")
                .unwrap();
            assert_eq!(
                DataType::fixed_binary(16)
                    .unwrap()
                    .scalar(uuid)
                    .unwrap()
                    .as_bytes(),
                Some(
                    [0_u8; 15]
                        .iter()
                        .copied()
                        .chain([1])
                        .collect::<Vec<_>>()
                        .as_slice()
                )
            );

            // The declared width is part of the layout on every path.
            let refused = DataType::fixed_binary(4)
                .unwrap()
                .scalar(Scalar::from(vec![1_u8, 2]))
                .unwrap_err()
                .to_string();
            assert!(refused.contains("exactly 4 bytes"), "{refused}");
        }

        #[test]
        fn a_record_is_the_entries_a_map_column_holds() {
            let map = dtype("map<utf8, int32>");
            let record = Scalar::from_struct([("a", Scalar::from(1_i32))]).unwrap();

            assert_eq!(
                map.scalar(record).unwrap(),
                Scalar::from_mapping([(Scalar::from("a"), Scalar::from(1_i32))]).unwrap()
            );
        }

        #[test]
        fn a_map_carries_its_invariants_however_its_entries_were_built() {
            // `Map::new` takes already-unique entries on trust, so the value
            // contract is what refuses a map that is not a function.
            let duplicates = Scalar::Map(Map::new(vec![
                (Scalar::from("a"), Scalar::from(1_i32)),
                (Scalar::from("a"), Scalar::from(2_i32)),
            ]));
            let refused = dtype("map<utf8, int32>")
                .scalar(duplicates)
                .unwrap_err()
                .to_string();
            assert!(refused.contains("collide"), "{refused}");

            // A declared ordering is checked whether or not anything was restated.
            let unsorted = Scalar::Map(Map::new(vec![
                (Scalar::from("b"), Scalar::from(1_i32)),
                (Scalar::from("a"), Scalar::from(2_i32)),
            ]));
            let sorted = DataType::map_of(DataType::utf8(), DataType::Int32, true).unwrap();
            let refused = sorted.scalar(unsorted.clone()).unwrap_err().to_string();
            assert!(refused.contains("not sorted"), "{refused}");
            DataType::map_of(DataType::utf8(), DataType::Int32, false)
                .unwrap()
                .scalar(unsorted)
                .unwrap();

            // Keys in strictly ascending order pass both checks in one pass,
            // short or past the sixteen a scan covers; a repeat at the end of
            // that order is still the collision it is, named where it stands.
            for len in [3_usize, 40] {
                let mut entries: Vec<(Scalar, Scalar)> = (0..len)
                    .map(|index| (Scalar::from(format!("k{index:03}")), Scalar::from(1_i32)))
                    .collect();
                let held = Scalar::Map(Map::new(entries.clone()));
                assert_eq!(sorted.scalar(held.clone()).unwrap(), held, "{len}");
                entries.push(entries[len - 1].clone());
                let refused = sorted
                    .scalar(Scalar::Map(Map::new(entries)))
                    .unwrap_err()
                    .to_string();
                assert!(refused.contains("collide"), "{len}: {refused}");
                assert!(refused.contains(&format!("[{len}]")), "{len}: {refused}");
            }
        }

        #[test]
        fn a_union_type_id_has_one_canonical_representation() {
            let union = dtype("union<0: int32>");
            let canonical = Scalar::from_sequence([Scalar::from(0_i64), Scalar::from(1_i32)]);

            // The id does not depend on whether the payload also needed restating.
            assert_eq!(
                union
                    .scalar(Scalar::from_sequence([
                        Scalar::from(0_i8),
                        Scalar::from(1_i32)
                    ]))
                    .unwrap(),
                canonical
            );
            assert_eq!(union.scalar(canonical.clone()).unwrap(), canonical);
        }

        fn pair(type_id: i64, payload: Scalar) -> Scalar {
            Scalar::from_sequence([Scalar::from(type_id), payload])
        }

        #[test]
        fn a_bare_value_two_members_fit_is_refused_naming_both() {
            // Two members of the value's own datatype are two readings of it.
            let union = dtype("union<0: int64, 1: int64>");
            let refused = union.scalar(7_i64).unwrap_err().to_string();
            assert!(refused.contains("more than one union member"), "{refused}");
            assert!(refused.contains("[type_id, payload]"), "{refused}");
            // So are two members of its family, when neither is its own.
            let refused = dtype("union<0: int16, 1: int32>")
                .scalar(Scalar::from(7_i64))
                .unwrap_err()
                .to_string();
            assert!(refused.contains("more than one union member"), "{refused}");
            // A value no member accepts names the members.
            let refused = dtype("union<0: int64, 1: date32>")
                .scalar(Scalar::from(true))
                .unwrap_err()
                .to_string();
            assert!(refused.contains("one union member accepts"), "{refused}");
        }

        #[test]
        fn a_bare_value_enters_the_member_its_datatype_names() {
            let union = dtype("union<0: int64, 1: utf8>");

            assert_eq!(union.scalar(7_i64).unwrap(), pair(0, Scalar::from(7_i64)));
            // Text is the text member's, even when it spells a number.
            assert_eq!(union.scalar("42").unwrap(), pair(1, Scalar::from("42")));
            // The value's family answers when no member is its own datatype,
            // and the member's contract then restates it.
            assert_eq!(
                dtype("union<0: int32, 1: utf8>").scalar(7_i64).unwrap(),
                pair(0, Scalar::from(7_i32))
            );
            // The one member that accepts it answers when neither does.
            assert_eq!(
                dtype("union<0: date32, 1: int64>")
                    .scalar("2024-01-02")
                    .unwrap(),
                pair(0, DataType::Date32.scalar("2024-01-02").unwrap())
            );
            // The pair is still the pair, and a bare value inside a row is read
            // the same way as a bare value on its own.
            assert_eq!(
                union.scalar(pair(1, Scalar::from("hi"))).unwrap(),
                pair(1, Scalar::from("hi"))
            );
            let row = dtype("struct<choice: union<0: int64, 1: utf8>>");
            assert_eq!(
                row.scalar(Scalar::from_sequence([Scalar::from("x")]))
                    .unwrap(),
                Scalar::from_sequence([pair(1, Scalar::from("x"))])
            );
        }

        #[test]
        fn a_tie_keeps_the_members_that_accept_the_value() {
            // Every record member is a struct, so the record's own datatype
            // finds them all; the one whose names it spells is its member.
            let union = dtype(
                "union<0: struct<a: int64 not null> not null, \
                 1: struct<b: utf8 not null> not null>",
            );
            let leg = Scalar::from_struct([("a", Scalar::from(1_i64))]).unwrap();
            let quote = Scalar::from_struct([("b", Scalar::from("q"))]).unwrap();
            assert_eq!(
                union.scalar(leg).unwrap(),
                pair(0, Scalar::from_sequence([Scalar::from(1_i64)]))
            );
            assert_eq!(
                union.scalar(quote).unwrap(),
                pair(1, Scalar::from_sequence([Scalar::from("q")]))
            );
            // Two members the record fits are still two readings, and one it
            // fits neither of names the members its datatype found.
            let twins = dtype(
                "union<0: struct<a: int64 not null> not null, \
                 1: struct<a: int64 not null> not null>",
            );
            let refused = twins
                .scalar(Scalar::from_struct([("a", Scalar::from(1_i64))]).unwrap())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("more than one union member"), "{refused}");
            let refused = union
                .scalar(Scalar::from_struct([("c", Scalar::from(1_i64))]).unwrap())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("one union member accepts"), "{refused}");
        }

        #[test]
        fn branch_of_reads_any_value_bare_a_sequence_included() {
            let union = dtype("union<0: serie<int64>, 1: int64>");
            let DataType::Union(members, _) = &union else {
                panic!("a union")
            };
            let list = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(5_i64)]);

            assert_eq!(members.branch_of(&list).unwrap().0, 0);
            assert_eq!(members.branch_of(&Scalar::from(5_i64)).unwrap().0, 1);
            // Under `scalar` the same sequence spells the pair, as it always has.
            assert_eq!(
                union.scalar(list.clone()).unwrap(),
                pair(1, Scalar::from(5_i64))
            );
        }
    }

    /// Absence is a value only where the layout stores it beside the values.
    mod absence {
        use yggdryl::{DataType, Field, Scalar, UnionMode};

        #[test]
        fn a_union_and_a_run_end_spell_absence_through_a_child() {
            // Both layouts carry absence inside a child rather than beside the
            // values. A union routes a bare null like any bare value: to the
            // `null` member, else the one member that takes a null.
            let members = |fields: Vec<(i8, Field)>| {
                DataType::union(fields, UnionMode::Dense).expect("a union")
            };
            let pair = |type_id: i64| Scalar::from_sequence([Scalar::from(type_id), Scalar::Null]);
            let with_null = members(vec![
                (0, Field::new("int", DataType::Int64, false)),
                (1, Field::new("str", DataType::utf8(), false)),
                (2, Field::new("NoneType", DataType::Null, true)),
            ]);
            assert_eq!(with_null.scalar(Scalar::Null).unwrap(), pair(2));
            assert_eq!(
                Field::new("u", with_null.clone(), true)
                    .scalar(Scalar::Null)
                    .unwrap(),
                pair(2)
            );
            // A required union field refuses absence however it is spelled.
            let refused = Field::new("u", with_null, false)
                .scalar(Scalar::Null)
                .unwrap_err()
                .to_string();
            assert!(refused.contains("non-nullable"), "{refused}");
            // One nullable member holds it where no member is `null`.
            let union: DataType = "union<0: int32>".parse().unwrap();
            assert_eq!(union.scalar(Scalar::Null).unwrap(), pair(0));
            // Two are two readings, and required members hold none.
            let refused = members(vec![
                (0, Field::new("int", DataType::Int64, true)),
                (1, Field::new("str", DataType::utf8(), true)),
            ])
            .scalar(Scalar::Null)
            .unwrap_err()
            .to_string();
            assert!(refused.contains("more than one union member"), "{refused}");
            let refused = members(vec![
                (0, Field::new("int", DataType::Int64, false)),
                (1, Field::new("str", DataType::utf8(), false)),
            ])
            .scalar(Scalar::Null)
            .unwrap_err()
            .to_string();
            assert!(refused.contains("one union member accepts"), "{refused}");
            // The pair that spells it is still accepted.
            union.scalar(pair(0)).unwrap();

            let required = DataType::run_end_encoded(
                DataType::Int32.required_field("run_ends"),
                DataType::Int32.required_field("values"),
            )
            .unwrap();
            let refused = required.scalar(Scalar::Null).unwrap_err().to_string();
            assert!(refused.contains("non-nullable"), "{refused}");
            // A run-end layout whose values child holds a null does hold one.
            DataType::run_end_encoded(
                DataType::Int32.required_field("run_ends"),
                DataType::Int32.nullable_field("values"),
            )
            .unwrap()
            .scalar(Scalar::Null)
            .unwrap();

            // Every other datatype still takes absence as a value of its own.
            assert_eq!(DataType::Int32.scalar(Scalar::Null).unwrap(), Scalar::Null);
        }
    }

    /// The column of `values` under a required `int64` item.
    fn int64_column(values: &[i64]) -> Scalar {
        let item = DataType::Int64.required_field("item");
        Scalar::from(Serie::from_scalars(item, values.iter().copied().map(Scalar::from)).unwrap())
    }

    /// The run of `values`.
    fn int64_run(values: &[i64]) -> Scalar {
        Scalar::from_sequence(values.iter().copied().map(Scalar::from))
    }

    #[test]
    fn a_column_fails_where_the_run_of_its_rows_fails() {
        let refusal =
            |schema: &Field, value: &Scalar| schema.validate_value(value).unwrap_err().to_string();

        // A row given as a column: the arity, then the second value's shape.
        let row = root([
            DataType::Int64.required_field("id"),
            DataType::from_str("struct<a: int64>")
                .unwrap()
                .required_field("pair"),
        ]);
        for values in [&[1_i64][..], &[1, 2][..]] {
            assert_eq!(
                refusal(&row, &int64_column(values)),
                refusal(&row, &int64_run(values))
            );
        }

        // A serie column whose item field is not the one the serie declares.
        let list =
            root([DataType::serie(DataType::Int8.required_field("item")).required_field("xs")]);
        assert_eq!(
            refusal(&list, &Scalar::from_sequence([int64_column(&[1, 300])])),
            refusal(&list, &Scalar::from_sequence([int64_run(&[1, 300])]))
        );
    }
}

/// A record - what every JSON or YAML object reads as - holds no order, so a
/// sorted map takes its entries in the order of the keys they name, not of
/// the names' text: `10` after `2`.
#[test]
fn a_record_read_into_a_sorted_map_is_ordered_by_its_keys() {
    use yggdryl::{DataType, Scalar};

    let sorted = DataType::map_of(DataType::Int64, DataType::utf8(), true).unwrap();
    let record =
        Scalar::from_struct([("10", Scalar::from("b")), ("2", Scalar::from("a"))]).unwrap();
    assert_eq!(
        sorted.scalar(record).unwrap(),
        Scalar::from_mapping([
            (Scalar::from(2_i64), Scalar::from("a")),
            (Scalar::from(10_i64), Scalar::from("b")),
        ])
        .unwrap()
    );
}

#[test]
fn a_value_of_a_bare_contract_leaf_is_answered_untouched() {
    // A value of the very leaf the datatype is, where the leaf carries no
    // parameter and its layout is its whole contract, is already canonical:
    // the door hands it back unread, sharing its handle. Every other value
    // still walks the door.
    let text =
        yggdryl::Scalar::from("a symbol long enough to live off the stack, shared by handle");
    let held = text.as_str().unwrap().as_ptr();
    let answered = yggdryl::DataType::utf8().scalar(text).unwrap();
    assert_eq!(
        answered.as_str().unwrap().as_ptr(),
        held,
        "the text was copied"
    );
    assert!(
        yggdryl::DataType::Int8
            .scalar(yggdryl::Scalar::from(300_i64))
            .is_err()
    );
    assert_eq!(
        yggdryl::DataType::Int8
            .scalar(yggdryl::Scalar::from(3_i64))
            .unwrap(),
        yggdryl::Scalar::from(3_i8)
    );
    let bounded = yggdryl::DataType::sized_utf8(4).unwrap();
    assert!(bounded.scalar(yggdryl::Scalar::from("too long")).is_err());
    assert_eq!(
        bounded
            .scalar(yggdryl::Scalar::from("ok"))
            .unwrap()
            .as_str(),
        Some("ok")
    );
}

#[test]
fn an_integer_of_any_width_narrows_to_an_integer_leaf_exactly_at_its_range() {
    // Every width meets every leaf at both ends of the leaf's range: in it,
    // the value is the leaf's own at the leaf's width; one past it, refused
    // by the check that names the leaf.
    use yggdryl::{DataType, Scalar};
    let widths = |value: i128| {
        [
            i8::try_from(value).ok().map(Scalar::from),
            i16::try_from(value).ok().map(Scalar::from),
            i32::try_from(value).ok().map(Scalar::from),
            i64::try_from(value).ok().map(Scalar::from),
            Some(Scalar::from(value)),
            u8::try_from(value).ok().map(Scalar::from),
            u16::try_from(value).ok().map(Scalar::from),
            u32::try_from(value).ok().map(Scalar::from),
            u64::try_from(value).ok().map(Scalar::from),
            u128::try_from(value).ok().map(Scalar::from),
        ]
        .into_iter()
        .flatten()
    };
    for (dtype, minimum, maximum) in [
        (DataType::Int8, i128::from(i8::MIN), i128::from(i8::MAX)),
        (DataType::Int16, i128::from(i16::MIN), i128::from(i16::MAX)),
        (DataType::Int32, i128::from(i32::MIN), i128::from(i32::MAX)),
        (DataType::Int64, i128::from(i64::MIN), i128::from(i64::MAX)),
        (DataType::UInt8, 0, i128::from(u8::MAX)),
        (DataType::UInt16, 0, i128::from(u16::MAX)),
        (DataType::UInt32, 0, i128::from(u32::MAX)),
        (DataType::UInt64, 0, i128::from(u64::MAX)),
    ] {
        for value in [minimum, maximum] {
            for held in widths(value) {
                let read = dtype.scalar(held.clone()).unwrap();
                assert_eq!(read.dtype().unwrap(), dtype, "{held:?} into {dtype}");
                assert_eq!(read.as_i128(), Some(value), "{held:?} into {dtype}");
            }
        }
        for value in [minimum - 1, maximum + 1] {
            for held in widths(value) {
                let refused = dtype.scalar(held.clone()).unwrap_err().to_string();
                assert!(
                    refused.contains(dtype.name()),
                    "{held:?} into {dtype}: {refused}"
                );
            }
        }
    }
}

/// A column stating `FIELD:representation=bits` reads a same-width integer
/// of the other signedness as its bits; the datatype's own door, and a
/// column stating nothing, read every value by value.
#[test]
fn a_column_stating_bits_reads_the_other_sign_of_its_width_as_its_bits() {
    use yggdryl::{DataType, Field, Representation, Scalar};

    let stating = |dtype: DataType| -> Field {
        let mut field = dtype.required_field("digest");
        field
            .as_field_properties_mut()
            .set_representation(Representation::Bits)
            .unwrap();
        field
    };

    let digest = stating(DataType::UInt64);
    assert_eq!(digest.scalar(-1_i64).unwrap(), Scalar::from(u64::MAX));
    assert_eq!(digest.scalar(i64::MIN).unwrap(), Scalar::from(1_u64 << 63));
    // A value both readings agree on is read by value.
    assert_eq!(digest.scalar(5_i64).unwrap(), Scalar::from(5_u64));
    let signed = stating(DataType::Int64);
    assert_eq!(signed.scalar(u64::MAX).unwrap(), Scalar::from(-1_i64));
    assert_eq!(signed.scalar(7_u64).unwrap(), Scalar::from(7_i64));

    for (unsigned, signed, negative, top) in [
        (
            DataType::UInt32,
            DataType::Int32,
            Scalar::from(-2_i32),
            Scalar::from(u32::MAX - 1),
        ),
        (
            DataType::UInt16,
            DataType::Int16,
            Scalar::from(-2_i16),
            Scalar::from(u16::MAX - 1),
        ),
        (
            DataType::UInt8,
            DataType::Int8,
            Scalar::from(-2_i8),
            Scalar::from(u8::MAX - 1),
        ),
    ] {
        assert_eq!(
            stating(unsigned.clone()).scalar(negative.clone()).unwrap(),
            top,
            "{unsigned}"
        );
        assert_eq!(
            stating(signed.clone()).scalar(top.clone()).unwrap(),
            negative,
            "{signed}"
        );
    }

    // Bits are read only across one width: a narrower negative is refused.
    assert!(digest.scalar(-1_i32).is_err());
    // A column stating nothing, and the datatype, read by value.
    assert!(
        DataType::UInt64
            .required_field("digest")
            .scalar(-1_i64)
            .is_err()
    );
    assert!(DataType::UInt64.scalar(Scalar::from(-1_i64)).is_err());
    assert!(DataType::Int64.scalar(Scalar::from(u64::MAX)).is_err());
    // The nullability of the column still holds.
    assert!(digest.scalar(Scalar::Null).is_err());

    // A column laid out from such values holds their bits.
    let column =
        yggdryl::Serie::from_scalars(digest, [Scalar::from(-1_i64), Scalar::from(5_i64)]).unwrap();
    assert_eq!(column.scalar(0).unwrap(), Scalar::from(u64::MAX));
    assert_eq!(column.scalar(1).unwrap(), Scalar::from(5_u64));
}

/// A row under a root whose column states bits validates and canonicalizes
/// the bits, at the root and inside a serie's item.
#[test]
fn a_row_under_a_root_stating_bits_validates_and_canonicalizes_the_bits() {
    use yggdryl::{DataType, Representation, Scalar, StructType};

    let mut digest = DataType::UInt64.required_field("d");
    digest
        .as_field_properties_mut()
        .set_representation(Representation::Bits)
        .unwrap();
    let item =
        DataType::from(StructType::from_fields([digest.clone()]).unwrap()).required_field("item");
    let root = DataType::from(
        StructType::from_fields([digest, DataType::serie(item).nullable_field("items")]).unwrap(),
    )
    .required_field("row");

    let row = Scalar::from_sequence([
        Scalar::from(-1_i64),
        Scalar::from_sequence([Scalar::from_sequence([Scalar::from(-2_i64)])]),
    ]);
    root.validate_value(&row).unwrap();
    let canonical = root.canonicalize_value(row).unwrap();
    assert_eq!(
        canonical.get(0).map(|cell| cell.into_owned()),
        Some(Scalar::from(u64::MAX))
    );
    let items = canonical.get(1).unwrap().into_owned();
    let first = items.get(0).unwrap().into_owned();
    assert_eq!(
        first.get(0).map(|cell| cell.into_owned()),
        Some(Scalar::from(u64::MAX - 1))
    );

    // The same row under a root stating nothing is refused.
    let plain =
        DataType::from(StructType::from_fields([DataType::UInt64.required_field("d")]).unwrap())
            .required_field("row");
    let refused = Scalar::from_sequence([Scalar::from(-1_i64)]);
    assert!(plain.validate_value(&refused).is_err());
    assert!(plain.canonicalize_value(refused).is_err());
}
