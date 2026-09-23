//! `rust/src/parser.rs`.

mod nested {
    use yggdryl::DataType;

    #[test]
    fn deeply_nested_sql_hive_and_spark_types_round_trip_canonically() {
        let values = [
            "array<struct<id:bigint,name:string,tags:array<string>>>",
            "map<string,array<decimal(38,18)>>",
            "struct<`quoted,name`:string, nested:map<string,struct<x:int,y:boolean>>>",
            "row(id integer not null, payload varbinary)",
        ];
        for value in values {
            let parsed = DataType::from_str(value).unwrap();
            let canonical = parsed.to_string();
            assert_eq!(
                DataType::from_str(&canonical).unwrap(),
                parsed,
                "{canonical}"
            );
        }
    }
}

mod grammar {
    use yggdryl::{DataType, DataTypeId, Field, StructType, TimeUnit, Timezone};

    #[test]
    fn variant_parser_alias_canonicalizes_to_dense_union() {
        let expected = DataType::dense_union([
            Field::new("number", DataType::Int64, true),
            Field::new("text", DataType::utf8(), true),
        ])
        .unwrap();
        for source in [
            "variant(number:int64,text:string)",
            "variant(dense,number:int64,text:string)",
            "variant(0=number:int64,1=text:string)",
            "variant([number:int64,text:string],dense)",
        ] {
            assert_eq!(DataType::from_str(source).unwrap(), expected, "{source}");
        }

        let canonical = expected.to_string();
        assert!(canonical.starts_with("union(dense,"), "{canonical}");
        assert!(!canonical.contains("variant"), "{canonical}");
        assert_eq!(DataType::from_str(&canonical).unwrap(), expected);
    }

    #[test]
    fn union_layout_words_remain_available_as_unquoted_member_names() {
        let members = [
            Field::new("dense", DataType::Int64, true),
            Field::new("sparse", DataType::utf8(), true),
        ];
        let variant = DataType::dense_union(members.clone()).unwrap();
        assert_eq!(
            DataType::from_str("variant(dense:int64,sparse:utf8)").unwrap(),
            variant,
        );

        let dense = DataType::union(
            [(0, members[0].clone()), (1, members[1].clone())],
            yggdryl::UnionMode::Dense,
        )
        .unwrap();
        assert_eq!(
            DataType::from_str("union(dense,dense:int64,sparse:utf8)").unwrap(),
            dense,
        );
        assert_eq!(
            DataType::from_str("dense_union(sparse:int64)").unwrap(),
            DataType::union(
                [(0, Field::new("sparse", DataType::Int64, true))],
                yggdryl::UnionMode::Dense,
            )
            .unwrap(),
        );
        assert_eq!(
            DataType::from_str("sparse_union(dense:int64)").unwrap(),
            DataType::union(
                [(0, Field::new("dense", DataType::Int64, true))],
                yggdryl::UnionMode::Sparse,
            )
            .unwrap(),
        );
    }

    #[test]
    fn variant_parser_rejects_sparse_or_nonsequential_layouts() {
        for source in [
            "variant(sparse,number:int64)",
            "variant([number:int64],sparse)",
            "variant(1=number:int64)",
            "variant(0=number:int64,2=text:string)",
        ] {
            let error = DataType::from_str(source).unwrap_err();
            assert!(
                error.to_string().contains("variant"),
                "unexpected error for {source}: {error}"
            );
        }
    }

    #[test]
    fn variant_parser_enforces_member_and_nesting_limits() {
        let members = (0..128)
            .map(|index| format!("member_{index}:int64"))
            .collect::<Vec<_>>()
            .join(",");
        let accepted = DataType::from_str(&format!("variant({members})")).unwrap();
        assert_eq!(accepted.field_len(), 128);

        let rejected = format!("variant({members},overflow:int64)");
        assert!(DataType::from_str(&rejected).is_err());

        let mut nested = "int64".to_owned();
        for depth in 0..24 {
            nested = format!("variant(level_{depth}:{nested})");
        }
        let parsed = DataType::from_str(&nested).unwrap();
        assert_eq!(DataType::from_str(&parsed.to_string()).unwrap(), parsed);
    }

