//! `rust/src/excel/formula/criteria.rs`: the wildcard matcher Find, Replace and the criteria functions share - `*`, `?` and `~` as Excel reads them, the leftmost match and the longest from it, one pass over the text.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::excel_formula_criteria::{count_whole_matches, find, replace};

    #[test]
    fn a_pattern_finds_the_leftmost_match_and_the_longest_from_it() {
        for (pattern, text, from, expected) in [
            ("b", "abcb", 0, Some((1, 2))),
            ("b", "abcb", 2, Some((3, 4))),
            ("a*c", "xabcbc", 0, Some((1, 6))),
            ("?c", "abcbc", 0, Some((1, 3))),
            ("*", "abc", 0, Some((0, 3))),
            ("*", "abc", 3, Some((3, 3))),
            ("~*", "a*b", 0, Some((1, 2))),
            ("~?~~", "a?~", 0, Some((1, 3))),
            ("a*b*c", "aaaa", 0, None),
            ("x", "abc", 4, None),
            ("a?c", "abxc", 0, None),
            ("a?b", "aab", 0, Some((0, 3))),
            // The leftmost start wins over a shorter match starting later.
            ("*b", "aab", 0, Some((0, 3))),
            ("a*b", "xaxab", 0, Some((1, 5))),
        ] {
            assert_eq!(
                find(pattern, false, false, text, from),
                expected,
                "{pattern} in {text} from {from}"
            );
        }
    }

    #[test]
    fn case_and_whole_texts_are_the_caller_s_choice() {
        assert_eq!(find("APPLE", false, false, "an apple", 0), Some((3, 8)));
        assert_eq!(find("APPLE", true, false, "an apple", 0), None);
        assert_eq!(find("app*", false, true, "Apple pie", 0), Some((0, 9)));
        assert_eq!(find("app", false, true, "Apple pie", 0), None);
        // A whole-text match starts nowhere but the start.
        assert_eq!(find("*", false, true, "abc", 1), None);
    }

    #[test]
    fn a_replace_takes_every_match_left_to_right_none_overlapping() {
        for (pattern, text, replacement, expected) in [
            ("ab", "abab", "x", Some("xx")),
            ("a?", "aaaaa", "b", Some("bba")),
            ("*", "abc", "x", Some("x")),
            ("b*", "abcbd", "", Some("a")),
            ("z", "abc", "x", None),
        ] {
            assert_eq!(
                replace(pattern, false, false, text, replacement).as_deref(),
                expected,
                "{pattern} in {text}"
            );
        }
        assert_eq!(
            replace("north", false, true, "North", "South").as_deref(),
            Some("South")
        );
    }

    #[test]
    fn compiled_whole_wildcards_match_without_per_row_allocations() {
        assert_eq!(
            count_whole_matches("e*", &["east", "East", "west", "EAST"]),
            3
        );
        assert_eq!(count_whole_matches("~*", &["*", "east", "**"]), 1);
        assert_eq!(count_whole_matches("a?c", &["abc", "aéc", "ac"]), 2);
        assert_eq!(count_whole_matches("*", &[""]), 1);
    }
    #[test]
    fn a_long_text_costs_its_length_per_match() {
        // Every start of 32,767 characters fails late: a restart per start
        // would read the square of the text.
        let text = "a".repeat(32_767);
        assert_eq!(find("*a*b", false, false, &text, 0), None);
        assert_eq!(find("a*a?", false, false, &text, 0), Some((0, 32_767)));
        assert_eq!(
            replace("a?", false, false, &text, "b").map(|text| text.len()),
            Some(16_384)
        );
    }
}

#[cfg(feature = "internals")]
#[test]
fn search_and_find_share_transitions_without_sharing_position_units() {
    use yggdryl::internals::excel_formula_criteria::{find, search_utf16};
    let text = "A\u{1f600}Z";
    assert_eq!(find("?Z", false, false, text, 0), Some((1, 3)));
    assert_eq!(search_utf16("?Z", text, 0, false), Some((2, 4)));
    assert_eq!(search_utf16("?Z", text, 0, true), None);
    assert_eq!(search_utf16("??Z", text, 0, true), Some((1, 4)));
    assert_eq!(find("~", false, false, "a~b", 0), Some((1, 2)));
    assert_eq!(search_utf16("~", "a~b", 0, false), Some((0, 0)));
}

#[cfg(feature = "internals")]
#[test]
fn text_criterion_tildes_match_native_literal_and_wildcard_modes() {
    use yggdryl::internals::excel_formula_criteria::count_text_criterion_matches;
    let texts = ["~", "~~", "~~~", "*", "?", "a~", "a~~", "~a"];
    for (pattern, expected) in [
        ("~", 1),
        ("~~", 1),
        ("~~~", 1),
        ("~~~~", 0),
        ("~a", 1),
        ("a~", 1),
        ("a~~", 1),
        ("~*", 1),
        ("~?", 1),
        ("~~*", 4),
        ("*~~", 5),
        ("*~", 8),
        ("?~", 3),
        ("~~?", 2),
    ] {
        assert_eq!(
            count_text_criterion_matches(pattern, &texts),
            expected,
            "{pattern}"
        );
    }
}
