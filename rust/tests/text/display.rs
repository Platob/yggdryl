//! `rust/src/text/display.rs`: the byte budget every error message crosses.
//!
//! Bounded interpolation is what keeps a refusal from copying a caller's
//! payload into itself. A caller reads the sentence and never the budget, so
//! the eliding is pinned through `yggdryl::internals`; the sentences it builds
//! are pinned wherever the refusals that carry them are.

use yggdryl::internals::text_display::{
    ERROR_TEXT_LIMIT, elide_display_to, elide_to, expected_got,
};

#[test]
fn short_text_is_unchanged() {
    assert_eq!(elide_to("id", ERROR_TEXT_LIMIT).to_string(), "id");
    assert_eq!(format!("{:?}", elide_to("id", ERROR_TEXT_LIMIT)), "\"id\"");
}

#[test]
fn debug_keeps_empty_and_whitespace_visible() {
    let quoted = |value: &str| format!("{:?}", elide_to(value, ERROR_TEXT_LIMIT));
    assert_eq!(quoted(""), "\"\"");
    assert_eq!(quoted("a "), "\"a \"");
    assert_eq!(quoted("a\tb"), "\"a\\tb\"");
}

#[test]
fn long_text_is_bounded_and_marked() {
    let long = "x".repeat(ERROR_TEXT_LIMIT * 4);
    let rendered = elide_to(&long, ERROR_TEXT_LIMIT).to_string();
    assert!(rendered.len() <= ERROR_TEXT_LIMIT + 4, "{}", rendered.len());
    assert!(rendered.ends_with('\u{2026}'), "{rendered}");
}

#[test]
fn truncation_never_splits_a_character() {
    // Each `é` is two bytes, so a 5-byte budget must stop at 4.
    let text = "ééé";
    let rendered = elide_to(text, 5).to_string();
    assert_eq!(rendered, "éé\u{2026}");
}

#[test]
fn debug_truncation_stays_a_balanced_literal() {
    let rendered = format!("{:?}", elide_to("abcdefgh", 3));
    assert_eq!(rendered, "\"abc\u{2026}\"");
}

#[test]
fn display_values_are_bounded_without_rendering_the_whole_value() {
    struct Wide(usize);
    impl std::fmt::Display for Wide {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            for index in 0..self.0 {
                write!(formatter, "field_{index},")?;
            }
            Ok(())
        }
    }

    let rendered = elide_display_to(&Wide(10_000), 32).to_string();
    assert!(rendered.len() <= 36, "{}", rendered.len());
    assert!(rendered.ends_with('\u{2026}'), "{rendered}");
    assert!(rendered.starts_with("field_0,"), "{rendered}");
}

#[test]
fn expected_got_uses_the_contract_sentence() {
    assert_eq!(expected_got("int64", "utf8"), "expected int64, got utf8");
}
