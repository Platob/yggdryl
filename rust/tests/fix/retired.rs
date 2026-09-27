//! `rust/src/fix/retired.rs`: the crate's own table of what the specification
//! retired, and of how its instrument fields are restated.
//!
//! Every parse applies it, and what a caller sees is the restated message; the
//! table itself is reached through `yggdryl::internals`. What it *means* is
//! pinned by the restated messages of `rust/tests/fix/latest.rs` and
//! `rust/tests/fix/crated.rs`; this is the table's own shape.

use yggdryl::internals::fix_retired::{RULES, When, rules_of};

#[test]
fn the_table_is_sorted_by_tag_with_each_tag_once() {
    for pair in RULES.windows(2) {
        assert!(pair[0].0 < pair[1].0, "{} before {}", pair[0].0, pair[1].0);
    }
    // The specification's thirty-seven retired tags and the crate's two
    // instrument fields.
    assert_eq!(RULES.len(), 39);
    assert_eq!(
        RULES.iter().map(|(_, rules)| rules.len()).sum::<usize>(),
        102
    );
}

#[test]
fn a_lookup_answers_the_tag_it_is_asked_for() {
    assert!(rules_of(47).is_some_and(|rules| rules.len() == 23));
    assert!(rules_of(18).is_some_and(|rules| rules.len() == 9));
    assert!(rules_of(687).is_some_and(|rules| rules.len() == 2));
    assert!(rules_of(1).is_none());
    assert!(rules_of(9_999).is_none());
    // A bridge's instrument folds under its own source where the source can
    // hold it.
    for (tag, source) in [
        (yggdryl::OMSINSTRUMENTID_TAG_NAME.0, "OMSINSTRUMENTID"),
        (yggdryl::ULLINKINSTRUMENTID_TAG_NAME.0, "ULLINKINSTRUMENTID"),
    ] {
        let rules = rules_of(tag).expect("an instrument field's rule");
        assert_eq!(rules.len(), 1);
        assert!(matches!(rules[0].when, When::FitsSource(held) if held == source));
    }
}

#[test]
fn every_rule_fills_something_and_a_catch_all_comes_last() {
    for (tag, rules) in RULES {
        for (at, rule) in rules.iter().enumerate() {
            assert!(!rule.fills.is_empty(), "tag {tag} entry {at} fills nothing");
            if matches!(rule.when, When::Any) && rule.msgtypes.is_empty() && rule.within.is_none() {
                assert_eq!(
                    at + 1,
                    rules.len(),
                    "tag {tag}: a catch-all is the last entry"
                );
            }
        }
    }
}
