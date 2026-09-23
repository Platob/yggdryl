//! `rust/src/diff.rs`: the borrowed field-pair walk no caller can reach.
//!
//! `Differences::from_fields` is crate-private: it is the borrowed walk
//! [`OwnedDifferences::from_fields`] owns its answers from, so the wide
//! cases have to be measured from inside. What a caller can observe lives
//! beside it here.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::diff::{
        differences_from_fields, owned_differences_from_fields, pending_is_empty, work_len,
    };
    use yggdryl::internals::metadata::shares_storage_with;
    use yggdryl::{DataType, Field, StructType};

    fn wide_struct(prefix: &str) -> DataType {
        StructType::from_fields(
            (0..1_024)
                .map(|index| Field::new(format!("{prefix}_{index:04}"), DataType::Int64, false)),
        )
        .map(DataType::from)
        .unwrap()
    }

    #[test]
    fn wide_slice_work_stays_bounded_before_the_first_difference() {
        let left = Field::new("root", wide_struct("left"), false);
        let right = Field::new("root", wide_struct("right"), false);
        let mut differences = differences_from_fields(&left, &right, true, false);
        assert_eq!(work_len(&differences), 1);
        assert!(pending_is_empty(&differences));

        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.dtype.fields[0].name: \"left_0000\" → \"right_0000\"")
        );
        assert!(work_len(&differences) <= 2);
        assert!(pending_is_empty(&differences));

        let left = Field::new(
            "root",
            DataType::from(StructType::from_fields(std::iter::empty()).unwrap()),
            false,
        );
        let right = Field::new("root", wide_struct("added"), false);
        let mut differences = differences_from_fields(&left, &right, true, false);
        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.dtype.field_count: 0 → 1024")
        );
        assert_eq!(work_len(&differences), 1);
        assert!(pending_is_empty(&differences));
    }

    #[test]
    fn physical_first_difference_does_not_scan_equal_wide_metadata() {
        let entries = (0..1_024)
            .map(|index| (format!("key_{index:04}"), format!("value_{index:04}")))
            .collect::<Vec<_>>();
        let left = Field::from_parts("root", DataType::Int32, false, entries.clone()).unwrap();
        let right = Field::from_parts("root", DataType::Int64, false, entries).unwrap();
        assert!(!shares_storage_with(
            left.as_metadata(),
            right.as_metadata()
        ));

        let mut differences = differences_from_fields(&left, &right, true, false);
        assert_eq!(work_len(&differences), 1);
        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.dtype.kind: int32 → int64")
        );
        assert_eq!(work_len(&differences), 1);
        assert!(pending_is_empty(&differences));
    }

    #[test]
    fn shared_deep_snapshots_complete_without_traversal() {
        let mut dtype = DataType::Int64;
        for depth in 0..64 {
            dtype = DataType::list(Field::new(format!("item_{depth}"), dtype, false));
        }
        let left = Field::new("root", dtype, false);
        let right = left.clone();
        let mut differences = differences_from_fields(&left, &right, true, false);
        assert!(work_len(&differences) == 0);
        assert_eq!(differences.next(), None);
    }

    #[test]
    fn owned_cursor_outlives_source_snapshots() {
        let mut differences = {
            let left = Field::new("left", wide_struct("left"), false);
            let right = Field::new("right", wide_struct("right"), false);
            owned_differences_from_fields(&left, &right, true, false)
        };
        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.name: \"left\" → \"right\"")
        );
        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.dtype.fields[0].name: \"left_0000\" → \"right_0000\"")
        );
    }
}

mod comparison {
    use yggdryl::{DataType, Field, StructType, TimeUnit, Timezone, UnionMode};

