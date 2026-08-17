//! Edge cases for glob detection, decomposition, matching, and Hive parsing.

use crate::Url;

fn url(value: &str) -> Url {
    Url::from_str(value).expect("a valid URL")
}

mod detection {
    use super::{Url, url};

    #[test]
    fn plain_names_are_not_globs() {
        assert!(!url("file:///data/trades.arrows").is_glob());
        assert!(!url("s3://bucket/lake/year=2024/part-0.parquet").is_glob());
        assert!(!url("file:///data/").is_glob());
    }

    #[test]
    fn every_pattern_character_marks_a_glob() {
        for value in [
            "file:///data/*.parquet",
            "file:///data/part-*",
            "file:///data/**/x.arrows",
            "s3://bucket/lake/*/x.arrows",
        ] {
            assert!(url(value).is_glob(), "{value} should be a glob");
        }
    }

    #[test]
    fn reserved_pattern_characters_do_not_parse_as_a_location() {
        // `[` belongs to an IPv6 host, so a class reaches matching as text.
        assert!(Url::from_str("file:///data/part-[0-9].arrows").is_err());
        assert!(url("file:///data/part-7.arrows").matches_glob("part-[0-9].arrows"));
    }

    #[test]
    fn a_question_mark_in_a_url_is_a_query_not_a_pattern() {
        // `?` opens the query, so it never reaches a path segment. It stays a
        // pattern character for the text form passed to `matches_glob`.
        let value = url("file:///data/trades-?.arrows");

        assert!(!value.is_glob());
        assert_eq!(value.query(), Some(".arrows"));
        assert!(url("file:///data/trades-7.arrows").matches_glob("trades-?.arrows"));
    }

    #[test]
    fn only_double_star_is_recursive() {
        assert!(url("file:///data/**/*.parquet").is_recursive_glob());
        assert!(!url("file:///data/*.parquet").is_recursive_glob());
        // A star inside a name is not a segment of its own.
        assert!(!url("file:///data/a**b.parquet").is_recursive_glob());
    }

    #[test]
    fn a_glob_survives_a_round_trip_through_text() {
        let value = "file:///data/**/*.parquet";
        assert_eq!(
            Url::from_str(value).expect("a valid URL").to_string(),
            value
        );
    }
}

mod decomposition {
    use super::url;

    #[test]
    fn a_plain_location_is_its_own_root() {
        let (root, pattern) = url("file:///data/trades.arrows")
            .glob_parts()
            .expect("a decomposition");

        assert_eq!(root.to_string(), "file:///data/trades.arrows");
        assert_eq!(pattern, None);
    }

    #[test]
    fn the_root_stops_at_the_first_pattern_segment() {
        let (root, pattern) = url("file:///lake/trades/year=2024/**/*.parquet")
            .glob_parts()
            .expect("a decomposition");

        assert_eq!(root.to_string(), "file:///lake/trades/year=2024");
        assert_eq!(pattern.as_deref(), Some("**/*.parquet"));
    }

    #[test]
    fn a_pattern_in_the_first_segment_leaves_the_bare_root() {
        let (root, pattern) = url("file:///*.parquet")
            .glob_parts()
            .expect("a decomposition");

        assert_eq!(root.to_string(), "file:///");
        assert_eq!(pattern.as_deref(), Some("*.parquet"));
    }

    #[test]
    fn decomposition_keeps_the_scheme_and_authority() {
        let (root, pattern) = url("s3://bucket/lake/*/part-*.parquet")
            .glob_parts()
            .expect("a decomposition");

        assert_eq!(root.to_string(), "s3://bucket/lake");
        assert_eq!(pattern.as_deref(), Some("*/part-*.parquet"));
    }
}

mod matching {
    use super::url;

    #[test]
    fn a_pattern_without_a_separator_matches_the_name_at_any_depth() {
        assert!(url("file:///data/trades.parquet").matches_glob("*.parquet"));
        assert!(url("file:///data/nested/trades.parquet").matches_glob("*.parquet"));
        assert!(!url("file:///data/trades.arrows").matches_glob("*.parquet"));
    }

    #[test]
    fn a_pattern_with_a_separator_is_anchored_at_the_root() {
        assert!(url("file:///data/nested/trades.parquet").matches_glob("data/*/*.parquet"));
        // A single star stays inside one segment, so it cannot span `nested`.
        assert!(!url("file:///data/nested/trades.parquet").matches_glob("data/*.parquet"));
    }

    #[test]
    fn a_double_star_crosses_any_number_of_segments() {
        let deep = url("file:///lake/a/b/c/part-0.parquet");

        assert!(deep.matches_glob("**/*.parquet"));
        assert!(deep.matches_glob("lake/**/part-0.parquet"));
        // Zero segments is a legal expansion of `**`.
        assert!(url("file:///lake/part-0.parquet").matches_glob("lake/**/part-0.parquet"));
    }