    #[test]
    fn datatype_parser_reuses_unified_temporal_and_interval_aliases() {
        for (source, expected) in [
            (
                "datetime64(Second)",
                DataType::DateTime64 {
                    unit: TimeUnit::Second,
                    timezone: Timezone::NAIVE,
                },
            ),
            (
                "datetime64(Nanoseconds,UTC)",
                DataType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::UTC,
                },
            ),
            (
                "timestamp(nano seconds,UTC)",
                DataType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::UTC,
                },
            ),
            ("time32(seconds)", DataType::Time32(TimeUnit::Second)),
            (
                "time32(milli seconds)",
                DataType::Time32(TimeUnit::Millisecond),
            ),
            (
                "time64(Microsecond)",
                DataType::Time64(TimeUnit::Microsecond),
            ),
            (
                "time64(micro seconds)",
                DataType::Time64(TimeUnit::Microsecond),
            ),
            (
                "duration32(MILLIS)",
                DataType::Duration32(TimeUnit::Millisecond),
            ),
            (
                "duration64(micro seconds)",
                DataType::Duration64(TimeUnit::Microsecond),
            ),
            (
                "interval(YearMonth)",
                DataType::Interval(TimeUnit::YearMonth),
            ),
            (
                "interval(DAY TO SECOND)",
                DataType::Interval(TimeUnit::DayTime),
            ),
            (
                "interval(MonthDayNano)",
                DataType::Interval(TimeUnit::MonthDayNano),
            ),
            ("interval", DataType::Interval(TimeUnit::MonthDayNano)),
            ("INTERVAL YEAR", DataType::Interval(TimeUnit::YearMonth)),
            ("INTERVAL DAY", DataType::Interval(TimeUnit::DayTime)),
        ] {
            assert_eq!(DataType::from_str(source).unwrap(), expected, "{source:?}");
        }

        assert_eq!(
            DataType::from_str("timestamp(us,UTC)").unwrap().to_string(),
            "datetime64(us,\"UTC\")"
        );
    }

    #[test]
    fn datatype_parser_rejects_time_unit_category_mismatches() {
        for source in [
            "datetime64(year_month)",
            "time32(day_time)",
            "time64(month_day_nano)",
            "duration32(year_month)",
            "duration64(year_month)",
            "duration(ns)",
            "interval(ns)",
            "interval(fortnight)",
            "interval fortnight",
        ] {
            assert!(DataType::from_str(source).is_err(), "{source:?}");
        }
    }

    #[test]
    fn datatype_unit_errors_point_at_the_original_unit_token() {
        for (source, expected_position) in [
            ("datetime64(fortnight)", 11),
            ("interval(fortnight)", 9),
            ("time(day_time)", 5),
        ] {
            let error = DataType::from_str(source).unwrap_err();
            assert!(
                matches!(
                    &error,
                    yggdryl::Error::Parse {
                        target: "datatype",
                        position,
                        ..
                    } if *position == expected_position
                ),
                "unexpected error for {source:?}: {error}"
            );
        }
    }

    #[test]
    fn bare_interval_defaults_before_postfix_serie_wrapping() {
        let interval = DataType::interval(TimeUnit::MonthDayNano).unwrap();
        let list = DataType::serie(Field::new("item", interval, true));
        let nested_list = DataType::serie(Field::new("item", list.clone(), true));

        assert_eq!(DataType::from_str("interval[]").unwrap(), list);
        assert_eq!(DataType::from_str("interval[][]").unwrap(), nested_list);
    }

    #[test]
    fn escaped_quoted_unit_errors_map_decoded_offsets_to_original_bytes() {
        for source in [
            r#"interval("fort\night!")"#,
            r#"interval("fort\u006eight!")"#,
        ] {
            let expected_position = source.find('!').unwrap();
            let error = DataType::from_str(source).unwrap_err();
            assert!(
                matches!(
                    &error,
                    yggdryl::Error::Parse {
                        target: "datatype",
                        position,
                        ..
                    } if *position == expected_position
                ),
                "unexpected error for {source:?}: {error}"
            );
        }
    }

    #[test]
    fn parser_rejects_unbalanced_trailing_and_duplicate_input() {
        for malformed in [
            "array<struct<id:bigint>",
            "decimal(18,4) trailing",
            "struct<a:int,a:string>",
            "union(dense,0=field(\"a\",int64,nullable=true,metadata={}),0=field(\"b\",utf8,nullable=true,metadata={}))",
            "map<string>",
        ] {
            assert!(DataType::from_str(malformed).is_err(), "{malformed}");
        }
    }

    #[test]
    fn parser_recursion_limits_have_exact_public_boundaries() {
        let mut accepted_type = "int64".to_owned();
        for _ in 0..DataType::PARSE_RECURSION_LIMIT - 1 {
            accepted_type = format!("array<{accepted_type}>");
        }
        assert!(DataType::from_str(&accepted_type).is_ok());

        let rejected_type = format!("array<{accepted_type}>");
        assert!(DataType::from_str(&rejected_type).is_err());
    }

    #[test]
    fn every_datatype_variant_prints_a_spelling_the_grammar_reads_back() {
        // The grammar and the display are one contract, so a variant that prints
        // something the grammar cannot read is a variant a caller cannot write.
        // This list names every `DataTypeId`, so a new variant fails here until
        // both halves know it.
        let values = [
            DataType::Null,
            DataType::Boolean,
            DataType::Int8,
            DataType::Int16,
            DataType::Int32,
            DataType::Int64,
            DataType::UInt8,
            DataType::UInt16,
            DataType::UInt32,
            DataType::UInt64,
            DataType::Float16,
            DataType::Float32,
            DataType::Float64,
            DataType::binary(),
            DataType::fixed_binary(16).unwrap(),
            DataType::large_binary(),
            DataType::binary_view(),
            DataType::utf8(),
            DataType::large_utf8(),
            DataType::utf8_view(),
            DataType::ascii(),
            DataType::from_str("ascii(4)").unwrap(),
            DataType::fixed_ascii(4).unwrap(),
            DataType::from_str("sized_binary(16)").unwrap(),
            DataType::from_str("string(windows-1252)").unwrap(),
            DataType::Country,
            DataType::Currency,
            DataType::MicCode,
            DataType::CfiCode,
            DataType::Uuid,
            DataType::Version,
            DataType::variant(),
            DataType::date32(),
            DataType::date64(),
            DataType::time(TimeUnit::Second).unwrap(),
            DataType::time(TimeUnit::Nanosecond).unwrap(),
            DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
            DataType::datetime64(TimeUnit::Microsecond, "UTC".parse::<Timezone>().unwrap())
                .unwrap(),
            DataType::duration32(TimeUnit::Second).unwrap(),
            DataType::duration64(TimeUnit::Nanosecond).unwrap(),
            DataType::interval(TimeUnit::YearMonth).unwrap(),
            DataType::interval(TimeUnit::DayTime).unwrap(),
            DataType::interval(TimeUnit::MonthDayNano).unwrap(),
            DataType::decimal32(9, 2).unwrap(),
            DataType::decimal64(18, 2).unwrap(),
            DataType::decimal128(38, 2).unwrap(),
            DataType::decimal256(76, 2).unwrap(),
            DataType::serie(DataType::Int32.nullable_field("item")),
            DataType::large_serie(DataType::Int32.nullable_field("item")),
            DataType::serie_view(DataType::Int32.nullable_field("item")),
            DataType::large_serie_view(DataType::Int32.nullable_field("item")),
            DataType::fixed_size_serie(DataType::Int32.nullable_field("item"), 4).unwrap(),
            DataType::from(StructType::from_fields([DataType::Int32.required_field("a")]).unwrap()),
            DataType::map_of(DataType::utf8(), DataType::Int32, false).unwrap(),
            DataType::map_of(DataType::utf8(), DataType::Int32, true).unwrap(),
            DataType::dictionary(DataType::Int32, DataType::utf8()).unwrap(),
            DataType::run_end_encoded(
                DataType::Int32.required_field("run_ends"),
                DataType::utf8().nullable_field("values"),
            )
            .unwrap(),
            DataType::dense_union([DataType::Int64.nullable_field("number")]).unwrap(),
            DataType::geometry(None).unwrap(),
            DataType::geography(None, None).unwrap(),
        ];

        let mut seen = std::collections::HashSet::<yggdryl::DataTypeId>::new();
        for value in values {
            seen.insert(value.id());
            let printed = value.to_string();
            let reparsed = printed
                .parse::<DataType>()
                .unwrap_or_else(|error| panic!("{printed} does not read back: {error}"));
            assert_eq!(reparsed, value, "{printed}");
            // The id's own name is a spelling too, wherever it takes no parameter
            // and names the datatype's own default: `string` is `utf8`, not the
            // US-ASCII string that shares its id.
            let named = value.id().as_str();
            let default_charset = value.charset().is_none_or(|charset| charset.is_utf8());
            if !printed.contains('(') && !printed.contains('<') && default_charset {
                assert_eq!(
                    named
                        .parse::<DataType>()
                        .unwrap_or_else(|error| panic!("{named}: {error}")),
                    value,
                    "{named}"
                );
            }
        }
        // Every parameterized id name is a grammar keyword too.
        for (named, expected) in [
            ("fixed_ascii(4)", DataType::fixed_ascii(4).unwrap()),
            ("fixed_binary(16)", DataType::fixed_binary(16).unwrap()),
            ("fixed_size_serie(int32, 4)", {
                DataType::fixed_size_serie(DataType::Int32.nullable_field("item"), 4).unwrap()
            }),
        ] {
            assert_eq!(named.parse::<DataType>().unwrap(), expected, "{named}");
        }
        assert!(
            seen.len() >= 40,
            "the list should name every variant, saw {}",
            seen.len()
        );
    }

    #[test]
    fn a_declared_sql_length_is_a_length() {
        // A string length and a binary length are both bounds this crate
        // stores. A declaration no storage could have meant is malformed input.
        assert_eq!(
            "varchar(10)".parse::<DataType>().unwrap().to_string(),
            "sized_utf8(10)"
        );
        assert_eq!(
            "char(1)".parse::<DataType>().unwrap().to_string(),
            "fixed_utf8(1)"
        );
        for (accepted, max) in [
            ("sized_binary(16)", 16_u32),
            ("varbinary(4)", 4),
            ("binary(8)", 8),
        ] {
            let parsed = accepted.parse::<DataType>().unwrap();
            assert_eq!(
                parsed.to_string(),
                format!("sized_binary({max})"),
                "{accepted}"
            );
            assert_eq!(
                parsed.bytes_parameters().unwrap().max(),
                Some(max),
                "{accepted}"
            );
        }
        for malformed in ["binary(-1)", "varbinary(0)"] {
            let refused = malformed.parse::<DataType>().unwrap_err().to_string();
            assert!(
                refused.contains("maximum") || refused.contains("width"),
                "{refused}"
            );
        }
        // One byte is a number like any other. The two leaves that *are* their
        // number carry a placeholder in `BytesType::ALL`, and reading "was a
        // number stated?" off that placeholder refused the one width that
        // happened to equal it.
        assert_eq!(
            "fixed_binary(1)".parse::<DataType>().unwrap(),
            DataType::fixed_binary(1).unwrap()
        );
        assert_eq!(
            "sized_binary(1)".parse::<DataType>().unwrap().to_string(),
            "sized_binary(1)"
        );
        // And a leaf that is its number still refuses to stand without one.
        for bare in ["fixed_binary", "sized_binary"] {
            let refused = bare.parse::<DataType>().unwrap_err().to_string();
            assert!(refused.contains(bare), "{refused}");
            assert!(refused.contains("got none"), "{refused}");
        }
        for malformed in ["varchar(0)", "char(-1)"] {
            assert!(malformed.parse::<DataType>().is_err(), "{malformed}");
        }
    }

    #[test]
    fn one_arrow_field_form_answers_to_one_key_set() {
        // arrow-rs prints `data_type`; this crate spells the same property
        // `dtype`. Both readings of the form take both, so a Debug line and a
        // canonical line describe the same field.
        let expected = Field::new("a", DataType::Int32, true);
        for spelling in [
            "Field { name: \"a\", data_type: Int32, nullable: true }",
            "Field { name: \"a\", dtype: Int32, nullable: true }",
            "field{name: \"a\", data_type: int32, nullable: true}",
            "field{name: \"a\", dtype: int32, is_nullable: true}",
        ] {
            let parsed =
                Field::from_str(spelling).unwrap_or_else(|error| panic!("{spelling}: {error}"));
            assert_eq!(parsed, expected, "{spelling}");
        }
    }

    #[test]
    fn a_field_spelled_without_nullability_is_nullable_wherever_it_sits() {
        let expected = DataType::serie(Field::new("a", DataType::Int32, true));
        assert_eq!(
            "serie(field(\"a\",int32))".parse::<DataType>().unwrap(),
            expected
        );
        assert_eq!(
            "field(\"a\",int32)".parse::<Field>().unwrap(),
            Field::new("a", DataType::Int32, true)
        );
    }

    #[test]
    fn a_parameter_free_type_displays_as_the_name_its_identifier_spells() {
        // `Display` used to restate a literal for each of the thirty-three
        // parameter-free variants, beside the same word in `DataTypeId::as_str`.
        // The two spellings agreed only because nothing had drifted yet; this
        // walks every identifier so a variant that displays as anything but its
        // own name fails here instead of silently breaking the round trip.
        for id in DataTypeId::ALL {
            if id.is_parameterized() {
                continue;
            }
            let Ok(dtype) = DataType::from_str(id.as_str()) else {
                continue; // an identifier with no standalone datatype spelling
            };
            assert_eq!(dtype.name(), id.as_str(), "{}", id.as_str());
            assert_eq!(dtype.to_string(), id.as_str(), "{}", id.as_str());
            assert_eq!(
                DataType::from_str(&dtype.to_string()).unwrap(),
                dtype,
                "{}",
                id.as_str()
            );
        }
    }
}

