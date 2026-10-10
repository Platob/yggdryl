//! `rust/fix/src/retired.rs`: the crate's own table of what the specification
//! retired.
//!
//! Every parse applies it, and what a caller sees is the restated message; the
//! table itself is reached through `yggdryl_fix::internals`. What it *means* is
//! pinned by the restated messages of `rust/fix/tests/root/latest.rs`; this is the
//! table's own shape.

use yggdryl_fix::internals::retired::{RULES, When, rules_of};

#[test]
fn the_table_is_sorted_by_tag_with_each_tag_once() {
    crate::install::installed();
    for pair in RULES.windows(2) {
        assert!(pair[0].0 < pair[1].0, "{} before {}", pair[0].0, pair[1].0);
    }
    // The specification's thirty-seven retired tags, and nothing of the
    // crate's: a bridge's own instrument keys are read off their names, not
    // restated by a rule.
    assert_eq!(RULES.len(), 37);
    assert_eq!(
        RULES.iter().map(|(_, rules)| rules.len()).sum::<usize>(),
        100
    );
    assert!(
        RULES
            .iter()
            .all(|(tag, _)| !yggdryl_fix::is_crate_tag(*tag))
    );
}

#[test]
fn a_lookup_answers_the_tag_it_is_asked_for() {
    crate::install::installed();
    assert!(rules_of(47).is_some_and(|rules| rules.len() == 23));
    assert!(rules_of(18).is_some_and(|rules| rules.len() == 9));
    assert!(rules_of(687).is_some_and(|rules| rules.len() == 2));
    assert!(rules_of(1).is_none());
    assert!(rules_of(9_999).is_none());
    // The crate's own tags restate nothing: the retired bridge instrument
    // fields leave no rule behind.
    for tag in 65_000..65_100 {
        assert!(rules_of(tag).is_none(), "{tag}");
    }
}

#[test]
fn every_rule_fills_something_and_a_catch_all_comes_last() {
    crate::install::installed();
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
