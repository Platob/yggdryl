use yggdryl::{DataType, Field, TimeUnit, Timezone, UnionMode};

#[test]
fn equals_can_ignore_only_metadata_recursively() {
    let left_child = Field::from_parts("item", DataType::Utf8, true, [("source", "left")]).unwrap();
    let right_child =
        Field::from_parts("item", DataType::Utf8, true, [("source", "right")]).unwrap();
    let left = DataType::list(left_child);
    let right = DataType::list(right_child);

    assert!(!left.equals(&right, true));
    assert!(left.equals(&right, false));
    assert_eq!(left.show_diffs(&right, false, false).count(), 0);
    assert_eq!(left.show_diff(&right, false, true), "✓ equal");
    assert_eq!(
        left.show_diffs(&right, true, false).collect::<Vec<_>>(),
        vec![r#"≠ $.item.metadata["source"]: "left" → "right""#]
    );

    let mut left_dictionary = Field::new(
        "code",
        DataType::dictionary(DataType::Int16, DataType::Utf8).unwrap(),
        false,
    );
    let mut right_dictionary = left_dictionary.clone();
    left_dictionary.set_dictionary_options(1, false).unwrap();
    right_dictionary.set_dictionary_options(2, false).unwrap();
    assert!(!left_dictionary.equals(&right_dictionary, false));
}

#[test]
fn show_diff_reports_deep_physical_and_metadata_changes() {
    let left = Field::from_parts(
        "payload",
        DataType::fixed_size_list(
            Field::from_parts("item", DataType::Utf8, false, [("side", "left")]).unwrap(),
            2,
        )
        .unwrap(),
        false,
        [("root", "left"), ("removed", "yes")],
    )
    .unwrap();
    let right = Field::from_parts(
        "body",
        DataType::fixed_size_list(
            Field::from_parts(
                "item",
                DataType::Utf8,
                true,
                [("added", "yes"), ("side", "right")],
            )
            .unwrap(),
            3,
        )
        .unwrap(),
        true,
        [("root", "right")],
    )
    .unwrap();

    let lines = left.show_diffs(&right, true, false).collect::<Vec<_>>();
    assert!(
        lines
            .iter()
            .any(|line| line == "≠ $.name: \"payload\" → \"body\"")
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "≠ $.nullable: false → true")
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("− $.metadata[\"removed\"]"))
    );
    assert!(lines.iter().any(|line| line == "≠ $.dtype.length: 2 → 3"));
    assert!(
        lines
            .iter()
            .any(|line| { line == "≠ $.dtype.item.metadata[\"side\"]: \"left\" → \"right\"" })
    );
    assert_eq!(left.show_diff(&right, true, true), lines.join("\n"));
}

#[test]
fn differences_without_metadata_do_not_render_nested_metadata_as_context() {
    let private_child =
        Field::from_parts("private", DataType::Utf8, true, [("secret", "left")]).unwrap();
    let left = DataType::list(private_child);
    let right = DataType::from_fields([Field::new("public", DataType::Utf8, true)]).unwrap();
    let changed_kind = left.show_diff(&right, false, true);
    assert_eq!(changed_kind, "≠ $.kind: list → struct");
    assert!(!changed_kind.contains("secret"));

    let left = DataType::from_fields(std::iter::empty()).unwrap();
    let right = DataType::from_fields([Field::from_parts(
        "added",
        DataType::list(
            Field::from_parts("item", DataType::Utf8, true, [("secret", "right")]).unwrap(),
        ),
        true,
        [("secret", "root")],
    )
    .unwrap()])
    .unwrap();
    let added = left.show_diff(&right, false, true);
    assert!(added.contains("+ $.fields[0]: field(name=\"added\",dtype=list,nullable=true)"));
    assert!(!added.contains("secret"));
}

#[test]
fn empty_diff_exactly_matches_equality_for_parameterized_and_nested_types() {
    let item = || Field::new("item", DataType::Utf8, true);
    let pairs = vec![
        (
            DataType::Timestamp(TimeUnit::Second, None),
            DataType::Timestamp(TimeUnit::Second, Some(Timezone::UTC)),
        ),
        (
            DataType::Time32(TimeUnit::Second),
            DataType::Time32(TimeUnit::Millisecond),
        ),
        (
            DataType::Time64(TimeUnit::Microsecond),
            DataType::Time64(TimeUnit::Nanosecond),
        ),
        (
            DataType::Duration32(TimeUnit::Second),
            DataType::Duration32(TimeUnit::Nanosecond),
        ),
        (
            DataType::Duration64(TimeUnit::Second),
            DataType::Duration64(TimeUnit::Nanosecond),
        ),
        (
            DataType::Duration32(TimeUnit::Second),
            DataType::Duration64(TimeUnit::Second),
        ),
        (
            DataType::Interval(TimeUnit::YearMonth),
            DataType::Interval(TimeUnit::DayTime),
        ),
        (
            DataType::fixed_size_binary(8).unwrap(),
            DataType::fixed_size_binary(16).unwrap(),
        ),
        (
            DataType::fixed_size_list(item(), 2).unwrap(),
            DataType::fixed_size_list(item(), 3).unwrap(),
        ),
        (
            DataType::union([(0, item())], UnionMode::Sparse).unwrap(),
            DataType::union([(0, item())], UnionMode::Dense).unwrap(),
        ),
        (
            DataType::dictionary(DataType::Int8, DataType::Utf8).unwrap(),
            DataType::dictionary(DataType::Int16, DataType::Utf8).unwrap(),
        ),
        (
            DataType::decimal128(10, 2).unwrap(),
            DataType::decimal128(11, 2).unwrap(),
        ),
        (
            DataType::map_of(DataType::Utf8, DataType::Int64, false).unwrap(),
            DataType::map_of(DataType::Utf8, DataType::Int64, true).unwrap(),
        ),
        (
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::Utf8, true),
            )
            .unwrap(),
            DataType::run_end_encoded(
                Field::new("run_ends", DataType::Int32, false),
                Field::new("values", DataType::Int64, true),
            )
            .unwrap(),
        ),
    ];

    for (left, right) in pairs {
        assert_eq!(
            left.show_diffs(&right, true, false).next().is_none(),
            left.equals(&right, true),
            "{} compared with {}",
            left,
            right
        );
        let independently_built = DataType::from_str(&left.to_string()).unwrap();
        assert!(left.equals(&independently_built, true));
        assert_eq!(
            left.show_diffs(&independently_built, true, false).next(),
            None
        );
    }
}