mod aliases {
    use yggdryl::{DataType, Field};

    #[test]
    fn scalar_aliases_and_balanced_outer_wrappers_normalize() {
        for value in [
            "bigint",
            "BIGINT",
            "(bigint)",
            "[ bigint ]",
            "{bigint}",
            "'bigint'",
            "\"bigint\"",
        ] {
            assert_eq!(
                DataType::from_str(value).unwrap(),
                DataType::Int64,
                "{value}"
            );
        }

        assert_eq!(DataType::from_str("varchar").unwrap(), DataType::utf8());
        // A declared length is the maximum the column holds, which Arrow has
        // nowhere to say and this crate carries in its own metadata.
        assert_eq!(
            DataType::from_str("varchar(255)").unwrap().to_string(),
            "sized_utf8(255)"
        );
        assert_eq!(
            DataType::from_str("double precision").unwrap(),
            DataType::Float64
        );
        assert_eq!(DataType::from_str("bytea").unwrap(), DataType::binary());
    }

    #[test]
    fn the_list_spellings_read_as_the_serie_layouts_and_display_the_serie_spelling() {
        let item = || DataType::Int64.nullable_field("item");
        let named = || Field::new("px", DataType::Int64, false);
        // Each layout: every spelling the grammar reads for it - its own
        // word, underscored and folded, then the list word it was spelled
        // with before it took its own, underscored, folded and cased - the
        // datatype they all read as, and the one spelling it displays.
        let layouts = [
            (
                &[
                    "serie<int64>",
                    "SERIE<int64>",
                    "list<int64>",
                    "LIST<int64>",
                    "List<int64>",
                    "array<int64>",
                ][..],
                DataType::serie(item()),
                "serie(field(\"item\",int64,nullable=true,metadata={}))",
            ),
            (
                &[
                    "serie_view<int64>",
                    "serieview<int64>",
                    "list_view<int64>",
                    "listview<int64>",
                    "ListView<int64>",
                    "LIST_VIEW<int64>",
                    "arrayview<int64>",
                ][..],
                DataType::serie_view(item()),
                "serie_view(field(\"item\",int64,nullable=true,metadata={}))",
            ),
            (
                &[
                    "large_serie<int64>",
                    "largeserie<int64>",
                    "large_list<int64>",
                    "largelist<int64>",
                    "LargeList<int64>",
                    "LARGE_LIST<int64>",
                    "LARGE-LIST<int64>",
                    "largearray<int64>",
                ][..],
                DataType::large_serie(item()),
                "large_serie(field(\"item\",int64,nullable=true,metadata={}))",
            ),
            (
                &[
                    "large_serie_view<int64>",
                    "largeserieview<int64>",
                    "large_list_view<int64>",
                    "largelistview<int64>",
                    "LargeListView<int64>",
                    "LARGE_LIST_VIEW<int64>",
                    "largearrayview<int64>",
                ][..],
                DataType::large_serie_view(item()),
                "large_serie_view(field(\"item\",int64,nullable=true,metadata={}))",
            ),
            (
                &[
                    "fixed_size_serie(int64, 3)",
                    "fixedsizeserie(int64, 3)",
                    "fixed_size_list(int64, 3)",
                    "fixedsizelist(int64, 3)",
                    "FixedSizeList(int64, 3)",
                    "FIXED_SIZE_LIST(int64, 3)",
                    "fixedarray(int64, 3)",
                ][..],
                DataType::fixed_size_serie(item(), 3).unwrap(),
                "fixed_size_serie(field(\"item\",int64,nullable=true,metadata={}),3)",
            ),
            (
                &[
                    "serie(field(\"px\", int64, false))",
                    "list(field(\"px\", int64, false))",
                ][..],
                DataType::serie(named()),
                "serie(field(\"px\",int64,nullable=false,metadata={}))",
            ),
            (
                &[
                    "fixed_size_serie(field(\"px\", int64, false), 3)",
                    "fixed_size_list(field(\"px\", int64, false), 3)",
                    "fixedsizelist(field(\"px\", int64, false), 3)",
                ][..],
                DataType::fixed_size_serie(named(), 3).unwrap(),
                "fixed_size_serie(field(\"px\",int64,nullable=false,metadata={}),3)",
            ),
        ];
        for (spellings, expected, displayed) in layouts {
            assert_eq!(expected.to_string(), displayed);
            for spelling in spellings {
                let read = DataType::from_str(spelling)
                    .unwrap_or_else(|error| panic!("{spelling}: {error}"));
                assert_eq!(read, expected, "{spelling}");
                assert_eq!(read.to_string(), displayed, "{spelling}");
                assert_eq!(DataType::from_str(&read.to_string()).unwrap(), expected);
            }
        }
        // Nested, the old word reads at every depth and displays the new one.
        let nested = DataType::from_str("list<large_list<fixed_size_list(int64, 2)>>").unwrap();
        assert_eq!(
            nested,
            DataType::serie(
                DataType::large_serie(
                    DataType::fixed_size_serie(item(), 2)
                        .unwrap()
                        .nullable_field("item")
                )
                .nullable_field("item")
            )
        );
        let displayed = nested.to_string();
        assert!(
            displayed
                .starts_with("serie(field(\"item\",large_serie(field(\"item\",fixed_size_serie("),
            "{displayed}"
        );
        assert!(!displayed.contains("list"), "{displayed}");
    }
}

