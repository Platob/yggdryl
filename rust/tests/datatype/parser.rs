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
            "timestamp(Second)",
            DataType::Timestamp(TimeUnit::Second, None),
        ),
        (
            "timestamp(Nanoseconds,UTC)",
            DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
        ),
        (
            "timestamp(nano seconds,UTC)",
            DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
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
}

#[test]
fn datatype_parser_rejects_time_unit_category_mismatches() {
    for source in [
        "timestamp(year_month)",
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
        ("timestamp(fortnight)", 10),
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