    #[test]
    fn a_question_mark_matches_exactly_one_character() {
        assert!(url("file:///data/part-7.arrows").matches_glob("data/part-?.arrows"));
        assert!(!url("file:///data/part-70.arrows").matches_glob("data/part-?.arrows"));
    }

    #[test]
    fn a_character_class_selects_one_character() {
        let url = url("file:///data/part-7.arrows");

        assert!(url.matches_glob("part-[0-9].arrows"));
        assert!(url.matches_glob("part-[!a-z].arrows"));
        assert!(!url.matches_glob("part-[a-z].arrows"));
        assert!(!url.matches_glob("part-[0-9][0-9].arrows"));
    }

    #[test]
    fn an_unterminated_class_is_matched_literally() {
        // A URL cannot carry a bracket, so the matcher is exercised directly.
        use super::super::matches_segment;

        assert!(matches_segment("part-[0.arrows", "part-[0.arrows"));
        assert!(!matches_segment("part-0.arrows", "part-[0.arrows"));
        assert!(matches_segment("[a-b", "[a-b"));
        assert!(matches_segment("[!x", "[!x"));
    }

    #[test]
    fn a_star_matches_the_empty_string() {
        assert!(url("file:///data/.parquet").matches_glob("data/*.parquet"));
    }

    #[test]
    fn a_pattern_that_runs_out_of_path_does_not_match() {
        assert!(!url("file:///data").matches_glob("data/*/x.parquet"));
        assert!(!url("file:///data/a/b").matches_glob("data/a"));
    }

    #[test]
    fn a_relative_match_is_anchored_at_the_root_it_was_split_from() {
        let root = url("file:///lake/trades");
        let child = url("file:///lake/trades/year=2024/part-0.parquet");

        assert!(child.matches_glob_under(&root, "**/*.parquet"));
        assert!(child.matches_glob_under(&root, "year=*/part-0.parquet"));
        // Anchored at the root, one segment cannot cover two.
        assert!(!child.matches_glob_under(&root, "*.parquet"));
    }

    #[test]
    fn a_location_outside_the_root_never_matches() {
        let root = url("file:///lake/trades");

        assert!(
            !url("file:///lake/quotes/part-0.parquet").matches_glob_under(&root, "**/*.parquet")
        );
        assert!(!url("s3://bucket/lake/trades/part-0.parquet").matches_glob_under(&root, "**"));
        assert_eq!(url("file:///lake/quotes").segments_under(&root), None);
    }

    #[test]
    fn a_root_is_below_itself_with_nothing_left_over() {
        let root = url("file:///lake/trades");

        assert_eq!(root.segments_under(&root), Some(Vec::new()));
        // A name that merely starts the same way is not a child.
        assert_eq!(url("file:///lake/trades-old/x").segments_under(&root), None);
    }

    #[test]
    fn matching_is_literal_outside_the_pattern_characters() {
        assert!(!url("file:///data/trades.arrows").matches_glob("data/trades.parquet"));
        assert!(url("file:///data/year=2024/x.parquet").matches_glob("data/year=*/*.parquet"));
    }
}

mod hive {
    use super::url;

    #[test]
    fn partition_directories_are_read_in_path_order() {
        let url = url("file:///lake/trades/year=2024/month=01/part-0.parquet");

        assert_eq!(
            url.hive_partitions(),
            vec![
                ("year".to_owned(), "2024".to_owned()),
                ("month".to_owned(), "01".to_owned()),
            ]
        );
        assert_eq!(url.hive_partition("month").as_deref(), Some("01"));
        assert_eq!(url.hive_partition("day"), None);
    }

    #[test]
    fn a_path_without_pairs_carries_no_partitions() {
        let url = url("file:///lake/trades/part-0.parquet");

        assert!(!url.is_hive_partitioned());
        assert!(url.hive_partitions().is_empty());
    }

    #[test]
    fn an_empty_value_is_still_a_partition() {
        // Hive writes `key=` for a null partition value, and losing that pair
        // would silently drop the column from a partitioned read.
        assert_eq!(
            url("file:///lake/year=/part-0.parquet").hive_partitions(),
            vec![("year".to_owned(), String::new())]
        );
    }

    #[test]
    fn a_leading_equals_is_not_a_pair() {
        assert!(
            url("file:///lake/=2024/part-0.parquet")
                .hive_partitions()
                .is_empty()
        );
    }

    #[test]
    fn extra_equals_signs_belong_to_the_value() {
        assert_eq!(
            url("file:///lake/expr=a=b/part-0.parquet").hive_partitions(),
            vec![("expr".to_owned(), "a=b".to_owned())]
        );
    }

    #[test]
    fn a_partition_can_be_appended_and_read_back() {
        let partitioned = url("file:///lake/trades")
            .with_hive_partition("year", "2024")
            .expect("a partition directory");

        assert_eq!(partitioned.to_string(), "file:///lake/trades/year=2024");
        assert_eq!(partitioned.hive_partition("year").as_deref(), Some("2024"));
    }
}