mod families {

    use yggdryl::{DataType, StructType, TimeUnit};
    use yggdryl::{Error, Field};

    #[test]
    fn canonical_display_json_and_arrow_are_lossless() {
        let item = Field::from_parts(
            "item,東京",
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::from_parts(
                    "text",
                    DataType::utf8(),
                    true,
                    [("source", "quoted \"value\"")],
                )
                .unwrap(),
            ])
            .map(DataType::from)
            .unwrap(),
            true,
            [("doc", "nested, metadata")],
        )
        .unwrap();
        let value = DataType::serie(item);

        let canonical = value.to_string();
        assert_eq!(DataType::from_str(&canonical).unwrap(), value);
        let json = value.clone().into_json().unwrap();
        let structural: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(structural.is_object());
        assert_eq!(structural["type"], "serie");
        assert_eq!(structural["field"]["dtype"]["type"], "struct");
        assert_eq!(DataType::from_json(&json).unwrap(), value);
        assert_eq!(
            DataType::from_json(&value.clone().into_json().unwrap()).unwrap(),
            value
        );

        let arrow = value.clone().into_arrow_datatype().unwrap();
        assert_eq!(DataType::from_arrow_datatype(&arrow).unwrap(), value);
        assert_eq!(
            DataType::try_from(value.clone().into_arrow_datatype().unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn sql_hive_spark_and_arrow_spellings_parse_recursively() {
        let values = [
            "ROW(id INTEGER NOT NULL, payload VARBINARY, score DOUBLE PRECISION)",
            "struct<`quoted,name`:string,nested:map<string,array<decimal(38,18)>>>",
            "string[][]",
            "Dictionary(UInt16, List(Field { name: 'item', data_type: Utf8, nullable: true, metadata: {} }))",
            "Union([(0, Field { name: 'id', data_type: Int64, nullable: false, metadata: {} }), (7, Field { name: 'name', data_type: Utf8, nullable: true, metadata: {} })], Dense)",
            "run_end_encoded(int32,array<string>)",
            "fixed_size_list(string,16)",
        ];

        for source in values {
            let value =
                DataType::from_str(source).unwrap_or_else(|error| panic!("{source}: {error}"));
            assert_eq!(DataType::from_str(&value.to_string()).unwrap(), value);
        }
    }

    #[test]
    fn temporal_decimal_and_wrapper_forms_are_validated() {
        assert_eq!(
            DataType::from_str("timestamp(9,'Europe/Paris')").unwrap(),
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: yggdryl::Timezone::from_str("Europe/Paris").unwrap()
            }
        );
        assert_eq!(
            DataType::from_str("TIMESTAMP WITH TIME ZONE").unwrap(),
            DataType::DateTime64 {
                unit: TimeUnit::Microsecond,
                timezone: yggdryl::Timezone::UTC
            }
        );
        assert_eq!(
            DataType::from_str("interval year to month").unwrap(),
            DataType::interval(TimeUnit::YearMonth).unwrap()
        );
        assert_eq!(
            DataType::from_str("[{(decimal256(76,-20))}]").unwrap(),
            DataType::decimal256(76, -20).unwrap()
        );
        assert!(DataType::from_str("time32(ns)").is_err());
        assert!(DataType::from_str("decimal32(10,0)").is_err());
        assert!(DataType::from_str("timestamp(10)").is_err());
    }

    #[test]
    fn parser_reports_positions_and_rejects_adversarial_input() {
        for source in [
            "struct<a:int,a:string>",
            "map<string>",
            "array<struct<a:int>",
            "int64 trailing",
            "union(dense,0=a:int,0=b:string)",
            "'unterminated",
            "decimal(18,wat)",
        ] {
            assert!(
                matches!(
                    DataType::from_str(source),
                    Err(Error::Parse { .. }) | Err(Error::InvalidDataType { .. })
                ),
                "{source}"
            );
        }

        let mut overdeep = "int64".to_owned();
        for _ in 0..=DataType::PARSE_RECURSION_LIMIT {
            overdeep = format!("array<{overdeep}>");
        }
        assert!(matches!(
            DataType::from_str(&overdeep),
            Err(Error::Parse { position, .. }) if position > 0
        ));
    }
}