    #[test]
    fn equals_can_ignore_only_metadata_recursively() {
        let left_child =
            Field::from_parts("item", DataType::utf8(), true, [("source", "left")]).unwrap();
        let right_child =
            Field::from_parts("item", DataType::utf8(), true, [("source", "right")]).unwrap();
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
            DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap(),
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
                Field::from_parts("item", DataType::utf8(), false, [("side", "left")]).unwrap(),
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
                    DataType::utf8(),
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
            lines.iter().any(|line| {
                line == "≠ $.dtype.item.metadata[\"side\"]: \"left\" → \"right\""
            })
        );
        assert_eq!(left.show_diff(&right, true, true), lines.join("\n"));
    }

    #[test]
    fn differences_without_metadata_do_not_render_nested_metadata_as_context() {
        let private_child =
            Field::from_parts("private", DataType::utf8(), true, [("secret", "left")]).unwrap();
        let left = DataType::list(private_child);
        let right = DataType::from(
            StructType::from_fields([Field::new("public", DataType::utf8(), true)]).unwrap(),
        );
        let changed_kind = left.show_diff(&right, false, true);
        assert_eq!(changed_kind, "≠ $.kind: list → struct");
        assert!(!changed_kind.contains("secret"));

        let left = DataType::from(StructType::from_fields(std::iter::empty()).unwrap());
        let right = StructType::from_fields([Field::from_parts(
            "added",
            DataType::list(
                Field::from_parts("item", DataType::utf8(), true, [("secret", "right")]).unwrap(),
            ),
            true,
            [("secret", "root")],
        )
        .unwrap()])
        .map(DataType::from)
        .unwrap();
        let added = left.show_diff(&right, false, true);
        assert!(added.contains("+ $.fields[0]: field(name=\"added\",dtype=list,nullable=true)"));
        assert!(!added.contains("secret"));
    }

    #[test]
    fn empty_diff_exactly_matches_equality_for_parameterized_and_nested_types() {
        let item = || Field::new("item", DataType::utf8(), true);
        let pairs = vec![
            (
                DataType::DateTime64 {
                    unit: TimeUnit::Second,
                    timezone: Timezone::NAIVE,
                },
                DataType::DateTime64 {
                    unit: TimeUnit::Second,
                    timezone: Timezone::UTC,
                },
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
                DataType::interval(TimeUnit::YearMonth).unwrap(),
                DataType::interval(TimeUnit::DayTime).unwrap(),
            ),
            (
                DataType::fixed_binary(8).unwrap(),
                DataType::fixed_binary(16).unwrap(),
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
                DataType::dictionary(DataType::Int8, DataType::utf8()).unwrap(),
                DataType::dictionary(DataType::Int16, DataType::utf8()).unwrap(),
            ),
            (
                DataType::decimal128(10, 2).unwrap(),
                DataType::decimal128(11, 2).unwrap(),
            ),
            (
                DataType::map_of(DataType::utf8(), DataType::Int64, false).unwrap(),
                DataType::map_of(DataType::utf8(), DataType::Int64, true).unwrap(),
            ),
            (
                DataType::run_end_encoded(
                    Field::new("run_ends", DataType::Int32, false),
                    Field::new("values", DataType::utf8(), true),
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
        let left = Field::from_parts("value", DataType::utf8(), true, entries("left")).unwrap();
        let right = Field::from_parts("value", DataType::utf8(), true, entries("right")).unwrap();
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
                (1, Field::new("text", DataType::utf8(), true)),
            ],
            UnionMode::Dense,
        )
        .unwrap();
        let right = DataType::union(
            [
                (2, Field::new("number", DataType::Int64, false)),
                (1, Field::new("text", DataType::utf8(), true)),
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
        let right = Field::new("value", DataType::utf8(), true);

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
            .show_diffs(&DataType::utf8(), true, true)
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
        let text = DataType::utf8().nullable_field("payload");
        let diff = variant.show_diff(&text, false, false);
        assert!(diff.contains("variant"), "{diff}");
        assert_eq!(
            variant.show_diff(&variant.clone(), false, true),
            "\u{2713} equal"
        );
    }
}
