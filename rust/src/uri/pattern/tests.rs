//! The glob matcher an integration test cannot reach.
//!
//! A URL cannot carry a bracket, so the per-segment matcher behind
//! `matches_glob` is exercised directly here. Everything a caller can observe
//! lives in `tests/uri/pattern.rs`.

use super::matches_segment;

#[test]
fn an_unterminated_class_is_matched_literally() {
    // A URL cannot carry a bracket, so the matcher is exercised directly.

    assert!(matches_segment("part-[0.arrows", "part-[0.arrows"));
    assert!(!matches_segment("part-0.arrows", "part-[0.arrows"));
    assert!(matches_segment("[a-b", "[a-b"));
    assert!(matches_segment("[!x", "[!x"));
}