mod generic {

    use yggdryl::{DataType, Error, Field};

    #[test]
    fn canonical_field_preserves_unicode_quotes_commas_and_metadata() {
        let field = Field::from_parts(
            "prix,€\"",
            DataType::from_str("array<struct<clé:string,valeur:decimal(18,4)>>").unwrap(),
            false,
            [("a,b", "quoted \" value"), ("unicode", "東京")],
        )
        .unwrap();
        let canonical = field.to_string();
        let restored = Field::from_str(&canonical).unwrap();
        assert_eq!(restored, field);
        assert_eq!(restored.get_metadata("a,b"), Some("quoted \" value"));
    }

    #[test]
    fn parser_accepts_sql_hive_and_arrow_shapes() {
        let sql = Field::from_str("order_id BIGINT NOT NULL").unwrap();
        assert_eq!(sql.name(), "order_id");
        assert_eq!(sql.dtype(), &DataType::Int64);
        assert!(!sql.is_nullable());

        let hive = Field::from_str("events:array<struct<id:bigint,name:string>>").unwrap();
        assert!(hive.is_nullable());
        assert_eq!(hive.dtype().field_len(), 1);

        let arrow = Field::from_str(
            r#"Field { name: "id", dtype: Int64, nullable: false, metadata: {"source":"arrow"} }"#,
        )
        .unwrap();
        assert_eq!(
            arrow,
            Field::from_parts("id", DataType::Int64, false, [("source", "arrow")]).unwrap()
        );
    }

