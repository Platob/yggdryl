use yggdryl::{DataType, Field, TimeUnit, Timezone};

#[test]
fn variant_parser_alias_canonicalizes_to_dense_union() {
    let expected = DataType::dense_union([
        Field::new("number", DataType::Int64, true),
        Field::new("text", DataType::Utf8, true),
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
        Field::new("sparse", DataType::Utf8, true),
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
fn bare_interval_defaults_before_postfix_list_wrapping() {
    let interval = DataType::Interval(TimeUnit::MonthDayNano);
    let list = DataType::list(Field::new("item", interval, true));
    let nested_list = DataType::list(Field::new("item", list.clone(), true));

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
        DataType::Binary,
        DataType::fixed_size_binary(16).unwrap(),
        DataType::LargeBinary,
        DataType::BinaryView,
        DataType::Utf8,
        DataType::LargeUtf8,
        DataType::Utf8View,
        DataType::Ascii,
        DataType::ascii(4).unwrap(),
        DataType::Country,
        DataType::Currency,
        DataType::Mic,
        DataType::Cfi,
        DataType::Uuid,
        DataType::Version,
        DataType::variant(),
        DataType::Date32,
        DataType::Date64,
        DataType::time(TimeUnit::Second).unwrap(),
        DataType::time(TimeUnit::Nanosecond).unwrap(),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
        DataType::datetime64(TimeUnit::Microsecond, "UTC".parse::<Timezone>().unwrap()).unwrap(),
        DataType::duration32(TimeUnit::Second).unwrap(),
        DataType::duration64(TimeUnit::Nanosecond).unwrap(),
        DataType::Interval(TimeUnit::YearMonth),
        DataType::Interval(TimeUnit::DayTime),
        DataType::Interval(TimeUnit::MonthDayNano),
        DataType::decimal32(9, 2).unwrap(),
        DataType::decimal64(18, 2).unwrap(),
        DataType::decimal128(38, 2).unwrap(),
        DataType::decimal256(76, 2).unwrap(),
        DataType::list(DataType::Int32.nullable_field("item")),
        DataType::large_list(DataType::Int32.nullable_field("item")),
        DataType::list_view(DataType::Int32.nullable_field("item")),
        DataType::large_list_view(DataType::Int32.nullable_field("item")),
        DataType::fixed_size_list(DataType::Int32.nullable_field("item"), 4).unwrap(),
        DataType::from_fields([DataType::Int32.required_field("a")]).unwrap(),
        DataType::map_of(DataType::Utf8, DataType::Int32, false).unwrap(),
        DataType::map_of(DataType::Utf8, DataType::Int32, true).unwrap(),
        DataType::dictionary(DataType::Int32, DataType::Utf8).unwrap(),
        DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Utf8.nullable_field("values"),
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
        // The id's own name is a spelling too, wherever it takes no parameter.
        let named = value.id().as_str();
        if !printed.contains('(') && !printed.contains('<') {
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
        ("fixed_ascii(4)", DataType::ascii(4).unwrap()),
        (
            "fixed_size_binary(16)",
            DataType::fixed_size_binary(16).unwrap(),
        ),
        ("fixed_size_list(int32, 4)", {
            DataType::fixed_size_list(DataType::Int32.nullable_field("item"), 4).unwrap()
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
    // A string length is a bound this crate stores; a binary one still says
    // nothing its variable storage holds. Either way a declaration no storage
    // could have meant is malformed input.
    assert_eq!(
        "varchar(10)".parse::<DataType>().unwrap().to_string(),
        "utf8(10)"
    );
    assert_eq!(
        "char(1)".parse::<DataType>().unwrap().to_string(),
        "fixed_utf8(1)"
    );
    for accepted in ["binary(16)", "varbinary(4)"] {
        assert_eq!(accepted.parse::<DataType>().unwrap(), DataType::Binary);
    }
    for malformed in ["binary(-1)", "varbinary(0)"] {
        let refused = malformed.parse::<DataType>().unwrap_err().to_string();
        assert!(refused.contains("positive number of bytes"), "{refused}");
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
    let expected = DataType::list(Field::new("a", DataType::Int32, true));
    assert_eq!(
        "list(field(\"a\",int32))".parse::<DataType>().unwrap(),
        expected
    );
    assert_eq!(
        "field(\"a\",int32)".parse::<Field>().unwrap(),
        Field::new("a", DataType::Int32, true)
    );
}
