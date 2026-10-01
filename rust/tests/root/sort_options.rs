//! `rust/src/sort_options.rs`: the direction and the nulls placement an
//! ordering states beside its key, and the suffix they are spelled as.

use yggdryl::SortOptions;

#[test]
fn the_default_is_ascending_with_nulls_last_as_arrow_and_the_plan_default() {
    let options = SortOptions::default();
    assert!(!options.is_descending());
    assert!(!options.is_nulls_first());
    assert_eq!(options, SortOptions::ascending());
    assert_eq!(options.to_string(), "");
}

#[test]
fn the_builders_state_each_fact_and_display_writes_the_plan_suffix() {
    assert_eq!(SortOptions::descending().to_string(), " desc");
    assert_eq!(
        SortOptions::ascending().with_nulls_first(true).to_string(),
        " nulls first"
    );
    let both = SortOptions::descending().with_nulls_first(true);
    assert!(both.is_descending() && both.is_nulls_first());
    assert_eq!(both.to_string(), " desc nulls first");
    assert!(!both.with_nulls_first(false).is_nulls_first());
}

#[test]
fn the_suffix_reads_back_folded_in_either_order_and_the_empty_text_is_the_default() {
    for (text, expected) in [
        ("", SortOptions::default()),
        ("   ", SortOptions::default()),
        ("asc", SortOptions::ascending()),
        ("DESC", SortOptions::descending()),
        (
            " desc nulls first",
            SortOptions::descending().with_nulls_first(true),
        ),
        (
            "nulls first desc",
            SortOptions::descending().with_nulls_first(true),
        ),
        ("NULLS LAST", SortOptions::ascending()),
        ("asc nulls last", SortOptions::ascending()),
    ] {
        assert_eq!(
            text.parse::<SortOptions>().expect(text),
            expected,
            "{text:?}"
        );
    }
    for options in [
        SortOptions::default(),
        SortOptions::descending(),
        SortOptions::ascending().with_nulls_first(true),
        SortOptions::descending().with_nulls_first(true),
    ] {
        assert_eq!(
            options
                .to_string()
                .parse::<SortOptions>()
                .expect("its own suffix"),
            options
        );
    }
}

#[test]
fn a_word_that_is_neither_fact_or_a_fact_stated_twice_is_refused_by_name() {
    let refused = "descending".parse::<SortOptions>().unwrap_err().to_string();
    assert!(refused.contains("\"descending\""), "{refused}");
    let refused = "desc asc".parse::<SortOptions>().unwrap_err().to_string();
    assert!(refused.contains("direction is stated twice"), "{refused}");
    let refused = "nulls first nulls last"
        .parse::<SortOptions>()
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("nulls placement is stated twice"),
        "{refused}"
    );
    let refused = "nulls".parse::<SortOptions>().unwrap_err().to_string();
    assert!(
        refused.contains("`nulls first` or `nulls last`"),
        "{refused}"
    );
    let refused = "nulls middle"
        .parse::<SortOptions>()
        .unwrap_err()
        .to_string();
    assert!(refused.contains("`nulls middle`"), "{refused}");
}

#[test]
fn the_options_are_copy_ordered_and_hashable() {
    let options = SortOptions::descending();
    let copied = options;
    assert_eq!(options, copied);
    assert!(SortOptions::ascending() < SortOptions::descending());
    let set: std::collections::HashSet<SortOptions> = [options, copied, SortOptions::ascending()]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 2);
}