    #[test]
    fn parser_accepts_flexible_sql_whitespace_and_doubled_quotes() {
        let spaced = Field::from_str("trade_id\tBIGINT \n NOT \t NULL").unwrap();
        assert_eq!(spaced.name(), "trade_id");
        assert_eq!(spaced.dtype(), &DataType::Int64);
        assert!(!spaced.is_nullable());

        let single_quoted = Field::from_str("'owner''s code'   VARCHAR(32)").unwrap();
        assert_eq!(single_quoted.name(), "owner's code");
        assert_eq!(single_quoted.dtype().to_string(), "sized_utf8(32)");

        let double_quoted = Field::from_str(r#""desk""label" STRING"#).unwrap();
        assert_eq!(double_quoted.name(), "desk\"label");
        assert_eq!(double_quoted.dtype(), &DataType::utf8());
    }

    #[test]
    fn parser_errors_report_original_input_byte_offsets() {
        let input = r#"  ((field("id", definitely_bad, nullable=true, metadata={})))"#;
        let expected = input.find("definitely_bad").unwrap();
        let error = Field::from_str(input).unwrap_err();
        assert!(
            matches!(
                error,
                Error::Parse {
                    target: "field",
                    position,
                    ..
                } if position == expected
            ),
            "expected byte {expected}, got {error:?}"
        );
    }

    #[test]
    fn parser_rejects_invalid_nullability_and_duplicate_metadata() {
        for malformed in [
            "field(\"id\",int64,nullable=maybe,metadata={})",
            "field(\"id\",int64,nullable=true,metadata={\"a\":\"1\",\"a\":\"2\"})",
        ] {
            assert!(Field::from_str(malformed).is_err(), "{malformed}");
        }
    }

    #[test]
    fn parser_recursion_limit_has_an_exact_public_boundary() {
        let accepted = format!(
            "{}id:int64{}",
            "(".repeat(DataType::PARSE_RECURSION_LIMIT),
            ")".repeat(DataType::PARSE_RECURSION_LIMIT)
        );
        assert_eq!(Field::from_str(&accepted).unwrap().name(), "id");

        let rejected = format!("({accepted})");
        assert!(Field::from_str(&rejected).is_err());
    }
}