#[test]
fn wide_metadata_differences_stream_in_lexical_order_and_fuse() {
    let entries = |side: &str| {
        (0..1_024)
            .map(|index| (format!("key-{index:04}"), format!("{side}-{index:04}")))
            .collect::<Vec<_>>()
    };
    let left = Field::from_parts("value", DataType::Utf8, true, entries("left")).unwrap();
    let right = Field::from_parts("value", DataType::Utf8, true, entries("right")).unwrap();
    let mut differences = left.show_diffs(&right, true, false);

    assert_eq!(
        differences.next().as_deref(),
        Some(r#"≠ $.metadata["key-0000"]: "left-0000" → "right-0000""#)
    );
    assert_eq!(
        differences.next().as_deref(),
        Some(r#"≠ $.metadata["key-0001"]: "left-0001" → "right-0001""#)
    );
    assert_eq!(differences.count(), 1_022);
}

#[test]
fn union_difference_cursor_preserves_type_id_extra_and_child_order() {
    let left = DataType::union(
        [
            (0, Field::new("number", DataType::Int64, false)),
            (1, Field::new("text", DataType::Utf8, true)),
        ],
        UnionMode::Dense,
    )
    .unwrap();
    let right = DataType::union(
        [
            (2, Field::new("number", DataType::Int64, false)),
            (1, Field::new("text", DataType::Utf8, true)),
            (3, Field::new("flag", DataType::Boolean, false)),
        ],
        UnionMode::Dense,
    )
    .unwrap();

    assert_eq!(
        left.show_diffs(&right, true, false).collect::<Vec<_>>(),
        vec![
            "≠ $.field_count: 2 → 3",
            "≠ $.fields[0].type_id: 0 → 2",
            "+ $.fields[2]: type_id=3, field(\"flag\",boolean,nullable=false,metadata={})",
        ]
    );
}

#[test]
fn return_equal_controls_what_an_equal_comparison_reports() {
    let left = Field::new("value", DataType::Int64, true);
    let right = left.clone();

    // Silent by default: an equal comparison yields nothing at all.
    assert_eq!(left.show_diffs(&right, true, false).count(), 0);
    assert_eq!(left.show_diff(&right, true, false), "");

    // Opted in: exactly one equal line, never more.
    let lines: Vec<String> = left.show_diffs(&right, true, true).collect();
    assert_eq!(lines, vec!["\u{2713} equal".to_owned()]);
    assert_eq!(left.show_diff(&right, true, true), "\u{2713} equal");

    // The same contract holds at the datatype level.
    assert_eq!(DataType::Int64.show_diff(&DataType::Int64, true, false), "");
    assert_eq!(
        DataType::Int64.show_diff(&DataType::Int64, true, true),
        "\u{2713} equal"
    );
}

#[test]
fn return_equal_never_appears_when_values_differ() {
    let left = Field::new("value", DataType::Int64, true);
    let right = Field::new("value", DataType::Utf8, true);

    for return_equal in [false, true] {
        let lines: Vec<String> = left.show_diffs(&right, true, return_equal).collect();
        assert!(!lines.is_empty(), "{return_equal}");
        assert!(
            !lines.iter().any(|line| line.contains("equal")),
            "{lines:?}"
        );
        assert!(!left.show_diff(&right, true, return_equal).contains("equal"));
    }

    // A datatype difference behaves the same way.
    let lines: Vec<String> = DataType::Int64
        .show_diffs(&DataType::Utf8, true, true)
        .collect();
    assert!(
        !lines.iter().any(|line| line.contains("equal")),
        "{lines:?}"
    );
}

#[test]
fn geospatial_and_variant_differences_render_their_canonical_display() {
    let geometry = DataType::geometry(None).unwrap().nullable_field("shape");
    let geography = DataType::geography(None, None)
        .unwrap()
        .nullable_field("shape");
    assert!(!geometry.equals(&geography, false));
    let diff = geometry.show_diff(&geography, false, false);
    assert!(diff.contains("geometry"), "{diff}");
    assert!(diff.contains("geography"), "{diff}");

    let default_crs = DataType::geometry(None).unwrap().nullable_field("shape");
    let web_mercator = DataType::geometry(Some("EPSG:3857"))
        .unwrap()
        .nullable_field("shape");
    let diff = default_crs.show_diff(&web_mercator, false, false);
    assert!(diff.contains("EPSG:3857"), "{diff}");

    let variant = DataType::variant().nullable_field("payload");
    let text = DataType::Utf8.nullable_field("payload");
    let diff = variant.show_diff(&text, false, false);
    assert!(diff.contains("variant"), "{diff}");
    assert_eq!(
        variant.show_diff(&variant.clone(), false, true),
        "\u{2713} equal"
    );
}
